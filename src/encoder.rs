//! Encoder: [`DecodedReport`] -> VisualSource V4 payload string.
//!
//! Inverts [`crate::decoder`]. The canonical byte layout was derived
//! empirically from the three ground-truth fixtures (556 Block records, all
//! byte-identical when re-serialized):
//!
//! - Record order: `Editor` first, then `Block` records in `block_order`,
//!   then `Comment` records (comment order per the V4 format gist).
//! - Every `Block` record emits exactly the fields
//!   `Type, Name, VisualPosition, ChildBlocks, ElseChildBlock, Inputs, Outputs`
//!   in that order, all of them, always.
//! - `Editor` emits `CameraPosition, CameraZoom`.
//! - Input groups: mode 0 = `[name, 0, value]`, mode 1 = `[name, 1, value, type]`,
//!   mode 2 = `[name, 2, var]`.
//!
//! All strings (including hex numbers and positions) are passed through
//! verbatim from the decoded report, so `encode_records(decode(X))` is
//! byte-identical to the original decompressed stream.

use crate::base93;
use crate::deflate;
use crate::decoder::{DecodedInput, DecodedReport};

#[derive(Debug)]
pub enum EncodeError {
    DeflateError(std::io::Error),
}

impl From<std::io::Error> for EncodeError {
    fn from(e: std::io::Error) -> Self {
        Self::DeflateError(e)
    }
}

impl std::fmt::Display for EncodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DeflateError(e) => write!(f, "Deflate error: {}", e),
        }
    }
}

impl std::error::Error for EncodeError {}

/// Serializes the report into the raw (decompressed) record byte stream.
///
/// Deterministic: the same report always yields the same bytes.
pub fn encode_records(report: &DecodedReport) -> Vec<u8> {
    let mut out = Vec::with_capacity(report.decompressed_len.max(64));

    push_record_start(&mut out, b"Editor");
    push_opt(&mut out, b"CameraPosition", report.editor.camera_position.as_deref());
    push_opt(&mut out, b"CameraZoom", report.editor.camera_zoom.as_deref());

    for name in &report.block_order {
        let b = match report.blocks.get(name) {
            Some(b) => b,
            None => continue,
        };
        push_record_start(&mut out, b"Block");
        push_one(&mut out, b"Type", &b.block_type);
        push_one(&mut out, b"Name", &b.name);
        push_opt(&mut out, b"VisualPosition", b.visual_position.as_deref());
        push_list(&mut out, b"ChildBlocks", &b.children);
        push_one(&mut out, b"ElseChildBlock", b.else_child.as_deref().unwrap_or("nil"));
        push_list(&mut out, b"Inputs", &input_items(&b.inputs));
        push_list(&mut out, b"Outputs", &output_items(&b.outputs));
    }

    for c in &report.comments {
        push_record_start(&mut out, b"Comment");
        // Field order per the V4 format gist: Position, Size, Color, Name.
        push_opt(&mut out, b"Position", c.position.as_deref());
        push_opt(&mut out, b"Size", c.size.as_deref());
        push_opt(&mut out, b"Color", c.color.as_deref());
        push_opt(&mut out, b"Name", c.text.as_deref());
    }

    out
}

/// Builds a complete `.vs` payload string:
/// `\\x1a` + zero-padded 16-digit version tag + `\\x1b` + Base93(raw DEFLATE(records)).
pub fn encode_payload(report: &DecodedReport) -> Result<String, EncodeError> {
    let records = encode_records(report);
    let compressed = deflate::compress(&records)?;
    let body = base93::encode(&compressed);
    Ok(format!("\x1a{:016}\x1b{}", report.version, body))
}

fn push_record_start(out: &mut Vec<u8>, class: &[u8]) {
    out.extend_from_slice(&[0x1a, 0x1a]);
    out.extend_from_slice(class);
}

fn push_field(out: &mut Vec<u8>, name: &[u8]) {
    out.push(0x1a);
    out.extend_from_slice(name);
}

/// One mandatory value: `\x1a name \x1b value`.
fn push_one(out: &mut Vec<u8>, name: &[u8], value: &str) {
    push_field(out, name);
    out.push(0x1b);
    out.extend_from_slice(value.as_bytes());
}

/// One optional value: `\x1a name [\x1b value]` — absent value emits no `\x1b`.
fn push_opt(out: &mut Vec<u8>, name: &[u8], value: Option<&str>) {
    push_field(out, name);
    if let Some(v) = value {
        out.push(0x1b);
        out.extend_from_slice(v.as_bytes());
    }
}

/// A list field: `\x1a name (\x1b item)*` — empty list emits no `\x1b`.
fn push_list(out: &mut Vec<u8>, name: &[u8], values: &[String]) {
    push_field(out, name);
    for v in values {
        out.push(0x1b);
        out.extend_from_slice(v.as_bytes());
    }
}

fn input_items(inputs: &[DecodedInput]) -> Vec<String> {
    let mut items = Vec::with_capacity(inputs.len() * 3);
    for inp in inputs {
        if inp.use_var {
            items.push(inp.iname.clone());
            items.push("2".to_string());
            items.push(inp.var.clone());
        } else if let Some(vtype) = &inp.vtype {
            items.push(inp.iname.clone());
            items.push("1".to_string());
            items.push(inp.value.clone().unwrap_or_default());
            items.push(vtype.clone());
        } else {
            items.push(inp.iname.clone());
            items.push("0".to_string());
            items.push(inp.value.clone().unwrap_or_default());
        }
    }
    items
}

