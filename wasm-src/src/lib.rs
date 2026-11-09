//! WebAssembly bindings for the browser UI — no wasm-bindgen dependency.
//!
//! Built with `cargo build --target wasm32-unknown-unknown --release`, so
//! the only toolchain requirement is the Rust std for wasm; no extra CLI
//! tools (wasm-pack / wasm-bindgen) are needed. The browser UI instantiates
//! the raw module and talks to it through a tiny C ABI.
//!
//! Memory protocol (every operation follows the same pattern):
//!   1. JS writes a UTF-8 input string into wasm memory at an offset
//!      returned by `engine_malloc(len)`.
//!   2. JS calls the operation with `(ptr, len[, label_ptr, label_len])`.
//!   3. The operation returns a `u64` packing `(ptr << 32) | len` of the
//!      result bytes (a JSON string).
//!   4. JS reads the bytes, then calls `engine_free(ptr, len)`.
//!
//! Results are JSON so JS parses once: success payloads carry the same
//! field names the old server API used (`output`, `stats`, `error`), and
//! errors come back as `{"error": "..."}`.

use std::alloc::{alloc, dealloc, Layout};
use std::sync::OnceLock;

use serde_json::{json, Value};

use visual_source_rs::hex::Hex;
use visual_source_rs::{decoder, encoder, transpile, vs_blocks, VisualSource};

/// Result packed as `(ptr << 32) | len`; wasm pointers stay far below 2^32.
fn pack(ptr: usize, len: usize) -> u64 {
    ((ptr as u64) << 32) | (len as u64)
}

/// Decodes a RetroStudio hex number ("0.1F4" -> 0.5) for preview geometry.
fn hex_num(s: &Option<String>) -> Option<f64> {
    s.as_ref().and_then(|v| Hex::from_hex(v)).map(|h| h.0)
}

/// Decodes an "x,y" hex pair ("3,-0.271" -> (3.0, -0.625)).
fn hex_xy(s: &Option<String>) -> Option<(f64, f64)> {
    s.as_ref().and_then(|v| {
        let (a, b) = v.split_once(',')?;
        Some((Hex::from_hex(a)?.0, Hex::from_hex(b)?.0))
    })
}

/// Allocate `len` bytes of uninitialized wasm memory for JS to write into.
#[unsafe(no_mangle)]
pub extern "C" fn engine_malloc(len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    let layout = Layout::from_size_align(len, 1).unwrap();
    unsafe { alloc(layout) as usize }
}

/// Free memory returned by `engine_malloc` or by an operation result.
#[unsafe(no_mangle)]
pub extern "C" fn engine_free(ptr: usize, len: usize) {
    if ptr == 0 || len == 0 {
        return;
    }
    let layout = Layout::from_size_align(len, 1).unwrap();
    unsafe { dealloc(ptr as *mut u8, layout) }
}

/// Copy `s` into freshly allocated memory and pack (ptr, len) for JS.
fn own(s: String) -> u64 {
    let bytes = s.into_bytes();
    let len = bytes.len();
    if len == 0 {
        return pack(0, 0);
    }
    let layout = Layout::from_size_align(len, 1).unwrap();
    let ptr = unsafe { alloc(layout) };
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, len);
    }
    pack(ptr as usize, len)
}

fn read_input(ptr: usize, len: usize) -> String {
    if ptr == 0 || len == 0 {
        return String::new();
    }
    let slice = unsafe { std::slice::from_raw_parts(ptr as *const u8, len) };
    String::from_utf8_lossy(slice).into_owned()
}

/// Pointer to a static null-terminated version string ("1.1.2\0").
#[unsafe(no_mangle)]
pub extern "C" fn engine_version() -> usize {
    static VERSION: OnceLock<String> = OnceLock::new();
    let s = VERSION.get_or_init(|| format!("{}\0", env!("CARGO_PKG_VERSION")));
    s.as_ptr() as usize
}

