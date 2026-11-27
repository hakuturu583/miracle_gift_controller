//! miragift CLI
//!
//!   miragift gen <code|name> [-o out.wav] [--sr 48000] [--amp 0.5] [--repeat 2] [--gap 0.3] [--lead 0.15] [--ramp-ms 20] [--keyed]
//!   miragift list
//!   miragift decode <in.wav> [--channel N] [--segments]
//!   miragift scan <in.wav> [--channel N] [--min-hz 15000] [--max-hz 22000] [--thr-db 20] [--from S] [--to S]
//!
//! `scan` is protocol-agnostic: it finds every narrow-band near-ultrasonic tone in a recording and
//! prints its frequency/duration, plus histograms. Use it on a capture of the Blu-ray audio to
//! verify (or discover) tone frequencies and timings. `decode` applies the known protocol.

use miragift_core::decoder::Event;
use miragift_core::{synth, Command, Decoder, DecoderConfig, SynthConfig};
use rustfft::{num_complex::Complex, FftPlanner};
use std::collections::BTreeMap;
use std::process::exit;

fn usage() -> ! {
    eprintln!(
        "usage:\n  miragift gen <code|name> [-o out.wav] [--sr 48000] [--amp 0.5] [--repeat 2] [--gap 0.3] [--lead 0.15] [--ramp-ms 20] [--keyed]\n  miragift list\n  miragift decode <in.wav> [--channel N] [--segments]\n  miragift scan <in.wav> [--channel N] [--min-hz 15000] [--max-hz 22000] [--thr-db 20] [--from S] [--to S]"
    );
    exit(2)
}

struct Args {
    pos: Vec<String>,
    opts: BTreeMap<String, String>,
    flags: Vec<String>,
}

fn parse_args() -> Args {
    let mut a = Args { pos: vec![], opts: BTreeMap::new(), flags: vec![] };
    let mut it = std::env::args().skip(1).peekable();
    while let Some(s) = it.next() {
        if let Some(f) = s.strip_prefix("--").filter(|f| ["segments", "keyed"].contains(f)) {
            a.flags.push(f.to_string());
        } else if let Some(k) = s.strip_prefix("--").or_else(|| s.strip_prefix('-').filter(|k| k.len() == 1)) {
            let v = it.next().unwrap_or_else(|| usage());
            a.opts.insert(k.to_string(), v);
        } else {
            a.pos.push(s);
        }
    }
    a
}

impl Args {
    fn get<T: std::str::FromStr>(&self, k: &str, d: T) -> T {
        self.opts.get(k).map(|v| v.parse().unwrap_or_else(|_| usage())).unwrap_or(d)
    }
    fn flag(&self, f: &str) -> bool {
        self.flags.iter().any(|x| x == f)
    }
}

fn resolve(s: &str) -> Command {
    if let Ok(c) = s.parse::<Command>() {
        return c;
    }
    if let Some(n) = miragift_core::named_commands().iter().find(|n| n.name.eq_ignore_ascii_case(s)) {
        return n.command();
    }
    eprintln!("unknown command '{s}' (use a 7-digit code or a name from `miragift list`)");
    exit(1)
}

/// Streams a WAV file as mono f32 chunks (one channel or the mix).
fn read_wav(path: &str, channel: Option<usize>, mut f: impl FnMut(&[f32])) -> u32 {
    let mut r = hound::WavReader::open(path).unwrap_or_else(|e| {
        eprintln!("{path}: {e}");
        exit(1)
    });
    let spec = r.spec();
    let ch = spec.channels as usize;
    if let Some(c) = channel {
        if c >= ch {
            eprintln!("channel {c} out of range (file has {ch})");
            exit(1)
        }
    }
    let scale = match spec.sample_format {
        hound::SampleFormat::Float => 1.0,
        hound::SampleFormat::Int => 1.0 / (1u64 << (spec.bits_per_sample - 1)) as f32,
    };
    let mut frame = Vec::with_capacity(ch);
    let mut chunk = Vec::with_capacity(48_000);
    let mut push = |v: f32, frame: &mut Vec<f32>, chunk: &mut Vec<f32>| {
        frame.push(v * scale);
        if frame.len() == ch {
            let m = match channel {
                Some(c) => frame[c],
                None => frame.iter().sum::<f32>() / ch as f32,
            };
            chunk.push(m);
            frame.clear();
            if chunk.len() >= 48_000 {
                f(chunk);
                chunk.clear();
            }
        }
    };
    match spec.sample_format {
        hound::SampleFormat::Float => {
            for s in r.samples::<f32>() {
                push(s.expect("read"), &mut frame, &mut chunk);
            }
        }
        hound::SampleFormat::Int => {
            for s in r.samples::<i32>() {
                push(s.expect("read") as f32, &mut frame, &mut chunk);
            }
        }
    }
    if !chunk.is_empty() {
        f(&chunk);
    }
    spec.sample_rate
}

