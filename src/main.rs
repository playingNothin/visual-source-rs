use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use visual_source_rs::{VisualSource, base93, deflate, encoder};

const USAGE: &str = "\
visual-source-rs CLI

USAGE:
  visual-source-rs decode     <input.vs> [output.txt]   Decode a .vs payload into a text report
  visual-source-rs encode     <input.vs> <output.vs>    Decode a payload and re-encode it losslessly
  visual-source-rs roundtrip  <input.vs>                Verify round-trip losslessness layer by layer
  visual-source-rs transpile  <input.luau> [output.vs] [--dump]  Transpile Luau into a VisualSource payload
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    match run(&args) {
        Ok(msg) => {
            println!("{}", msg);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {}", e);
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<String, String> {
    match args.get(1).map(String::as_str) {
        Some("decode") => {
            let input = arg(args, 2, "decode <input.vs> [output.txt]")?;
            let blob = read(&input)?;
            let report = VisualSource::decode(&blob).map_err(|e| e.to_string())?;
            let text = report.build_report(&input);
            match args.get(3) {
                Some(out) => {
                    fs::write(out, &text).map_err(|e| format!("cannot write {}: {}", out, e))?;
                    Ok(format!(
                        "decoded {} blocks, {} comments -> {}",
                        report.blocks.len(),
                        report.comments.len(),
                        out
                    ))
                }
                None => Ok(text),
            }
        }
        Some("encode") => {
            let input = arg(args, 2, "encode <input.vs> <output.vs>")?;
            let output = arg(args, 3, "encode <input.vs> <output.vs>")?;
            let blob = read(&input)?;
            let report = VisualSource::decode(&blob).map_err(|e| e.to_string())?;
            let payload = encoder::encode_payload(&report).map_err(|e| e.to_string())?;
            fs::write(&output, payload.as_bytes())
                .map_err(|e| format!("cannot write {}: {}", output, e))?;
            Ok(format!(
                "re-encoded {} blocks, {} comments -> {} ({} chars)",
                report.blocks.len(),
                report.comments.len(),
                output,
                payload.len()
            ))
        }
        Some("transpile") => {
            let input = arg(args, 2, "transpile <input.luau> [output.vs] [--dump]")?;
            let source = read(&input)?;
            let dump = args.iter().any(|a| a == "--dump" || a == "-v" || a == "--verbose");
            let out = transpile_source(&input, &source, dump)?;
            let output_path = args.get(3).filter(|a| !a.starts_with('-'));
            match output_path {
                Some(path) => {
                    fs::write(path, out.payload.as_bytes())
                        .map_err(|e| format!("cannot write {}: {}", path, e))?;
                    Ok(format!(
                        "{}\npayload written to {} ({} chars)",
                        out.report_text.trim_end(), path, out.payload.len()
                    ))
                }
                None => Ok(format!("{}\n\npayload ({} chars):\n{}", out.report_text.trim_end(), out.payload.len(), out.payload)),
            }
        }
        Some("roundtrip") => {
            let input = arg(args, 2, "roundtrip <input.vs>")?;
            let blob = read(&input)?;
            roundtrip_report(&input, &blob)
        }
        Some("toluau") => {
            let input = arg(args, 2, "toluau <input.vs> [output.luau]")?;
            let blob = read(&input)?;
            let label = args
                .get(3)
                .filter(|a| !a.starts_with('-'))
                .cloned()
                .unwrap_or_else(|| input.clone());
            let report = VisualSource::decode(&blob).map_err(|e| e.to_string())?;
            let out = visual_source_rs::transpile::detranspile::detranspile(&report);
            let mut text = out.luau;
            if !out.notes.is_empty() {
                text.push_str("\n-- detranspile notes:\n");
                for (kind, msg) in &out.notes {
                    text.push_str(&format!("-- [{}] {}\n", kind, msg));
                }
            }
            match args.get(3).filter(|a| !a.starts_with('-')) {
                Some(path) => {
                    fs::write(path, &text).map_err(|e| format!("cannot write {}: {}", path, e))?;
                    Ok(format!(
                        "detranspiled {} blocks -> {} ({} gap notes)",
                        report.blocks.len(),
                        path,
                        out.notes.iter().filter(|(k, _)| k == "GAP").count()
                    ))
                }
                None => Ok(text),
            }
        }
        _ => Err(USAGE.to_string()),
    }
}

fn arg(args: &[String], idx: usize, usage: &str) -> Result<String, String> {
    args.get(idx)
        .cloned()
        .ok_or_else(|| format!("missing argument: {}", usage))
}

fn read(path: &str) -> Result<String, String> {
    let p = PathBuf::from(path);
    let bytes = fs::read(&p).map_err(|e| format!("cannot read {}: {}", path, e))?;
    String::from_utf8(bytes).map_err(|e| format!("{} is not valid UTF-8: {}", path, e))
}

pub struct TranspileCliOutput {
    pub payload: String,
    pub report_text: String,
}

fn transpile_source(label: &str, source: &str, dump: bool) -> Result<TranspileCliOutput, String> {
    let out = visual_source_rs::transpile::transpile(source, label, true)?;
    let mut text = String::new();
    text.push_str(&format!("transpiled: {} ({} Luau lines)\n", label, source.lines().count()));
    text.push_str(&out.budget);
    text.push('\n');
    if !out.notes.is_empty() {
        text.push_str("\nnotes:\n");
        for n in &out.notes {
            text.push_str(&format!("  [{}] {}\n", n.kind, n.msg));
        }
    }
    if dump {
        text.push('\n');
        text.push_str(&out.report.build_report(label));
    }
    // full pipeline: encode the emitted forest into a payload
    let payload = encoder::encode_payload(&out.report).map_err(|e| e.to_string())?;
    Ok(TranspileCliOutput { payload, report_text: text })
}

/// Strips "\x1a" + 16-digit version tag + "\x1b" and returns the Base93 body.
fn payload_body(blob: &str) -> &str {
    let t = blob.trim_start_matches('\x1a');
    let t = match t.as_bytes().get(..16) {
        Some(head) if head.iter().all(|b| b.is_ascii_digit()) => &t[16..],
        _ => t,
    };
    t.strip_prefix('\x1b').unwrap_or(t)
}

fn roundtrip_report(label: &str, blob: &str) -> Result<String, String> {
    let report = VisualSource::decode(blob).map_err(|e| e.to_string())?;
    let mut lines = vec![format!("fixture: {} ({} chars)", label, blob.len())];

    lines.push(format!(
        "[1/5] decode ................ OK ({} blocks, {} comments)",
        report.blocks.len(),
        report.comments.len()
    ));

    // Layer 1: record stream must be byte-identical.
    let records = encoder::encode_records(&report);
    let records_ok = records == report.raw_decompressed;
    lines.push(format!(
        "[2/5] record stream ......... {} ({} bytes)",
        if records_ok { "BYTE-IDENTICAL" } else { "MISMATCH" },
        records.len()
    ));

    // Layer 2: Base93 encoder must reproduce the game's text bit-for-bit.
    let body = payload_body(blob);
    let raw_original = base93::decode(body).map_err(|e| e.to_string())?;
    let base93_ok = base93::encode(&raw_original) == body;
    lines.push(format!(
        "[3/5] base93 layer .......... {} ({} chars)",
        if base93_ok { "BYTE-IDENTICAL" } else { "MISMATCH" },
        body.len()
    ));

    // Layer 3: DEFLATE - same input bytes, implementation-dependent output.
    let ours = deflate::compress(&report.raw_decompressed).map_err(|e| e.to_string())?;
    let deflate_equal = ours == raw_original;
    lines.push(format!(
        "[4/5] deflate layer ......... {} (game {} bytes vs ours {} bytes; both are valid raw DEFLATE of the same {} bytes)",
        if deflate_equal { "BYTE-IDENTICAL" } else { "differs by compressor" },
        raw_original.len(),
        ours.len(),
        report.raw_decompressed.len()
    ));

    // Layer 4: full decode(encode(decode(X))) == decode(X).
    let payload = encoder::encode_payload(&report).map_err(|e| e.to_string())?;
    let decoded = VisualSource::decode(&payload).map_err(|e| e.to_string())?;
    let tree_ok = decoded.version == report.version
        && decoded.block_order == report.block_order
        && decoded.blocks == report.blocks
        && decoded.comments == report.comments
        && decoded.editor == report.editor;
    lines.push(format!(
        "[5/5] decode(encode(decode)) {}",
        if tree_ok { "TREE-IDENTICAL" } else { "TREE MISMATCH" }
    ));

    if records_ok && base93_ok && tree_ok {
        lines.push("RESULT: LOSSLESS (payload byte-equality impossible only at the DEFLATE layer: different compressors, same decompressed bytes)".to_string());
        Ok(lines.join("\n"))
    } else {
        Err(lines.join("\n"))
    }
}