/// `transpile_luau(src, label)` -> JSON
/// `{"output":"<payload>","report":"...","notes":[...],"stats":{...}}`.
/// Field names mirror the old server's /luautos2 response so the UI logic
/// stays unchanged.
#[unsafe(no_mangle)]
pub extern "C" fn transpile_luau(
    src_ptr: usize,
    src_len: usize,
    label_ptr: usize,
    label_len: usize,
) -> u64 {
    let src = read_input(src_ptr, src_len);
    let label = read_input(label_ptr, label_len);
    let result = match transpile::transpile(&src, &label, true) {
        Ok(out) => {
            let mut text = String::new();
            text.push_str(&format!(
                "transpiled: {} ({} Luau lines)\n",
                label,
                src.lines().count()
            ));
            text.push_str(&out.budget);
            text.push('\n');
            if !out.notes.is_empty() {
                text.push_str("\nnotes:\n");
                for n in &out.notes {
                    text.push_str(&format!("  [{}] {}\n", n.kind, n.msg));
                }
            }
            let notes: Vec<Value> = out
                .notes
                .iter()
                .map(|n| json!({ "kind": n.kind, "msg": n.msg }))
                .collect();
            match encoder::encode_payload(&out.report) {
                Ok(payload) => json!({
                    "output": payload,
                    "report": text,
                    "notes": notes,
                    "stats": {
                        "blocks": out.report.blocks.len(),
                        "luau_lines": src.lines().count(),
                        "encoded_chars": payload.len(),
                        "gap": out.gap_count,
                    }
                }),
                Err(e) => json!({ "error": format!("encode: {}", e) }),
            }
        }
        Err(e) => json!({ "error": format!("transpile: {}", e) }),
    };
    own(result.to_string())
}

/// `decode_vs(blob, label)` -> JSON
/// `{"output":"<text report>","stats":{"version":4,...}}` (server /decode).
#[unsafe(no_mangle)]
pub extern "C" fn decode_vs(
    blob_ptr: usize,
    blob_len: usize,
    label_ptr: usize,
    label_len: usize,
) -> u64 {
    let blob = read_input(blob_ptr, blob_len);
    let label = read_input(label_ptr, label_len);
    let result = match VisualSource::decode(&blob) {
        Ok(report) => {
            let output = report.build_report(&label);
            json!({
                "output": output,
                "stats": {
                    "version": report.version,
                    "blocks": report.blocks.len(),
                    "comments": report.comments.len(),
                    "output_chars": output.len(),
                    "input_chars": blob.len(),
                }
            })
        }
        Err(e) => json!({ "error": format!("decode: {}", e) }),
    };
    own(result.to_string())
}

/// `encode_vs(blob)` -> JSON re-encoded payload (server /encode round-trip).
#[unsafe(no_mangle)]
pub extern "C" fn encode_vs(blob_ptr: usize, blob_len: usize) -> u64 {
    let blob = read_input(blob_ptr, blob_len);
    let result = match VisualSource::decode(&blob) {
        Ok(report) => match encoder::encode_payload(&report) {
            Ok(payload) => json!({
                "output": payload,
                "stats": {
                    "input_chars": blob.len(),
                    "output_chars": payload.len(),
                    "blocks": report.blocks.len(),
                    "comments": report.comments.len(),
                    "compressed_bytes": report.raw_decompressed.len(),
                }
            }),
            Err(e) => json!({ "error": format!("encode: {}", e) }),
        },
        Err(e) => json!({ "error": format!("decode: {}", e) }),
    };
    own(result.to_string())
}

/// `toluau_vs(blob)` -> JSON `{"output":"<luau>","notes":[...],"stats":{...}}`
/// (server /toluau). Detranspiles a .vs payload back into Luau source.
#[unsafe(no_mangle)]
pub extern "C" fn toluau_vs(blob_ptr: usize, blob_len: usize) -> u64 {
    let blob = read_input(blob_ptr, blob_len);
    let result = match VisualSource::decode(&blob) {
        Ok(report) => {
            let out = transpile::detranspile::detranspile(&report);
            let mut text = out.luau;
            if !out.notes.is_empty() {
                text.push_str("\n-- detranspile notes:\n");
                for (kind, msg) in &out.notes {
                    text.push_str(&format!("-- [{}] {}\n", kind, msg));
                }
            }
            let notes: Vec<Value> = out
                .notes
                .iter()
                .map(|(k, m)| json!({ "kind": k, "msg": m }))
                .collect();
            json!({
                "output": text,
                "notes": notes,
                "stats": {
                    "blocks": report.blocks.len(),
                    "comments": report.comments.len(),
                    "gaps": notes.iter().filter(|n| n["kind"] == "GAP").count(),
                    "output_chars": text.len(),
                }
            })
        }
        Err(e) => json!({ "error": format!("decode: {}", e) }),
    };
    own(result.to_string())
}

