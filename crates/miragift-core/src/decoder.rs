//! Streaming decoder: feeds arbitrary-sized chunks of mono PCM, emits tone segments and packets.
//!
//! Used for (a) verifying the synthesizer end-to-end through a real speaker/mic, and
//! (b) analysing recordings (e.g. the Blu-ray audio track) to confirm/extend the protocol.

use crate::{Command, FIRST_SLOT_SEC, PREAMBLE_SEC, SLOT_SEC, SYMBOLS, SYMBOL_SEC, TONES_HZ};
use std::collections::VecDeque;

/// Goertzel power of `x` at normalised frequency `f` (cycles/sample).
pub fn goertzel_power(x: &[f32], f: f64) -> f64 {
    let coeff = 2.0 * (std::f64::consts::TAU * f).cos();
    let (mut s1, mut s2) = (0.0f64, 0.0f64);
    for &v in x {
        let s0 = v as f64 + coeff * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    (s1 * s1 + s2 * s2 - coeff * s1 * s2).max(0.0)
}

#[derive(Clone, Debug)]
pub struct DecoderConfig {
    pub sample_rate: u32,
    /// Hop between analysis frames (s).
    pub hop_sec: f64,
    /// Required tone-to-noise ratio (dB) for a frame to count as a tone.
    pub snr_db: f64,
    /// Minimum ratio (dB) between the chosen and the rejected tone of each slot.
    pub min_bit_margin_db: f64,
}

impl Default for DecoderConfig {
    fn default() -> Self {
        DecoderConfig { sample_rate: 48_000, hop_sec: 0.005, snr_db: 12.0, min_bit_margin_db: 6.0 }
    }
}

#[derive(Clone, Debug)]
pub struct DecodedPacket {
    /// Estimated preamble start (s from stream start).
    pub start: f64,
    pub command: Command,
    /// Smallest per-slot margin between the chosen and the rejected tone (dB).
    pub margin_db: f64,
    /// Mean in-band tone level (dB relative to a full-scale sine).
    pub level_db: f64,
    /// All slots passed the margin test.
    pub valid: bool,
    /// Per-slot margins (dB).
    pub slot_margin_db: [f64; SYMBOLS],
}

/// A run of consecutive frames dominated by one tone.
#[derive(Clone, Debug)]
pub struct Segment {
    pub start: f64,
    pub end: f64,
    pub tone: usize,
    pub level_db: f64,
}

#[derive(Clone, Debug)]
pub enum Event {
    Segment(Segment),
    Packet(DecodedPacket),
}

#[derive(Clone, Copy)]
struct Frame {
    t: f64,
    p: [f64; 5],
}

pub struct Decoder {
    cfg: DecoderConfig,
    hop: usize,
    win: usize,
    window: Vec<f32>,
    /// Power of a full-scale sine through the window (for dBFS).
    ref_power: f64,
    buf: Vec<f32>,
    consumed: u64,
    noise: f64,
    history: VecDeque<Frame>,
    run: Option<(usize, f64, f64, f64, usize)>, // tone, start, end, power sum, frames
    pending: VecDeque<f64>,
    last_packet_start: f64,
}

impl Decoder {
    pub fn new(cfg: DecoderConfig) -> Self {
        let sr = cfg.sample_rate as f64;
        let hop = ((cfg.hop_sec * sr).round() as usize).max(1);
        // 20 ms window: 50 Hz bins, plenty to separate tones 250 Hz apart.
        let win = ((0.020 * sr).round() as usize).max(hop);
        let window: Vec<f32> = (0..win)
            .map(|n| (0.5 - 0.5 * (std::f64::consts::TAU * (n as f64 + 0.5) / win as f64).cos()) as f32)
            .collect();
        let probe: Vec<f32> = window
            .iter()
            .enumerate()
            .map(|(n, w)| w * (std::f64::consts::TAU * TONES_HZ[2] / sr * n as f64).sin() as f32)
            .collect();
        let ref_power = goertzel_power(&probe, TONES_HZ[2] / sr);
        Decoder {
            cfg,
            hop,
            win,
            window,
            ref_power,
            buf: Vec::new(),
            consumed: 0,
            noise: ref_power * 1e-9,
            history: VecDeque::new(),
            run: None,
            pending: VecDeque::new(),
            last_packet_start: f64::NEG_INFINITY,
        }
    }

    pub fn config(&self) -> &DecoderConfig {
        &self.cfg
    }

    fn db(&self, p: f64) -> f64 {
        10.0 * (p.max(1e-30) / self.ref_power).log10()
    }

    /// Feed samples; returns events that completed.
    pub fn push(&mut self, samples: &[f32]) -> Vec<Event> {
        let mut events = Vec::new();
        self.buf.extend_from_slice(samples);
        let sr = self.cfg.sample_rate as f64;
        let mut off = 0;
        let mut tmp = vec![0.0f32; self.win];
        while self.buf.len() - off >= self.win {
            for (i, d) in tmp.iter_mut().enumerate() {
                *d = self.buf[off + i] * self.window[i];
            }
            let t = (self.consumed as f64 + off as f64 + self.win as f64 / 2.0) / sr;
            let p: [f64; 5] = std::array::from_fn(|k| goertzel_power(&tmp, TONES_HZ[k] / sr));
            self.on_frame(Frame { t, p }, &mut events);
            off += self.hop;
        }
        self.buf.drain(..off);
        self.consumed += off as u64;
        events
    }

    /// Flush any pending state at end of stream.
    pub fn finish(&mut self) -> Vec<Event> {
        let mut events = Vec::new();
        let pad = vec![0.0f32; (self.cfg.sample_rate as f64 * (crate::PACKET_SEC + 0.1)) as usize];
        events.extend(self.push(&pad));
        if let Some(seg) = self.close_run() {
            events.push(Event::Segment(seg));
        }
        events
    }

    fn close_run(&mut self) -> Option<Segment> {
        let (tone, start, end, psum, n) = self.run.take()?;
        let h = self.cfg.hop_sec / 2.0;
        Some(Segment { start: start - h, end: end + h, tone, level_db: self.db(psum / n as f64) })
    }

    fn on_frame(&mut self, f: Frame, events: &mut Vec<Event>) {
        let (mut imax, mut pmax, mut pmin) = (0, 0.0f64, f64::INFINITY);
        for (k, &p) in f.p.iter().enumerate() {
            if p > pmax {
                pmax = p;
                imax = k;
            }
            pmin = pmin.min(p);
        }
        // Noise floor: fast attack downward, slow release upward, from the quietest tone bin.
        let floor = self.ref_power * 1e-12;
        if pmin < self.noise {
            self.noise = pmin.max(floor);
        } else {
            self.noise += (pmin - self.noise) * 0.002;
        }
        let snr = 10f64.powf(self.cfg.snr_db / 10.0);
        let tone = if pmax > self.noise * snr && pmax > self.ref_power * 1e-8 { Some(imax) } else { None };

        // Segment tracking.
        match (&mut self.run, tone) {
            (Some(r), Some(k)) if r.0 == k => {
                r.2 = f.t;
                r.3 += pmax;
                r.4 += 1;
            }
            _ => {
                if let Some(seg) = self.close_run() {
                    self.on_segment(&seg);
                    events.push(Event::Segment(seg));
                }
                if let Some(k) = tone {
                    self.run = Some((k, f.t, f.t, pmax, 1));
                }
            }
        }

        self.history.push_back(f);
        while self.history.front().is_some_and(|h| h.t < f.t - 2.0) {
            self.history.pop_front();
        }

        // Decode packets whose data slots are fully in the history.
        while let Some(&t0) = self.pending.front() {
            if f.t < t0 + crate::PACKET_SEC + 0.03 {
                break;
            }
            self.pending.pop_front();
            if let Some(p) = self.decode_at(t0) {
                if p.valid {
                    self.last_packet_start = t0;
                }
                events.push(Event::Packet(p));
            }
        }
    }

    fn on_segment(&mut self, seg: &Segment) {
        if seg.tone != 0 {
            return;
        }
        let d = seg.end - seg.start;
        if !(0.10..=0.30).contains(&d) {
            return;
        }
        // Combine onset and offset estimates of the 190 ms preamble.
        let t0 = if d > PREAMBLE_SEC + 0.03 { seg.end - PREAMBLE_SEC } else { 0.5 * (seg.start + seg.end - PREAMBLE_SEC) };
        if t0 - self.last_packet_start < crate::PACKET_SEC {
            return;
        }
        self.pending.push_back(t0);
    }

    fn decode_at(&self, t0: f64) -> Option<DecodedPacket> {
        let mut bits = 0u8;
        let mut margins = [0.0f64; SYMBOLS];
        let mut level = 0.0;
        // Slot centre +- a guard that keeps the 20 ms window inside the 90 ms tone.
        let half = SYMBOL_SEC / 2.0 - 0.015;
        for (i, m) in margins.iter_mut().enumerate() {
            let c = t0 + FIRST_SLOT_SEC + SLOT_SEC * i as f64 + SYMBOL_SEC / 2.0;
            let (lo_k, hi_k) = if i % 2 == 0 { (1, 3) } else { (2, 4) };
            let (mut lo, mut hi, mut n) = (0.0, 0.0, 0);
            for h in self.history.iter().filter(|h| (h.t - c).abs() <= half) {
                lo += h.p[lo_k];
                hi += h.p[hi_k];
                n += 1;
            }
            if n == 0 {
                return None;
            }
            if hi > lo {
                bits |= 1 << i;
            }
            *m = 10.0 * (lo.max(hi) / lo.min(hi).max(1e-30)).log10();
            level += lo.max(hi) / n as f64;
        }
        let margin_db = margins.iter().cloned().fold(f64::INFINITY, f64::min);
        Some(DecodedPacket {
            start: t0,
            command: Command::from_bits(bits),
            margin_db,
            level_db: self.db(level / SYMBOLS as f64),
            valid: margin_db >= self.cfg.min_bit_margin_db,
            slot_margin_db: margins,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth::{synthesize, SynthConfig};

    fn decode_all(pcm: &[f32], sr: u32, chunk: usize) -> Vec<DecodedPacket> {
        let mut d = Decoder::new(DecoderConfig { sample_rate: sr, ..Default::default() });
        let mut ev = Vec::new();
        for c in pcm.chunks(chunk) {
            ev.extend(d.push(c));
        }
        ev.extend(d.finish());
        ev.into_iter()
            .filter_map(|e| if let Event::Packet(p) = e { Some(p) } else { None })
            .collect()
    }

    #[test]
    fn roundtrip_every_command() {
        for sr in [44_100u32, 48_000] {
            for b in 0..128u8 {
                let cmd = Command::from_bits(b);
                let cfg = SynthConfig { sample_rate: sr, repeat: 2, ..Default::default() };
                let pcm = synthesize(cmd, &cfg);
                let pk = decode_all(&pcm, sr, 1000);
                assert_eq!(pk.len(), 2, "sr={sr} cmd={cmd}");
                for p in pk {
                    assert!(p.valid, "{p:?}");
                    assert_eq!(p.command, cmd);
                }
            }
        }
    }

    #[test]
    fn survives_noise_and_attenuation() {
        let cmd: Command = "1234123".parse().unwrap();
        let cfg = SynthConfig { amplitude: 0.01, repeat: 1, ..Default::default() };
        let mut pcm = synthesize(cmd, &cfg);
        // add broadband noise ~ -46 dBFS rms plus a loud audible hum
        let mut s: u32 = 1;
        for (n, v) in pcm.iter_mut().enumerate() {
            s = s.wrapping_mul(1_103_515_245).wrapping_add(12345);
            let r = ((s >> 9) as f32 / (1u32 << 23) as f32) - 0.5;
            *v += 0.017 * r + 0.3 * (n as f32 * std::f32::consts::TAU * 440.0 / 48_000.0).sin();
        }
        let pk = decode_all(&pcm, 48_000, 480);
        assert_eq!(pk.len(), 1);
        assert_eq!(pk[0].command, cmd);
        assert!(pk[0].valid);
    }
}
