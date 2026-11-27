//! wasm-bindgen API. The browser side only has to put the returned samples into an AudioBuffer
//! created at `AudioContext.sampleRate` and start one AudioBufferSourceNode (see web/app.js).

use miragift_core::decoder::Event;
use miragift_core::{synth, Command, Decoder, DecoderConfig, SynthConfig};
use wasm_bindgen::prelude::*;

fn parse(code: &str) -> Result<Command, JsError> {
    code.parse::<Command>().map_err(|e| JsError::new(&e.to_string()))
}

fn cfg(sample_rate: u32, amplitude: f32, repeat: u32, lead_in_sec: f64, gap_sec: f64, ramp_sec: f64, continuous: bool) -> SynthConfig {
    SynthConfig { sample_rate, amplitude, repeat, lead_in_sec, gap_sec, ramp_sec, continuous, ..Default::default() }
}

/// Render a command (e.g. "3234321") to mono f32 PCM at `sample_rate`.
#[wasm_bindgen]
pub fn synthesize(
    code: &str,
    sample_rate: u32,
    amplitude: f32,
    repeat: u32,
    lead_in_sec: f64,
    gap_sec: f64,
    ramp_sec: f64,
    continuous: bool,
) -> Result<Vec<f32>, JsError> {
    let cmd = parse(code)?;
    Ok(synth::synthesize(cmd, &cfg(sample_rate, amplitude, repeat, lead_in_sec, gap_sec, ramp_sec, continuous)))
}

/// Render a command to a 16-bit mono WAV file.
#[wasm_bindgen]
pub fn synthesize_wav(
    code: &str,
    sample_rate: u32,
    amplitude: f32,
    repeat: u32,
    lead_in_sec: f64,
    gap_sec: f64,
    ramp_sec: f64,
    continuous: bool,
) -> Result<Vec<u8>, JsError> {
    let cmd = parse(code)?;
    let pcm = synth::synthesize(cmd, &cfg(sample_rate, amplitude, repeat, lead_in_sec, gap_sec, ramp_sec, continuous));
    Ok(synth::to_wav_i16(&pcm, sample_rate))
}

/// Normalise / validate a command string; throws on invalid input.
#[wasm_bindgen]
pub fn normalize(code: &str) -> Result<String, JsError> {
    Ok(parse(code)?.to_string())
}

/// Reference ID (0..127) as used by ohtake/miracle-light-controller.
#[wasm_bindgen]
pub fn reference_id(code: &str) -> Result<u8, JsError> {
    Ok(parse(code)?.reference_id())
}

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn colors_json(c: &[miragift_core::Color]) -> String {
    let v: Vec<String> = c.iter().map(|c| format!("\"{}\"", c.css())).collect();
    format!("[{}]", v.join(","))
}

/// `{"named":[{group,name,code,bpm,colors}], "all":[{code,id,bpm,colors,strawberry}]}`
#[wasm_bindgen]
pub fn commands_json() -> String {
    let named: Vec<String> = miragift_core::named_commands()
        .iter()
        .map(|n| {
            format!(
                "{{\"group\":\"{}\",\"name\":\"{}\",\"code\":\"{}\",\"bpm\":{},\"colors\":{}}}",
                esc(n.group),
                esc(n.name),
                n.code,
                n.bpm,
                colors_json(n.colors)
            )
        })
        .collect();
    let all: Vec<String> = miragift_core::all_commands()
        .iter()
        .map(|k| {
            format!(
                "{{\"code\":\"{}\",\"id\":{},\"bpm\":{},\"colors\":{},\"strawberry\":{}}}",
                k.code,
                k.command().reference_id(),
                k.bpm,
                colors_json(k.colors),
                k.strawberry_only
            )
        })
        .collect();
    format!("{{\"named\":[{}],\"all\":[{}]}}", named.join(","), all.join(","))
}

/// Streaming decoder for microphone self-test.
#[wasm_bindgen]
pub struct StreamDecoder {
    inner: Decoder,
}

#[wasm_bindgen]
impl StreamDecoder {
    #[wasm_bindgen(constructor)]
    pub fn new(sample_rate: u32) -> StreamDecoder {
        StreamDecoder { inner: Decoder::new(DecoderConfig { sample_rate, ..Default::default() }) }
    }

    /// Feed samples; returns a JSON array of decoded packets
    /// `[{start, code, valid, margin_db, level_db, names:[..]}]`.
    pub fn push(&mut self, samples: &[f32]) -> String {
        let out: Vec<String> = self
            .inner
            .push(samples)
            .into_iter()
            .filter_map(|e| match e {
                Event::Packet(p) => {
                    let names: Vec<String> =
                        p.command.named().map(|n| format!("\"{}\"", esc(n.name))).collect();
                    Some(format!(
                        "{{\"start\":{:.3},\"code\":\"{}\",\"valid\":{},\"margin_db\":{:.1},\"level_db\":{:.1},\"names\":[{}]}}",
                        p.start,
                        p.command,
                        p.valid,
                        p.margin_db,
                        p.level_db,
                        names.join(",")
                    ))
                }
                _ => None,
            })
            .collect();
        format!("[{}]", out.join(","))
    }
}