fn wav_rate(path: &str) -> u32 {
    hound::WavReader::open(path)
        .unwrap_or_else(|e| {
            eprintln!("{path}: {e}");
            exit(1)
        })
        .spec()
        .sample_rate
}

fn describe(c: Command) -> String {
    let names: Vec<&str> = c.named().map(|n| n.name).collect();
    let mut s = format!("{c} (id {:3})", c.reference_id());
    if !names.is_empty() {
        s += &format!(" [{}]", names.join(", "));
    } else if let Some(k) = c.known() {
        let cols: Vec<&str> = k.colors.iter().map(|c| c.name()).collect();
        if !cols.is_empty() {
            s += &format!(" [{} @{}]", cols.join("/"), k.bpm);
        }
    }
    s
}

fn main() {
    let a = parse_args();
    match a.pos.first().map(String::as_str) {
        Some("gen") => {
            let cmd = resolve(a.pos.get(1).unwrap_or_else(|| usage()));
            let d = SynthConfig::default();
            let cfg = SynthConfig {
                sample_rate: a.get("sr", d.sample_rate),
                amplitude: a.get("amp", d.amplitude),
                repeat: a.get("repeat", d.repeat),
                gap_sec: a.get("gap", d.gap_sec),
                lead_in_sec: a.get("lead", d.lead_in_sec),
                ramp_sec: a.get::<f64>("ramp-ms", d.ramp_sec * 1e3) / 1e3,
                continuous: !a.flag("keyed"),
                ..d
            };
            let out = a.opts.get("o").cloned().unwrap_or_else(|| format!("{cmd}.wav"));
            let pcm = synth::synthesize(cmd, &cfg);
            std::fs::write(&out, synth::to_wav_i16(&pcm, cfg.sample_rate)).expect("write");
            println!("{out}: {} @ {} Hz, {:.2} s", describe(cmd), cfg.sample_rate, pcm.len() as f64 / cfg.sample_rate as f64);
        }
        Some("list") => {
            for n in miragift_core::named_commands() {
                println!("{:8} {:20} {}", n.group, n.name, describe(n.command()));
            }
        }
        Some("decode") => {
            let path = a.pos.get(1).unwrap_or_else(|| usage());
            let channel = a.opts.get("channel").map(|c| c.parse().unwrap_or_else(|_| usage()));
            let sr = wav_rate(path);
            let mut dec = Decoder::new(DecoderConfig { sample_rate: sr, ..Default::default() });
            let show_seg = a.flag("segments");
            let mut n = 0;
            let mut print = |e: Event| match e {
                Event::Segment(s) if show_seg => println!(
                    "  seg {:9.3}s {:6.1}ms F{} {:7.0}Hz {:6.1}dBFS",
                    s.start,
                    (s.end - s.start) * 1e3,
                    s.tone,
                    miragift_core::TONES_HZ[s.tone],
                    s.level_db
                ),
                Event::Packet(p) => {
                    n += 1;
                    println!(
                        "{:9.3}s {} {} margin {:5.1}dB level {:6.1}dBFS slots {:?}",
                        p.start,
                        if p.valid { "OK " } else { "BAD" },
                        describe(p.command),
                        p.margin_db,
                        p.level_db,
                        p.slot_margin_db.map(|m| m.round() as i32)
                    )
                }
                _ => {}
            };
            read_wav(path, channel, |c| dec.push(c).into_iter().for_each(&mut print));
            dec.finish().into_iter().for_each(&mut print);
            eprintln!("{n} packet(s)");
        }
        Some("scan") => scan(&a),
        _ => usage(),
    }
}