/// `roundtrip_vs(blob, label)` -> JSON `{"report":"...","lossless":bool}`.
#[unsafe(no_mangle)]
pub extern "C" fn roundtrip_vs(
    blob_ptr: usize,
    blob_len: usize,
    label_ptr: usize,
    label_len: usize,
) -> u64 {
    let blob = read_input(blob_ptr, blob_len);
    let label = read_input(label_ptr, label_len);
    let result = match decoder::decode_blob(&blob) {
        Ok(report) => {
            let records = encoder::encode_records(&report);
            let records_ok = records == report.raw_decompressed;
            let tree_ok = match encoder::encode_payload(&report) {
                Ok(payload) => match VisualSource::decode(&payload) {
                    Ok(r) => {
                        r.version == report.version
                            && r.block_order == report.block_order
                            && r.blocks == report.blocks
                            && r.comments == report.comments
                            && r.editor == report.editor
                    }
                    Err(_) => false,
                },
                Err(_) => false,
            };
            let overall = records_ok && tree_ok;
            json!({
                "report": format!(
                    "fixture: {} ({} chars)\n\
                     [1/3] decode ...... OK ({} blocks, {} comments)\n\
                     [2/3] records ..... {} ({} bytes)\n\
                     [3/3] re-decode ... {}\n\
                     RESULT: {}",
                    label,
                    blob.len(),
                    report.blocks.len(),
                    report.comments.len(),
                    if records_ok { "BYTE-IDENTICAL" } else { "MISMATCH" },
                    records.len(),
                    if tree_ok { "TREE-IDENTICAL" } else { "TREE MISMATCH" },
                    if overall { "LOSSLESS" } else { "MISMATCH" },
                ),
                "lossless": overall,
            })
        }
        Err(e) => json!({ "error": format!("decode: {}", e) }),
    };
    own(result.to_string())
}

/// `vs_to_json(blob)` -> graph JSON for the VS-Preview tab:
/// `{"version":N,"order":[names],"blocks":{name:{type,category,x,y,inputs,
/// outputs,children,else}},"comments":[{text,x,y,w,h}],"comments_count":N,
/// "warnings":[...]}`.
///
/// `x`/`y` are the save's own VisualPosition grid coordinates (top-left of
/// the 1.0x0.5 block footprint), so the preview draws the EXACT layout
/// RetroStudio shows in-game. Missing positions come back as null and the
/// UI falls back to an auto layout.
#[unsafe(no_mangle)]
pub extern "C" fn vs_to_json(blob_ptr: usize, blob_len: usize) -> u64 {
    let blob = read_input(blob_ptr, blob_len);
    let result = match VisualSource::decode(&blob) {
        Ok(report) => {
            let catalog = vs_blocks::catalog();
            let category = |t: &str| -> String {
                catalog
                    .get(t)
                    .and_then(|e| e.get("Category"))
                    .and_then(|c| c.as_str())
                    .unwrap_or("Miscellaneous")
                    .to_string()
            };
            let mut blocks = serde_json::Map::new();
            for name in &report.block_order {
                if let Some(b) = report.blocks.get(name) {
                    let inputs: serde_json::Map<String, Value> = b
                        .inputs
                        .iter()
                        .map(|i| {
                            let v = if i.use_var {
                                json!({ "var": i.var })
                            } else {
                                json!({
                                    "value": i.value.clone().unwrap_or_default(),
                                    "type": i.vtype.clone().unwrap_or_default(),
                                })
                            };
                            (i.iname.clone(), v)
                        })
                        .collect();
                    let outputs: serde_json::Map<String, Value> = b
                        .outputs
                        .iter()
                        .map(|(n, bnd)| (n.clone(), json!(bnd)))
                        .collect();
                    let (x, y) = hex_xy(&b.visual_position)
                        .map(|(x, y)| (json!(x), json!(y)))
                        .unwrap_or((Value::Null, Value::Null));
                    blocks.insert(
                        name.clone(),
                        json!({
                            "type": b.block_type,
                            "category": category(&b.block_type),
                            "x": x,
                            "y": y,
                            "inputs": inputs,
                            "outputs": outputs,
                            "children": b.children,
                            "else": b.else_child,
                        }),
                    );
                }
            }
            let comment_list: Vec<Value> = report
                .comments
                .iter()
                .filter_map(|c| {
                    let (x, y) = hex_xy(&c.position)?;
                    let (w, h) = hex_xy(&c.size).unwrap_or((1.75, 0.5));
                    Some(json!({
                        "text": c.text.clone().unwrap_or_default(),
                        "x": x,
                        "y": y,
                        "w": w,
                        "h": h,
                    }))
                })
                .collect();
            json!({
                "version": report.version,
                "order": report.block_order,
                "blocks": blocks,
                "comments": comment_list,
                "comments_count": report.comments.len(),
                "warnings": report.warnings,
            })
        }
        Err(e) => json!({ "error": format!("decode: {}", e) }),
    };
    own(result.to_string())
}

/// `find_block(name)` -> catalog entry for a block type, or `{"found":false}`.
#[unsafe(no_mangle)]
pub extern "C" fn find_block(name_ptr: usize, name_len: usize) -> u64 {
    let name = read_input(name_ptr, name_len);
    let catalog = visual_source_rs::vs_blocks::catalog();
    let result = match catalog.get(&name) {
        Some(entry) => json!({ "found": true, "name": name, "entry": entry }),
        None => json!({ "found": false, "name": name }),
    };
    own(result.to_string())
}
