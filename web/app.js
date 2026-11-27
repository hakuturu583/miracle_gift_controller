// UI + Web Audio glue. All signal generation happens in Rust/WASM (pkg/).
//
// Playback strategy (why this is clean where a live Tone.js synth is not):
//  * the whole packet train is rendered sample-exactly in WASM at ctx.sampleRate, so the browser
//    never resamples and nothing depends on main-thread timer jitter;
//  * one AudioBufferSourceNode -> destination, no envelopes/effects in between;
//  * any packet still playing is stopped before a new one starts (no summing -> no clipping);
//  * leading silence + an optional always-running silent source keep the output device awake,
//    so the start of the preamble is not swallowed by device power-up/fade-in;
//  * iOS: navigator.audioSession.type = 'playback' so the ringer switch doesn't mute us.
import init, { synthesize, synthesize_wav, normalize, commands_json, StreamDecoder } from './pkg/miragift_wasm.js';

const $ = (s) => document.querySelector(s);
let ctx = null;
let keepAliveNode = null;
let current = null; // { src, button }

window.__miragiftStage = 'WASM 初期化 (init)';
await init();
window.__miragiftStage = 'コード表の読み込み';
const table = JSON.parse(commands_json());
window.__miragiftStage = 'ボタン生成';

function settings() {
  return {
    amp: parseFloat($('#amp').value),
    repeat: Math.max(1, Math.min(5, parseInt($('#repeat').value, 10) || 1)),
    lead: Math.max(0, parseFloat($('#lead').value) || 0),
    gap: Math.max(0.05, parseFloat($('#gap').value) || 0.3),
    ramp: Math.max(1, Math.min(30, parseFloat($('#ramp').value) || 6)) / 1000,
  };
}

async function audio() {
  if (!ctx) {
    try { if (navigator.audioSession) navigator.audioSession.type = 'playback'; } catch (_) {}
    const AC = window.AudioContext || window.webkitAudioContext;
    ctx = new AC({ latencyHint: 'playback' });
    $('#status').textContent = `オーディオ有効 (sampleRate ${ctx.sampleRate} Hz, 出力レイテンシ ${((ctx.outputLatency || ctx.baseLatency || 0) * 1000).toFixed(0)} ms)`;
  }
  if (ctx.state !== 'running') await ctx.resume();
  updateKeepAlive();
  return ctx;
}

function updateKeepAlive() {
  if (!ctx) return;
  const want = $('#keepalive').checked;
  if (want && !keepAliveNode) {
    // A running source with zero output keeps the audio stream (and the DAC/BT link) open.
    keepAliveNode = new ConstantSourceNode(ctx, { offset: 0 });
    keepAliveNode.connect(ctx.destination);
    keepAliveNode.start();
  } else if (!want && keepAliveNode) {
    keepAliveNode.stop();
    keepAliveNode.disconnect();
    keepAliveNode = null;
  }
}

async function play(code, button) {
  const c = await audio();
  const s = settings();
  let pcm;
  try {
    pcm = synthesize(code, c.sampleRate, s.amp, s.repeat, s.lead, s.gap, s.ramp);
  } catch (e) {
    $('#code-msg').textContent = String(e.message || e);
    return;
  }
  if (current) {
    current.src.onended = null;
    try { current.src.stop(); } catch (_) {}
    current.button?.classList.remove('playing');
  }
  const buf = c.createBuffer(2, pcm.length, c.sampleRate);
  buf.copyToChannel(pcm, 0);
  buf.copyToChannel(pcm, 1);
  const src = new AudioBufferSourceNode(c, { buffer: buf });
  src.connect(c.destination);
  src.start(c.currentTime + 0.03);
  button?.classList.add('playing');
  const me = { src, button };
  current = me;
  src.onended = () => {
    button?.classList.remove('playing');
    if (current === me) { current = null; $('#now').textContent = ''; }
  };
  $('#now').textContent = `送信中: ${code} × ${s.repeat} (${(pcm.length / c.sampleRate).toFixed(2)} s)`;
}

function swatch(colors, bpm) {
  const sw = document.createElement('div');
  sw.className = 'swatch';
  if (!colors.length) sw.classList.add('unknown');
  for (const col of colors) {
    const sp = document.createElement('span');
    sp.style.background = col;
    sw.append(sp);
  }
  return sw;
}