/// Protocol-agnostic narrow-band tone finder.
fn scan(a: &Args) {
    let path = a.pos.get(1).unwrap_or_else(|| usage());
    let channel = a.opts.get("channel").map(|c| c.parse().unwrap_or_else(|_| usage()));
    let sr = wav_rate(path) as f64;
    let (min_hz, max_hz): (f64, f64) = (a.get("min-hz", 15_000.0), a.get("max-hz", (sr / 2.0 - 200.0).min(22_000.0)));
    let thr_db: f64 = a.get("thr-db", 20.0);
    let (from, to): (f64, f64) = (a.get("from", 0.0), a.get("to", f64::INFINITY));
    // ~21 ms window: 47 Hz bins (tones are 250 Hz apart) while keeping ~10 ms timing resolution.
    let nfft = ((sr * 0.021) as usize).next_power_of_two();
    let hop = (sr * 0.005).round() as usize;
    let win: Vec<f32> = (0..nfft).map(|n| (0.5 - 0.5 * (std::f64::consts::TAU * n as f64 / nfft as f64).cos()) as f32).collect();
    let fft = FftPlanner::<f32>::new().plan_fft_forward(nfft);
    let bin_hz = sr / nfft as f64;
    let (b0, b1) = ((min_hz / bin_hz).ceil() as usize, (max_hz / bin_hz).floor() as usize);
    let thr = 10f32.powf(thr_db as f32 / 10.0);
    // full-scale sine through the Hann window -> |X|^2 = (N/4)^2
    let ref_p = (nfft as f64 / 4.0).powi(2);

    let mut buf: Vec<f32> = Vec::new();
    let mut consumed = 0usize;
    let mut cur: Option<(f64, f64, f64, f64, usize)> = None; // start, end, freq sum, power sum, n
    let mut segs: Vec<(f64, f64, f64, f64)> = vec![];
    let mut spec = vec![Complex::new(0.0f32, 0.0); nfft];
    let mut mags = vec![0.0f32; b1 + 2];
    let close = |cur: &mut Option<(f64, f64, f64, f64, usize)>, segs: &mut Vec<(f64, f64, f64, f64)>| {
        if let Some((s, e, fs, ps, n)) = cur.take() {
            if n >= 3 {
                segs.push((s, e + hop as f64 / sr, fs / n as f64, 10.0 * (ps / n as f64 / ref_p).log10()));
            }
        }
    };
    read_wav(path, channel, |chunk| {
        buf.extend_from_slice(chunk);
        let mut off = 0;
        while buf.len() - off >= nfft {
            let t = (consumed + off) as f64 / sr + nfft as f64 / sr / 2.0;
            off += hop;
            if t < from || t > to {
                continue;
            }
            for i in 0..nfft {
                spec[i] = Complex::new(buf[off - hop + i] * win[i], 0.0);
            }
            fft.process(&mut spec);
            for b in b0.saturating_sub(1)..=b1 + 1 {
                mags[b.min(b1 + 1)] = spec[b].norm_sqr();
            }
            let mut band: Vec<f32> = mags[b0..=b1].to_vec();
            let (pk, &pv) = band.iter().enumerate().max_by(|x, y| x.1.total_cmp(y.1)).unwrap();
            band.sort_by(f32::total_cmp);
            let median = band[band.len() / 2].max(1e-12);
            let pkb = pk + b0;
            if pv > median * thr {
                // parabolic interpolation on log magnitude
                let (l, c, r) = (mags[pkb - 1].max(1e-20).ln(), pv.ln(), mags[pkb + 1].max(1e-20).ln());
                let d = 0.5 * (l - r) / (l - 2.0 * c + r);
                let f = (pkb as f64 + d as f64) * bin_hz;
                match &mut cur {
                    Some(c) if (f - c.2 / c.4 as f64).abs() < 60.0 => {
                        c.1 = t;
                        c.2 += f;
                        c.3 += pv as f64;
                        c.4 += 1;
                    }
                    _ => {
                        close(&mut cur, &mut segs);
                        cur = Some((t, t, f, pv as f64, 1));
                    }
                }
            } else {
                close(&mut cur, &mut segs);
            }
        }
        buf.drain(..off);
        consumed += off;
    });
    close(&mut cur, &mut segs);

    let mut prev_end = f64::NAN;
    for &(s, e, f, l) in &segs {
        println!("{:10.3}s  dur {:6.1}ms  gap {:7.1}ms  {:8.1}Hz  {:6.1}dBFS", s, (e - s) * 1e3, (s - prev_end) * 1e3, f, l);
        prev_end = e;
    }
    let mut fh: BTreeMap<i64, (usize, f64)> = BTreeMap::new();
    let mut dh: BTreeMap<i64, usize> = BTreeMap::new();
    for &(s, e, f, _) in &segs {
        let k = (f / 25.0).round() as i64 * 25;
        let x = fh.entry(k).or_default();
        x.0 += 1;
        x.1 += e - s;
        *dh.entry(((e - s) * 1e3 / 10.0).round() as i64 * 10).or_default() += 1;
    }
    println!("\n== frequency histogram (25 Hz bins): count, total seconds");
    for (k, (n, d)) in fh {
        println!("{k:7} Hz  {n:6}  {d:8.2}s");
    }
    println!("\n== duration histogram (10 ms bins)");
    for (k, n) in dh {
        println!("{k:5} ms  {n:6}");
    }
    eprintln!("{} tone segment(s)", segs.len());
}
