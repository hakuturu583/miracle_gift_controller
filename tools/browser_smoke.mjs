// node tools/browser_smoke.mjs — loads web/ in headless Chrome, clicks a colour button and reports
// console errors + status text. Needs `python3 -m http.server 8765 -d web` running.
import { spawn } from 'node:child_process';
const chrome = spawn('google-chrome', ['--headless=new', '--remote-debugging-port=9333', '--autoplay-policy=no-user-gesture-required',
  '--user-data-dir=' + process.argv[2], 'about:blank'], { stdio: 'ignore' });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let tabs; for (let i = 0; i < 50; i++) { try { tabs = await (await fetch('http://127.0.0.1:9333/json')).json(); break; } catch { await sleep(200); } }
const ws = new WebSocket(tabs.find((t) => t.type === 'page').webSocketDebuggerUrl);
await new Promise((r) => (ws.onopen = r));
let id = 0; const pending = new Map(); const logs = [];
ws.onmessage = (m) => { const d = JSON.parse(m.data);
  if (d.id && pending.has(d.id)) { pending.get(d.id)(d.result); pending.delete(d.id); }
  if (d.method === 'Runtime.consoleAPICalled') logs.push(d.params.type + ': ' + d.params.args.map((a) => a.value ?? a.description).join(' '));
  if (d.method === 'Runtime.exceptionThrown') logs.push('EXCEPTION: ' + JSON.stringify(d.params.exceptionDetails.exception?.description ?? d.params.exceptionDetails.text));
};
const send = (method, params = {}) => new Promise((r) => { const i = ++id; pending.set(i, r); ws.send(JSON.stringify({ id: i, method, params })); });
const ev = async (expr) => (await send('Runtime.evaluate', { expression: expr, awaitPromise: true, returnByValue: true })).result.value;
await send('Runtime.enable');
await send('Page.navigate', { url: 'http://127.0.0.1:8765/index.html' });
await sleep(2500);
console.log('buttons:', await ev(`document.querySelectorAll('.cmd').length`));
await ev(`[...document.querySelectorAll('.cmd')].find(b => b.textContent.includes('red')).click()`);
await sleep(800);
console.log('status:', await ev(`document.querySelector('#status').textContent`));
console.log('now:', await ev(`document.querySelector('#now').textContent`));
await sleep(2500);
console.log('after end:', JSON.stringify(await ev(`document.querySelector('#now').textContent`)));
await ev(`document.querySelector('#code').value='9999999'; document.querySelector('#custom').requestSubmit()`);
await sleep(300);
console.log('bad code msg:', await ev(`document.querySelector('#code-msg').textContent`));
console.log('logs:', logs);
ws.close(); chrome.kill();
