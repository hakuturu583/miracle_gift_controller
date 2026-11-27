//! Offline (sample-exact) packet synthesizer.
//!
//! The whole control signal is rendered into a PCM buffer up front, at the exact sample rate of
//! the output device, and then played back in one go. Compared to driving a live oscillator from
//! JavaScript timers (Tone.js `Synth` + `Transport` in the reference controller) this removes:
//!
//! * timing jitter from main-thread scheduling,
//! * envelope artefacts (Tone.Synth's default ADSR has a 1 s release and a 0.3 sustain, so tones
//!   bleed into the following slot and the level drops ~10 dB after 100 ms),
//! * hard on/off keying clicks: every tone gets a raised-cosine fade so energy stays inside the
//!   18.5–19.5 kHz band instead of splattering into the audible range,
//! * clipping / intermodulation from driving the DAC at 0 dBFS,
//! * browser resampling of a fixed-rate buffer near Nyquist (we render at `ctx.sampleRate`).

use crate::{Command, FIRST_SLOT_SEC, PREAMBLE_SEC, SLOT_SEC, SYMBOLS, SYMBOL_SEC, TONES_HZ};

#[derive(Clone, Debug)]
pub struct SynthConfig {
    /// Output sample rate in Hz. Must match the playback device (AudioContext.sampleRate).
    pub sample_rate: u32,
    /// Peak amplitude, 0..1. Default 0.5 (-6 dBFS) leaves headroom for DAC / amp non-linearity.
    pub amplitude: f32,
    /// Raised-cosine fade in/out per tone (s).
    pub ramp_sec: f64,
    /// Number of times the packet is sent.
    pub repeat: u32,
    /// Silence between repeated packets (s).
    pub gap_sec: f64,
    /// Leading silence (s). Lets output paths that power up / fade in on start (Bluetooth,
    /// Android audio HAL, USB DACs) settle before the preamble.
    pub lead_in_sec: f64,
    /// Trailing silence (s), so the last tone's fade is never cut by the device stopping.
    pub tail_sec: f64,
}

impl Default for SynthConfig {
    fn default() -> Self {
        SynthConfig {
            sample_rate: 48_000,
            amplitude: 0.5,
            ramp_sec: 0.006,
            repeat: 2,
            gap_sec: 0.30,
            lead_in_sec: 0.15,
            tail_sec: 0.10,
        }
    }
}

/// One keyed tone: (start s, duration s, tone index).
pub fn packet_schedule(cmd: Command) -> Vec<(f64, f64, usize)> {
    let mut v = Vec::with_capacity(SYMBOLS + 1);
    v.push((0.0, PREAMBLE_SEC, 0));
    for (i, t) in cmd.tones().into_iter().enumerate() {
        v.push((FIRST_SLOT_SEC + SLOT_SEC * i as f64, SYMBOL_SEC, t));
    }
    v
}

/// Render `cmd` as mono f32 PCM.
pub fn synthesize(cmd: Command, cfg: &SynthConfig) -> Vec<f32> {
    let sr = cfg.sample_rate as f64;
    let repeat = cfg.repeat.max(1);
    let packet_len = crate::PACKET_SEC;
    let total = cfg.lead_in_sec
        + packet_len * repeat as f64
        + cfg.gap_sec * (repeat - 1) as f64
        + cfg.tail_sec;
    let mut out = vec![0.0f32; (total * sr).ceil() as usize + 1];
    let sched = packet_schedule(cmd);
    for r in 0..repeat {
        let base = cfg.lead_in_sec + (packet_len + cfg.gap_sec) * r as f64;
        for &(start, dur, tone) in &sched {
            render_tone(&mut out, sr, base + start, dur, TONES_HZ[tone], cfg);
        }
    }
    out
}

fn render_tone(out: &mut [f32], sr: f64, start: f64, dur: f64, freq: f64, cfg: &SynthConfig) {
    let n0 = (start * sr).round() as usize;
    let len = (dur * sr).round() as usize;
    let ramp = ((cfg.ramp_sec * sr).round() as usize).clamp(1, len / 2);
    let w = std::f64::consts::TAU * freq / sr;
    let amp = cfg.amplitude.clamp(0.0, 1.0) as f64;
    for n in 0..len {
        let env = if n < ramp {
            0.5 - 0.5 * (std::f64::consts::PI * (n as f64 + 0.5) / ramp as f64).cos()
        } else if n >= len - ramp {
            let k = len - n; // 1..=ramp
            0.5 - 0.5 * (std::f64::consts::PI * (k as f64 - 0.5) / ramp as f64).cos()
        } else {
            1.0
        };
        if let Some(s) = out.get_mut(n0 + n) {
            *s += (amp * env * (w * n as f64).sin()) as f32;
        }
    }
}

/// Encode mono f32 samples as a 16-bit PCM WAV file (TPDF-dithered).
pub fn to_wav_i16(samples: &[f32], sample_rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut v = Vec::with_capacity(44 + data_len as usize);
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&(36 + data_len).to_le_bytes());
    v.extend_from_slice(b"WAVEfmt ");
    v.extend_from_slice(&16u32.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes()); // PCM
    v.extend_from_slice(&1u16.to_le_bytes()); // mono
    v.extend_from_slice(&sample_rate.to_le_bytes());
    v.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    v.extend_from_slice(&2u16.to_le_bytes());
    v.extend_from_slice(&16u16.to_le_bytes());
    v.extend_from_slice(b"data");
    v.extend_from_slice(&data_len.to_le_bytes());
    // Small deterministic LCG for TPDF dither.
    let mut seed: u32 = 0x1234_5678;
    let mut rnd = || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (seed >> 8) as f32 / (1u32 << 24) as f32
    };
    for &s in samples {
        let d = rnd() - rnd();
        let x = (s * 32767.0 + d).round().clamp(-32768.0, 32767.0) as i16;
        v.extend_from_slice(&x.to_le_bytes());
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peak_is_bounded() {
        let cmd: Command = "3234321".parse().unwrap();
        for sr in [44_100, 48_000, 96_000] {
            let cfg = SynthConfig { sample_rate: sr, ..Default::default() };
            let pcm = synthesize(cmd, &cfg);
            let peak = pcm.iter().fold(0.0f32, |m, s| m.max(s.abs()));
            assert!(peak <= cfg.amplitude + 1e-4 && peak > cfg.amplitude * 0.99, "sr={sr} peak={peak}");
            // starts and ends silent
            assert_eq!(pcm[0], 0.0);
            assert_eq!(*pcm.last().unwrap(), 0.0);
        }
    }

    #[test]
    fn little_out_of_band_energy() {
        // Energy below 17 kHz (audible "click" region) must be tiny compared with the in-band energy.
        let cfg = SynthConfig { repeat: 1, ..Default::default() };
        let pcm = synthesize("1212121".parse().unwrap(), &cfg);
        let sr = cfg.sample_rate as f64;
        let power = |f: f64| crate::decoder::goertzel_power(&pcm, f / sr);
        let inband = power(18750.0);
        let mut worst: f64 = 0.0;
        let mut f = 100.0;
        while f < 17000.0 {
            worst = worst.max(power(f));
            f += 50.0;
        }
        assert!(worst < inband * 1e-6, "worst={worst:e} inband={inband:e}");
    }
}
