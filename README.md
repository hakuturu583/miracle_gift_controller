# miracle_gift_controller

ミラクルギフトパレードのペンライト（ミラギフ棒）を、スマホや PC のスピーカーから出す近超音波で操作するツールです。Rust で書いた本体を WebAssembly にコンパイルしてブラウザで動かします。
プロトコルは [PROTOCOL.md](PROTOCOL.md) にまとめています。

## 構成

| パス | 内容 |
|---|---|
| `crates/miragift-core` | プロトコル、事前合成シンセ、ストリーミングデコーダ、コード表（128 件）。依存なし |
| `crates/miragift-wasm` | wasm-bindgen バインディング（`synthesize`, `synthesize_wav`, `commands_json`, `StreamDecoder`） |
| `crates/miragift-cli` | `miragift gen / list / decode / scan`（WAV の生成と解析） |
| `web/` | ブラウザ UI（色ボタン、コード直接入力、WAV 保存、マイク受信テスト） |
| `tools/gen_table.py` | 参照実装の `mlc.ts` から `table.rs` を生成する |

## ビルド

```sh
cargo test                                   # 全 128 コード × 44.1/48 kHz の合成→復号、帯域外漏れ、ノイズ耐性
wasm-pack build crates/miragift-wasm --target web --release --out-dir ../../web/pkg --no-typescript
python3 -m http.server 8765 -d web           # http://localhost:8765/
node tools/wasm_smoke.mjs                    # WASM 版の往復テスト
```

スマホから使うには `web/` を HTTPS で配信してください（GitHub Pages など）。マイク受信テストは HTTPS か localhost でしか動きません。

## CLI

```sh
cargo run --release -p miragift-cli -- list
cargo run --release -p miragift-cli -- gen red -o red.wav          # 名前でも 7 桁コードでも指定できる
cargo run --release -p miragift-cli -- decode rec.wav --segments
cargo run --release -p miragift-cli -- scan rec.wav                 # プロトコルを仮定しないトーン解析
```

## 成功率を上げるコツ

* 端末の音量は最大近くにし、UI の音量（既定 −6 dBFS）で調整する。歪むときは UI 側の音量を下げる
* Bluetooth スピーカーは避ける（SBC/AAC などのコーデックが 16 kHz 以上をカットすることが多い）。内蔵スピーカーか有線にする
* 「マイクで受信テスト」で余裕 (margin) が 10 dB 以上出る距離と音量を目安にする
