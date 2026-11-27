// node tools/wasm_smoke.mjs  — smoke-test the built web/pkg in Node.
import { readFileSync } from 'node:fs';
import { initSync, synthesize, synthesize_wav, commands_json, normalize, StreamDecoder } from '../web/pkg/miragift_wasm.js';
initSync({ module: readFileSync(new URL('../web/pkg/miragift_wasm_bg.wasm', import.meta.url)) });
const t = JSON.parse(commands_json());
console.log('named', t.named.length, 'all', t.all.length);
let ok = 0;
for (const sr of [44100, 48000]) for (const k of t.all) {
  const pcm = synthesize(k.code, sr, 0.5, 1, 0.15, 0.3, 0.006);
  const d = new StreamDecoder(sr);
  const pk = [...JSON.parse(d.push(pcm)), ...JSON.parse(d.push(new Float32Array(sr)))];
  if (pk.length === 1 && pk[0].valid && pk[0].code === k.code) ok++; else console.log('FAIL', sr, k.code, pk);
}
console.log('roundtrip ok', ok, '/ 256');
console.log('wav bytes', synthesize_wav('3234321', 48000, 0.5, 2, 0.15, 0.3, 0.006).length);
try { normalize('2222222'); } catch (e) { console.log('rejects bad:', e.message); }
