# visual-source-rs

<img width="240" alt="basicallyLua logo" src="assets/logo.png" />

**visual-source-rs** is a Rust crate for creating, modifying and reading
RetroStudio's VisualSource scripts — plus two ways to use it without writing
any Rust: a command-line transpiler and a **100% offline browser app** with
the engine compiled to WebAssembly.

```
Luau source  ──transpile──▶  .vs payload  ──detranspile──▶  Luau source
   (To VS)                    (RetroStudio)                 (To Luau)
```

## What it does

- **Decode** any VisualSource V4 payload into a full block tree report
  (blocks, inputs, outputs, comments, editor camera, repair info).
- **Encode / re-encode** payloads **losslessly** — the Base93 + raw-DEFLATE
  pipeline reproduces the game's bytes layer by layer.
- **Transpile Luau → VisualSource** (the `To VS` direction): lexer, parser and
  block emitter covering variables, operators, control flow, loops, functions,
  events, remotes, tables, property paths and more.
- **Detranspile VisualSource → Luau** (the `To Luau` direction): walks the
  decoded block forest and reconstructs Luau source. The reconstruction is
  *semantic*, not byte-identical — `transpile(detranspile(p))` produces an
  equivalent tree, verified by the round-trip test suite.
- **Data-driven block catalog** (`src/vs_blocks.json`, 425 block types) used
  to validate every emitted block, input name and literal type.

## Use it in the browser (no install, no internet)

The compiled WebAssembly engine ships inside a **single self-contained HTML
file** (`site/index.html`, also deployed on GitHub Pages and included in every
release zip). Save it anywhere, double-click it, and it works offline — even
with no network at all, because everything (UI, dagre graph layout, the Rust
engine as `.wasm`) is embedded in that one file.

Tabs:

| Tab | Direction | Engine call |
| --- | --- | --- |
| Encoder | re-encode a payload losslessly | `encode_vs` |
| Decoder | payload → block-tree text report | `decode_vs` |
| To Luau | payload → reconstructed Luau | `toluau_vs` |
| To VS | Luau → encoded payload | `transpile_luau` |
| Roundtrip | verify a payload is lossless | `roundtrip_vs` |
| VS-Preview | payload → RetroStudio editor view (real save layout, category colors) | `vs_to_json` |

Hand-made RetroStudio payloads can contain block types the detranspiler does
not model yet; those are marked with `-- [detranspile] unsupported block: X`
comments instead of being silently dropped. Every payload the transpiler
itself can produce reconstructs with **zero gaps**.

## Use it from the command line

```bash
cargo build --release
./target/release/visual-source-rs transpile script.luau script.vs   # Luau → .vs
./target/release/visual-source-rs toluau   script.vs  script.luau   # .vs → Luau
./target/release/visual-source-rs decode    script.vs               # text report
./target/release/visual-source-rs encode    script.vs clean.vs      # lossless re-encode
./target/release/visual-source-rs roundtrip script.vs               # layer-by-layer proof
```

## Use it as a Rust crate

```toml
[dependencies]
visual-source-rs = "1.1"
```

```rust
use visual_source_rs::{transpile, VisualSource, encoder};

// Luau -> payload
let out = transpile::transpile("print(\"hello\")", "demo", true)?;
let payload = encoder::encode_payload(&out.report)?;

// payload -> Luau
let report = VisualSource::decode(&payload)?;
let luau = transpile::detranspile::detranspile(&report).luau;
```

## Building the WebAssembly engine

No `wasm-pack` needed — the wasm bindings are plain `#[unsafe(no_mangle)]`
extern "C" functions with a tiny memory protocol (see `wasm-src/src/lib.rs`).

```bash
rustup target add wasm32-unknown-unknown
cd wasm-src
cargo build --target wasm32-unknown-unknown --release
# → wasm-src/target/wasm32-unknown-unknown/release/visual_source_rs_wasm.wasm
```

To rebuild the self-contained site (inlines the wasm as base64):

```bash
# from the repository root, after the wasm build
python3 site/build.py
```

## Tests

```bash
cargo test
```

The suite is self-contained — no fixtures or sample files needed. Every
transpiled tree round-trips through the encoder with tree equality, stable
record streams and deterministic encoding (`tests/roundtrip.rs`), the
Luau → payload → Luau → payload path through the detranspiler is asserted
to reproduce an equivalent tree, and `tests/repro_split.rs` pins the
else-chain/loop-continuation regressions.

## Repository layout

```
src/                 the crate (decoder, encoder, transpiler, detranspiler)
  transpile/         lexer, parser, emitter, detranspiler
  vs_blocks.json     the 425-type block catalog
wasm-src/            the WebAssembly bindings (cdylib)
site/                the self-contained offline browser app
docs/                format documentation (FORMAT, TRANSPILE, BLOCK_CATALOG)
tests/               lossless + transpile + detranspile round-trip suites
```

## Credits

This project is built on top of the original
[visual-source-rs](https://github.com/Prouddani/visual-source-rs) crate by
**Prouddani** — the VisualSource decoder/encoder pipeline and the block
catalog began life there, and this repository would not exist without it.
The Luau transpiler, detranspiler, WebAssembly offline app, VS-Preview and
the round-trip test suites in this repository are new work, published under
the same MIT license.

## License

MIT — see [LICENSE](LICENSE).