function button(code, name, colors, bpm, extra = '') {
  const b = document.createElement('button');
  b.className = 'cmd';
  b.type = 'button';
  const sw = swatch(colors, bpm);
  const n = document.createElement('span'); n.className = 'name'; n.textContent = name + extra;
  const cd = document.createElement('span'); cd.className = 'code'; cd.textContent = code;
  b.append(sw, n, cd);
  b.addEventListener('click', () => { $('#status').textContent = 'タップ受付: ' + code; play(code, b); });
  return b;
}

for (const n of table.named) {
  document.querySelector(`[data-group="${n.group}"]`)?.append(button(n.code, n.name, n.colors, n.bpm));
}
const allGrid = document.querySelector('[data-group="all"]');
for (const k of [...table.all].sort((a, b) => a.id - b.id)) {
  allGrid.append(button(k.code, `#${k.id}`, k.colors, k.bpm, k.strawberry ? ' 🍓' : ''));
}

$('#custom').addEventListener('submit', (e) => {
  e.preventDefault();
  $('#code-msg').textContent = '';
  try {
    play(normalize($('#code').value), null);
  } catch (err) {
    $('#code-msg').textContent = String(err.message || err);
  }
});

$('#dl').addEventListener('click', () => {
  $('#code-msg').textContent = '';
  try {
    const code = normalize($('#code').value);
    const s = settings();
    const wav = synthesize_wav(code, 48000, s.amp, s.repeat, s.lead, s.gap, s.ramp);
    const a = document.createElement('a');
    a.href = URL.createObjectURL(new Blob([wav], { type: 'audio/wav' }));
    a.download = `miragift-${code}.wav`;
    a.click();
    setTimeout(() => URL.revokeObjectURL(a.href), 10000);
  } catch (err) {
    $('#code-msg').textContent = String(err.message || err);
  }
});

const ampOut = () => { $('#amp-o').textContent = `${(20 * Math.log10(parseFloat($('#amp').value))).toFixed(1)} dBFS`; };
$('#amp').addEventListener('input', ampOut);
ampOut();
$('#keepalive').addEventListener('change', updateKeepAlive);

// ---- microphone self-test -------------------------------------------------------------------
const WORKLET = `
class Tap extends AudioWorkletProcessor {
  constructor() { super(); this.buf = new Float32Array(2048); this.n = 0; }
  process(inputs) {
    const ch = inputs[0] && inputs[0][0];
    if (ch) {
      for (let i = 0; i < ch.length; i++) {
        this.buf[this.n++] = ch[i];
        if (this.n === this.buf.length) { this.port.postMessage(this.buf.slice()); this.n = 0; }
      }
    }
    return true;
  }
}
registerProcessor('miragift-tap', Tap);`;

let mic = null;
$('#mic').addEventListener('click', async () => {
  const c = await audio();
  if (mic) {
    mic.stream.getTracks().forEach((t) => t.stop());
    mic.node.disconnect();
    mic = null;
    $('#mic').textContent = '受信テスト開始';
    return;
  }
  const stream = await navigator.mediaDevices.getUserMedia({
    audio: { echoCancellation: false, noiseSuppression: false, autoGainControl: false, channelCount: 1 },
  });
  await c.audioWorklet.addModule(URL.createObjectURL(new Blob([WORKLET], { type: 'text/javascript' })));
  const node = new AudioWorkletNode(c, 'miragift-tap', { numberOfOutputs: 0 });
  c.createMediaStreamSource(stream).connect(node);
  const dec = new StreamDecoder(c.sampleRate);
  const nameOf = new Map(table.named.map((n) => [n.code, n.name]));
  node.port.onmessage = (e) => {
    for (const p of JSON.parse(dec.push(e.data))) {
      const li = document.createElement('li');
      if (!p.valid) li.className = 'bad';
      li.textContent = `${p.valid ? 'OK ' : 'NG '} ${p.code} ${nameOf.get(p.code) ?? ''}  余裕 ${p.margin_db} dB / レベル ${p.level_db} dBFS`;
      $('#miclog').prepend(li);
    }
  };
  mic = { stream, node };
  $('#mic').textContent = '受信テスト停止';
});

clearTimeout(window.__miragiftBoot);
window.__miragiftStage = '起動完了';