fn output_items(outputs: &[(String, String)]) -> Vec<String> {
    let mut items = Vec::with_capacity(outputs.len() * 2);
    for (oname, var) in outputs {
        items.push(oname.clone());
        items.push(var.clone());
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decoder::{
        DecodedBlock, DecodedComment, DecodedEditor, DecodedInput,
    };
    use crate::VisualSource;

    fn inp_implicit(name: &str, value: &str) -> DecodedInput {
        DecodedInput {
            iname: name.to_string(),
            use_var: false,
            var: String::new(),
            vtype: None,
            value: Some(value.to_string()),
        }
    }

    fn inp_explicit(name: &str, value: &str, vtype: &str) -> DecodedInput {
        DecodedInput {
            iname: name.to_string(),
            use_var: false,
            var: String::new(),
            vtype: Some(vtype.to_string()),
            value: Some(value.to_string()),
        }
    }

    fn inp_var(name: &str, var: &str) -> DecodedInput {
        DecodedInput {
            iname: name.to_string(),
            use_var: true,
            var: var.to_string(),
            vtype: None,
            value: None,
        }
    }

    fn sample_report() -> DecodedReport {
        let if_block = DecodedBlock {
            name: "If1".to_string(),
            block_type: "If".to_string(),
            visual_position: Some("1.2,3.4".to_string()),
            children: vec!["Print1".to_string()],
            else_child: Some("Warn1".to_string()),
            parents: vec![],
            inputs: vec![
                inp_implicit("Value1", "a"),
                inp_explicit("ComparisonType", "==", "Choice"),
                inp_var("Value2", "myVar"),
            ],
            outputs: vec![],
        };
        let print_block = DecodedBlock {
            name: "Print1".to_string(),
            block_type: "Print".to_string(),
            visual_position: Some("5,6".to_string()),
            children: vec![],
            else_child: None,
            parents: vec!["If1".to_string()],
            inputs: vec![inp_explicit("Text", "hi", "String")],
            outputs: vec![("Value".to_string(), String::new())],
        };

        let mut blocks = std::collections::HashMap::new();
        blocks.insert("If1".to_string(), if_block);
        blocks.insert("Print1".to_string(), print_block);

        DecodedReport {
            version: 4,
            payload_len: 0,
            raw_len: 0,
            decompressed_len: 0,
            inflate_method: String::new(),
            repair_mode: String::new(),
            block_order: vec!["If1".to_string(), "Print1".to_string()],
            blocks,
            comments: vec![DecodedComment {
                text: Some("note".to_string()),
                position: Some("0,0".to_string()),
                size: Some("2,1".to_string()),
                color: Some("1F4".to_string()),
            }],
            editor: DecodedEditor {
                camera_position: Some("-1.F3,1.4A".to_string()),
                camera_zoom: Some("0.92".to_string()),
            },
            raw_decompressed: Vec::new(),
            warnings: Vec::new(),
        }
    }

    #[test]
    fn test_input_item_shapes() {
        let inputs = vec![
            inp_implicit("A", "x"),
            inp_explicit("B", "y", "String"),
            inp_var("C", "v"),
        ];
        assert_eq!(
            input_items(&inputs),
            vec!["A", "0", "x", "B", "1", "y", "String", "C", "2", "v"]
        );
    }

    #[test]
    fn test_record_bytes_exact() {
        // Hand-checked serialization of the editor record + the If1 block.
        let report = sample_report();
        let bytes = encode_records(&report);
        let expected_head = b"\x1a\x1aEditor\
            \x1aCameraPosition\x1b-1.F3,1.4A\
            \x1aCameraZoom\x1b0.92\
            \x1a\x1aBlock\
            \x1aType\x1bIf\
            \x1aName\x1bIf1\
            \x1aVisualPosition\x1b1.2,3.4\
            \x1aChildBlocks\x1bPrint1\
            \x1aElseChildBlock\x1bWarn1\
            \x1aInputs\x1bValue1\x1b0\x1ba\x1bComparisonType\x1b1\x1b==\x1bChoice\x1bValue2\x1b2\x1bmyVar\
            \x1aOutputs";
        assert!(bytes.starts_with(expected_head), "got: {:?}", bytes);
        assert!(bytes.ends_with(
            b"\x1a\x1aComment\x1aPosition\x1b0,0\x1aSize\x1b2,1\x1aColor\x1b1F4\x1aName\x1bnote".as_slice()
        ));
    }

    #[test]
    fn test_payload_header_format() {
        let payload = encode_payload(&sample_report()).unwrap();
        assert!(payload.starts_with("\x1a0000000000000004\x1b"));
    }

    #[test]
    fn test_full_roundtrip_synthetic() {
        // Covers paths the fixtures never exercise: mode-2 inputs,
        // a non-nil ElseChildBlock, and a Comment record.
        let report = sample_report();
        let payload = encode_payload(&report).unwrap();
        let decoded = VisualSource::decode(&payload).unwrap();

        assert_eq!(decoded.version, 4);
        assert_eq!(decoded.block_order, report.block_order);
        assert_eq!(decoded.blocks, report.blocks);
        assert_eq!(decoded.comments, report.comments);
        assert_eq!(decoded.editor, report.editor);

        // Re-encoding the re-decoded report reproduces the same record bytes.
        assert_eq!(encode_records(&decoded), encode_records(&report));
    }
}
