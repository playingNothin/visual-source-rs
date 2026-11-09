//! Phase 2 acceptance: round-trip losslessness over all fixtures.
//!
//! Layered guarantees:
//! 1. Record stream: `encode_records(decode(X))` is byte-identical to the
//!    original decompressed stream (the decompressed bytes ARE the ground
//!    truth carried in `raw_decompressed`).
//! 2. Base93: our encoder reproduces the game's Base93 text bit-for-bit
//!    (`encode(decode(payload)) == payload`), proving the Base93 layer is
//!    fully deterministic and matches the game's implementation.
//! 3. Full pipeline: `decode(encode(decode(X))) == decode(X)` (tree equality:
//!    blocks, order, comments, editor).
//!
//! Full-payload byte equality is NOT asserted: the game compresses with
//! LibDeflate; flate2/miniz_oxide emits a different (equally valid) raw
//! DEFLATE stream for the same bytes. zlib level sweep evidence: original
//! 8305 bytes sits between zlib level 5 (8433) and level 6 (8294) output,
//! so no level of any mainstream compressor reproduces it. See
//! docs/FORMAT.md section 6.

use visual_source_rs::{VisualSource, base93, deflate, encoder};

fn payload_body(blob: &str) -> &str {
    let t = blob.trim_start_matches('\x1a');
    let t = match t.as_bytes().get(..16) {
        Some(head) if head.iter().all(|b| b.is_ascii_digit()) => &t[16..],
        _ => t,
    };
    t.strip_prefix('\x1b').unwrap_or(t)
}

fn assert_roundtrip(label: &str, blob: &str) {
    let report = VisualSource::decode(blob).unwrap_or_else(|e| panic!("[{}] decode failed: {}", label, e));

    // --- Layer 1: record stream byte-equality -----------------------------
    let records = encoder::encode_records(&report);
    assert_eq!(
        records,
        report.raw_decompressed,
        "[{}] re-serialized record stream differs from the original decompressed bytes",
        label
    );

    // --- Layer 2: Base93 byte-equality -------------------------------------
    let body = payload_body(blob);
    let raw_original = base93::decode(body).unwrap();
    assert_eq!(
        base93::encode(&raw_original),
        body,
        "[{}] our Base93 encoder does not reproduce the game's payload text",
        label
    );

    // --- Layer 3: full decode(encode(decode(X))) == decode(X) --------------
    let payload = encoder::encode_payload(&report).unwrap();
    let red = VisualSource::decode(&payload).unwrap_or_else(|e| panic!("[{}] re-decode failed: {}", label, e));

    assert_eq!(red.version, report.version, "[{}] version", label);
    assert_eq!(red.block_order, report.block_order, "[{}] block order", label);
    assert_eq!(red.blocks, report.blocks, "[{}] blocks", label);
    assert_eq!(red.comments, report.comments, "[{}] comments", label);
    assert_eq!(red.editor, report.editor, "[{}] editor", label);

    // The re-decoded report must also re-serialize to the same bytes.
    assert_eq!(encoder::encode_records(&red), report.raw_decompressed, "[{}] second-pass records", label);

    // Our compressed layer is valid raw DEFLATE of the same record stream.
    let raw_ours = base93::decode(payload_body(&payload)).unwrap();
    assert_eq!(
        deflate::decompress(&raw_ours).unwrap(),
        report.raw_decompressed,
        "[{}] our DEFLATE layer does not inflate to the same records",
        label
    );

    // Encoding is deterministic: encode(decode(encode(decode(X)))) is stable.
    let payload_again = encoder::encode_payload(&red).unwrap();
    assert_eq!(payload_again, payload, "[{}] encoding not deterministic", label);

    // Byte-equality of the full payload is only blocked by the DEFLATE layer;
    // everything above it (header + Base93 of the same compressed bytes)
    // would be identical. Demonstrate that: re-compressing the ORIGINAL
    // compressed bytes through our Base93 + header reproduces the original
    // file byte-for-byte.
    let rebuilt = format!("\x1a{:016}\x1b{}", report.version, base93::encode(&raw_original));
    assert_eq!(rebuilt, blob, "[{}] header+Base93 wrapper is not byte-exact", label);
}

#[test]
fn test_roundtrip_script() {
    assert_roundtrip("script", include_str!("../fixtures/script.vs"));
}

#[test]
fn test_roundtrip_local_script() {
    assert_roundtrip("local_script", include_str!("../fixtures/local_script.vs"));
}

#[test]
fn test_roundtrip_module_script() {
    assert_roundtrip("module_script", include_str!("../fixtures/module_script.vs"));
}
