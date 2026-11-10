use std::collections::HashMap;
use crate::base93;
use crate::deflate;
use crate::block::{Block, BlockInput, BlockInputVisibility, BlockOutput, BlockOutputValueType};
use crate::editor::Editor;
use crate::field_types::string::VSString;
use crate::field_types::vector2::VSVector2;
use crate::unique_id::UniqueIdService;
use crate::VisualSource;

#[derive(Debug)]
pub enum DecodeError {
    HeaderError(String),
    Base93Error(base93::Base93Error),
    DeflateError(std::io::Error),
    ParseError(String),
}

impl From<base93::Base93Error> for DecodeError {
    fn from(e: base93::Base93Error) -> Self {
        Self::Base93Error(e)
    }
}

impl From<std::io::Error> for DecodeError {
    fn from(e: std::io::Error) -> Self {
        Self::DeflateError(e)
    }
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::HeaderError(s) => write!(f, "Header error: {}", s),
            Self::Base93Error(e) => write!(f, "Base93 error: {}", e),
            Self::DeflateError(e) => write!(f, "Deflate error: {}", e),
            Self::ParseError(s) => write!(f, "Parse error: {}", s),
        }
    }
}

impl std::error::Error for DecodeError {}

#[derive(Clone, Debug, PartialEq)]
pub struct DecodedInput {
    pub iname: String,
    pub use_var: bool,
    pub var: String,
    pub vtype: Option<String>,
    pub value: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DecodedBlock {
    pub name: String,
    pub block_type: String,
    pub visual_position: Option<String>,
    pub children: Vec<String>,
    pub else_child: Option<String>,
    pub parents: Vec<String>,
    pub inputs: Vec<DecodedInput>,
    pub outputs: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DecodedComment {
    pub text: Option<String>,
    pub position: Option<String>,
    pub size: Option<String>,
    pub color: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct DecodedEditor {
    pub camera_position: Option<String>,
    pub camera_zoom: Option<String>,
}

#[derive(Clone, Debug)]
pub struct DecodedReport {
    pub version: u8,
    pub payload_len: usize,
    pub raw_len: usize,
    pub decompressed_len: usize,
    pub inflate_method: String,
    pub repair_mode: String,
    pub block_order: Vec<String>,
    pub blocks: HashMap<String, DecodedBlock>,
    pub comments: Vec<DecodedComment>,
    pub editor: DecodedEditor,
    pub raw_decompressed: Vec<u8>,
    pub warnings: Vec<String>,
}

const HEX_CHARS: &[u8] = b"0123456789abcdefABCDEF";

pub fn try_hex(s: &str) -> Option<i64> {
    if s.is_empty() {
        return None;
    }
    let (neg, core) = if let Some(stripped) = s.strip_prefix('-') {
        (true, stripped)
    } else {
        (false, s)
    };
    if core.is_empty() || core.bytes().any(|b| !HEX_CHARS.contains(&b)) {
        return None;
    }
    let v = i64::from_str_radix(core, 16).ok()?;
    Some(if neg { -v } else { v })
}

pub fn fmt_num(s: Option<&str>) -> String {
    let s = match s {
        Some(v) => v,
        None => return "0".to_string(),
    };
    if s.contains('.') {
        let (whole, frac) = s.split_once('.').unwrap();
        let f = match try_hex(frac) {
            Some(v) => v,
            None => return "0".to_string(),
        };
        let (neg, whole_core) = if let Some(stripped) = whole.strip_prefix('-') {
            (true, stripped)
        } else {
            (false, whole)
        };
        let w = try_hex(whole_core).unwrap_or(0);
        let val = (w as f64) + (f as f64) / 1000.0;
        let final_val = if neg { -val } else { val };
        numstr(final_val)
    } else if let Some(w) = try_hex(s) {
        w.to_string()
    } else {
        "0".to_string()
    }
}

fn numstr(x: f64) -> String {
    if x == (x as i64 as f64) {
        format!("{}", x as i64)
    } else {
        let s = format!("{}", x);
        s
    }
}

const PASSTHROUGH_TYPES: &[&str] = &[
    "String",
    "Choice",
    "Object",
    "Table",
    "Function",
    "EventConnection",
    "Equation",
];

pub fn format_typed(v: Option<&str>, t: &str) -> String {
    let v_str = v.unwrap_or("");
    if PASSTHROUGH_TYPES.contains(&t) {
        return format!("\"{}\"", v_str);
    }
    if t == "Number" {
        return format!("{} (hex {:?})", fmt_num(v), v_str);
    }
    if t == "Bool" {
        let bool_str = if v_str == "1" { "true" } else { "false" };
        return format!("{} (raw {:?})", bool_str, v_str);
    }
    if t == "BrickColor" {
        return format!("BrickColor({})", fmt_num(v));
    }
    if t == "Color3" || t == "Vector3" {
        let p: Vec<&str> = v_str.split(',').collect();
        let x = p.get(0).copied();
        let y = p.get(1).copied();
        let z = p.get(2).copied();
        return format!("{}({}, {}, {})", t, fmt_num(x), fmt_num(y), fmt_num(z));
    }
    if t == "Vector2" {
        let p: Vec<&str> = v_str.split(',').collect();
        let x = p.get(0).copied();
        let y = p.get(1).copied();
        return format!("Vector2({}, {})", fmt_num(x), fmt_num(y));
    }
    if t == "UDim2" {
        let p: Vec<&str> = v_str.split(',').collect();
        let formatted: Vec<String> = (0..4).map(|i| fmt_num(p.get(i).copied())).collect();
        return format!("UDim2({})", formatted.join(", "));
    }
    if t == "Tuple" {
        if v_str.is_empty() {
            return "[]".to_string();
        }
        let parts: Vec<&str> = v_str.split(',').collect();
        return format!("[{}]", parts.join(", "));
    }
    format!("{:?} <{}>", v_str, t)
}

pub fn heur(v: Option<&str>) -> String {
    let v_str = match v {
        Some(s) => s,
        None => return "nil".to_string(),
    };
    if v_str.is_empty() {
        return "\"\"".to_string();
    }
    if let Some(h) = try_hex(v_str) {
        if let Ok(d) = v_str.parse::<i64>() {
            if d == h {
                return format!("\"{}\"", v_str);
            }
        }
        return format!("\"{}\"  [hex number: {}]", v_str, h);
    }
    if v_str.contains('.') {
        let (whole, frac) = v_str.split_once('.').unwrap();
        if !whole.is_empty()
            && !frac.is_empty()
            && try_hex(whole).is_some()
            && try_hex(frac).is_some()
        {
            return format!("\"{}\"  [hex number: {}]", v_str, fmt_num(Some(v_str)));
        }
    }
    if v_str.contains(',') {
        let parts: Vec<&str> = v_str.split(',').collect();
        let hex_parts: Option<Vec<i64>> = parts.iter().map(|p| try_hex(p)).collect();
        if let Some(hs) = hex_parts {
            let str_parts: Vec<String> = hs.iter().map(|h| h.to_string()).collect();
            return format!("\"{}\"  [hex parts: ({})]", v_str, str_parts.join(", "));
        }
        return format!("\"{}\"  [comma-separated]", v_str);
    }
    format!("\"{}\"", v_str)
}

pub fn fmt_input(iname: &str, input: &DecodedInput) -> String {
    if input.use_var {
        let var_name = if input.var.is_empty() {
            "?"
        } else {
            &input.var
        };
        format!("in  {} = ${}", iname, var_name)
    } else if let Some(ref vtype) = input.vtype {
        format!(
            "in  {} = {}   [type: {}]",
            iname,
            format_typed(input.value.as_deref(), vtype),
            vtype
        )
    } else {
        format!("in  {} = {}", iname, heur(input.value.as_deref()))
    }
}

/// One rendered output row: a plain output, or a merged tuple group.
enum OutRow {
    Plain(String, String),
    Tuple {
        tuple: String,
        arity: Option<i64>,
        arity_raw: String,
        elems: Vec<(u32, String)>,
    },
}

/// Splits encoded tuple wire names:
/// `TUPLEPARAM_Parameters_1` -> ("Parameters", Some(1)), `TUPLE_Parameters` -> ("Parameters", None).
fn split_tuple_name(name: &str) -> Option<(String, Option<u32>)> {
    if let Some(rest) = name.strip_prefix("TUPLEPARAM_") {
        let cut = rest.rfind('_')?;
        let idx = rest[cut + 1..].parse::<u32>().ok()?;
        Some((rest[..cut].to_string(), Some(idx)))
    } else {
        let tuple = name.strip_prefix("TUPLE_")?;
        if tuple.is_empty() {
            None
        } else {
            Some((tuple.to_string(), None))
        }
    }
}

/// Formats outputs for the human-readable tree view. Encoded tuples arrive as
/// a `TUPLE_X` wire (its value being the parameter count, stored as a hex
/// string) plus one `TUPLEPARAM_X_N` wire per element; both collapse into a
/// single `out X (tuple of N) -> ($a, $b, ...)` line. Plain outputs keep the
/// `out NAME -> $VAR` shape. Raw names stay available in render_flat().
pub fn fmt_output_lines(outputs: &[(String, String)]) -> Vec<String> {
    let mut rows: Vec<OutRow> = Vec::new();
    let mut pos_of: HashMap<String, usize> = HashMap::new();
    let mut group_pos = |tuple: &str, rows: &mut Vec<OutRow>, pos_of: &mut HashMap<String, usize>| -> usize {
        if let Some(&p) = pos_of.get(tuple) {
            return p;
        }
        rows.push(OutRow::Tuple {
            tuple: tuple.to_string(),
            arity: None,
            arity_raw: String::new(),
            elems: Vec::new(),
        });
        pos_of.insert(tuple.to_string(), rows.len() - 1);
        rows.len() - 1
    };
    for (name, var) in outputs {
        match split_tuple_name(name) {
            None => rows.push(OutRow::Plain(name.clone(), var.clone())),
            Some((tuple, Some(idx))) => {
                let pos = group_pos(&tuple, &mut rows, &mut pos_of);
                if let OutRow::Tuple { elems, .. } = &mut rows[pos] {
                    elems.push((idx, var.clone()));
                }
            }
            Some((tuple, None)) => {
                let pos = group_pos(&tuple, &mut rows, &mut pos_of);
                if let OutRow::Tuple {
                    arity, arity_raw, ..
                } = &mut rows[pos]
                {
                    // The count is a hex string; cap it so a misread value
                    // cannot spray thousands of "?" placeholders.
                    match try_hex(var) {
                        Some(v) if v >= 0 && v <= 64 => *arity = Some(v),
                        _ => *arity_raw = var.clone(),
                    }
                }
            }
        }
    }
    rows.iter()
        .map(|r| match r {
            OutRow::Plain(name, var) => format!("out {} -> ${}", name, var),
            OutRow::Tuple {
                tuple,
                arity,
                arity_raw,
                elems,
            } => {
                let mut es = elems.clone();
                es.sort_by_key(|(i, _)| *i);
                let (count_label, list) = match *arity {
                    Some(n) => (
                        format!(" of {}", n),
                        (1..=n as u32)
                            .map(|i| {
                                es.iter()
                                    .find(|(idx, _)| *idx == i)
                                    .map(|(_, v)| format!("${}", v))
                                    .unwrap_or_else(|| "?".to_string())
                            })
                            .collect::<Vec<_>>(),
                    ),
                    None if !arity_raw.is_empty() => (
                        format!(" of {}", arity_raw),
                        es.iter().map(|(_, v)| format!("${}", v)).collect::<Vec<_>>(),
                    ),
                    None => (
                        String::new(),
                        es.iter().map(|(_, v)| format!("${}", v)).collect::<Vec<_>>(),
                    ),
                };
                let body = if list.is_empty() {
                    "(no elements)".to_string()
                } else {
                    format!("({})", list.join(", "))
                };
                format!("out {} (tuple{}) -> {}", tuple, count_label, body)
            }
        })
        .collect()
}

/// Rewrites encoded tuple wire references inside a rendered line for the
/// human-readable tree view: `TUPLEPARAM_X_N` -> `X[N]`, `TUPLE_X` -> `X (tuple)`.
/// Raw names remain visible in render_flat().
pub fn pretty_wire_refs(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if line[i..].starts_with("TUPLEPARAM_") || line[i..].starts_with("TUPLE_") {
            let start = i;
            let mut j = i;
            while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
                j += 1;
            }
            let token = &line[start..j];
            match split_tuple_name(token) {
                Some((tuple, Some(idx))) => {
                    out.push_str(&tuple);
                    out.push_str(&format!("[{}]", idx));
                }
                Some((tuple, None)) => {
                    out.push_str(&tuple);
                    out.push_str(" (tuple)");
                }
                None => out.push_str(token),
            }
            i = j;
        } else {
            let ch = line[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

pub fn extract_payload(text: &str) -> Result<(u8, String), DecodeError> {
    let mut t = text.replace("\\x1a", "\x1a").replace("\\x1b", "\x1b");
    if t.starts_with('\x1a') {
        t.remove(0);
    }
    if t.starts_with("0000000000000004") {
        t = t["0000000000000004".len()..].to_string();
        if t.starts_with('\x1b') {
            t.remove(0);
        }
        return Ok((4, t));
    }
    if t.starts_with("0000000000000003") {
        t = t["0000000000000003".len()..].to_string();
        if t.starts_with('\x1b') {
            t.remove(0);
        }
        return Ok((3, t));
    }
    // assume bare v4 payload
    Ok((4, t))
}

pub fn decode_blob(blob_text: &str) -> Result<DecodedReport, DecodeError> {
    // Strip line breaks
    let base: String = blob_text
        .chars()
        .filter(|&c| c != '\r' && c != '\n' && c != '\t' && c != '\x0b' && c != '\x0c')
        .collect();

    // Try variants matching python.py
    let variants = [
        ("as pasted (line breaks removed)", base.clone()),
        ("outer spaces trimmed", base.trim_matches(' ').to_string()),
        (
            "all whitespace removed (chat-wrap repair)",
            base.chars().filter(|c| !c.is_whitespace()).collect(),
        ),
    ];

    let mut last_err = None;
    for (vlabel, variant_text) in variants {
        let (version, payload) = match extract_payload(&variant_text) {
            Ok(res) => res,
            Err(e) => {
                last_err = Some(e);
                continue;
            }
        };

        if version != 4 {
            last_err = Some(DecodeError::HeaderError(format!("unsupported version v{}", version)));
            continue;
        }

        let raw_bytes = match base93::decode(&payload) {
            Ok(b) => b,
            Err(e) => {
                last_err = Some(DecodeError::Base93Error(e));
                continue;
            }
        };

        let decompressed = match deflate::decompress(&raw_bytes) {
            Ok(d) => d,
            Err(e) => {
                last_err = Some(DecodeError::DeflateError(e));
                continue;
            }
        };

        match parse_records(
            version,
            payload.len(),
            raw_bytes.len(),
            decompressed,
            vlabel.to_string(),
        ) {
            Ok(report) => return Ok(report),
            Err(e) => {
                last_err = Some(e);
            }
        }
    }

    Err(last_err.unwrap_or_else(|| DecodeError::ParseError("All decode attempts failed".to_string())))
}

fn dec_str(b: &[u8]) -> String {
    String::from_utf8(b.to_vec())
        .unwrap_or_else(|_| b.iter().map(|&c| c as char).collect())
}

fn parse_records(
    version: u8,
    payload_len: usize,
    raw_len: usize,
    decompressed: Vec<u8>,
    repair_mode: String,
) -> Result<DecodedReport, DecodeError> {
    let mut warnings = Vec::new();
    let sep = &[0x1a, 0x1a];
    let records_raw: Vec<usize> = decompressed
        .windows(2)
        .enumerate()
        .filter_map(|(i, w)| if w == sep { Some(i) } else { None })
        .collect();

    let mut record_slices = Vec::new();
    if decompressed.starts_with(sep) {
        let mut prev = 2;
        for &idx in &records_raw[1..] {
            record_slices.push(&decompressed[prev..idx]);
            prev = idx + 2;
        }
        record_slices.push(&decompressed[prev..]);
    } else {
        return Err(DecodeError::ParseError(
            "Data does not start with record separator (\\x1a\\x1a)".to_string(),
        ));
    }

    let mut block_order = Vec::new();
    let mut blocks = HashMap::new();
    let mut comments = Vec::new();
    let mut editor = DecodedEditor::default();

    for rec in record_slices {
        let idx = match rec.iter().position(|&b| b == 0x1a) {
            Some(i) => i,
            None => {
                return Err(DecodeError::ParseError(
                    "Record without a type splitter".to_string(),
                ));
            }
        };

        let rclass = dec_str(&rec[..idx]);
        let mut fields: HashMap<String, Vec<String>> = HashMap::new();

        for field in rec[idx + 1..].split(|&b| b == 0x1a) {
            let pieces: Vec<String> = field.split(|&b| b == 0x1b).map(dec_str).collect();
            if !pieces.is_empty() {
                let fname = pieces[0].clone();
                let fvals = pieces[1..].to_vec();
                fields.insert(fname, fvals);
            }
        }

        if rclass == "Block" {
            let name = match fields.get("Name").and_then(|v| v.first()) {
                Some(n) => n.clone(),
                None => {
                    return Err(DecodeError::ParseError(
                        "Block record without a Name".to_string(),
                    ));
                }
            };

            let btype = fields
                .get("Type")
                .and_then(|v| v.first())
                .cloned()
                .unwrap_or_else(|| "?".to_string());

            let children = fields.get("ChildBlocks").cloned().unwrap_or_default();
            let else_child = fields
                .get("ElseChildBlock")
                .and_then(|v| v.first())
                .filter(|s| !s.is_empty() && s.as_str() != "nil")
                .cloned();

            let visual_position = fields.get("VisualPosition").and_then(|v| v.first()).cloned();

            let raw_in = fields.get("Inputs").cloned().unwrap_or_default();
            let mut inputs = Vec::new();
            let mut i = 0;
            let n_in = raw_in.len();

            while i + 1 < n_in {
                let iname = raw_in[i].clone();
                let mode: Option<u32> = raw_in[i + 1].parse().ok();
                let value = if i + 2 < n_in {
                    Some(raw_in[i + 2].clone())
                } else {
                    None
                };

                match mode {
                    Some(0) => {
                        inputs.push(DecodedInput {
                            iname,
                            use_var: false,
                            var: String::new(),
                            vtype: None,
                            value,
                        });
                        i += 3;
                    }
                    Some(1) => {
                        let vtype = if i + 3 < n_in {
                            Some(raw_in[i + 3].clone())
                        } else {
                            None
                        };
                        inputs.push(DecodedInput {
                            iname,
                            use_var: false,
                            var: String::new(),
                            vtype,
                            value,
                        });
                        i += 4;
                    }
                    Some(2) => {
                        inputs.push(DecodedInput {
                            iname,
                            use_var: true,
                            var: value.unwrap_or_default(),
                            vtype: None,
                            value: None,
                        });
                        i += 3;
                    }
                    _ => {
                        i += 3;
                    }
                }
            }

            let raw_out = fields.get("Outputs").cloned().unwrap_or_default();
            let mut outputs = Vec::new();
            let n_out = raw_out.len() - (raw_out.len() % 2);
            let mut j = 0;
            while j < n_out {
                outputs.push((raw_out[j].clone(), raw_out[j + 1].clone()));
                j += 2;
            }

            let block = DecodedBlock {
                name: name.clone(),
                block_type: btype,
                visual_position,
                children,
                else_child,
                parents: Vec::new(),
                inputs,
                outputs,
            };

            block_order.push(name.clone());
            blocks.insert(name, block);
        } else if rclass == "Comment" {
            comments.push(DecodedComment {
                text: fields.get("Name").and_then(|v| v.first()).cloned(),
                position: fields.get("Position").and_then(|v| v.first()).cloned(),
                size: fields.get("Size").and_then(|v| v.first()).cloned(),
                color: fields.get("Color").and_then(|v| v.first()).cloned(),
            });
        } else if rclass == "Editor" {
            editor.camera_position = fields.get("CameraPosition").and_then(|v| v.first()).cloned();
            editor.camera_zoom = fields.get("CameraZoom").and_then(|v| v.first()).cloned();
        } else {
            warnings.push(format!("unknown record class {:?} (skipped)", rclass));
        }
    }

    // Reconstruct parents
    for name in &block_order {
        let b = blocks.get(name).unwrap().clone();
        for child in &b.children {
            if child.is_empty() {
                continue;
            }
            if let Some(target) = blocks.get_mut(child) {
                target.parents.push(name.clone());
            } else {
                warnings.push(format!(
                    "missing child block {:?} referenced by [{}]",
                    child, name
                ));
            }
        }
        if let Some(ref else_c) = b.else_child {
            if let Some(target) = blocks.get_mut(else_c) {
                target.parents.push(name.clone());
            } else {
                warnings.push(format!(
                    "missing else block {:?} referenced by [{}]",
                    else_c, name
                ));
            }
        }
    }

    let decompressed_len = decompressed.len();
    Ok(DecodedReport {
        version,
        payload_len,
        raw_len,
        decompressed_len,
        inflate_method: "raw DEFLATE (LibDeflate DecompressDeflate)".to_string(),
        repair_mode,
        block_order,
        blocks,
        comments,
        editor,
        raw_decompressed: decompressed,
        warnings,
    })
}

impl DecodedReport {
    /// Renders the tree view matching python.py / MonikaOS_DecodedVS.txt
    pub fn render_tree(&self) -> String {
        let mut lines = Vec::new();
        let roots: Vec<&String> = self
            .block_order
            .iter()
            .filter(|name| {
                self.blocks
                    .get(*name)
                    .map(|b| b.parents.is_empty())
                    .unwrap_or(false)
            })
            .collect();

        if !self.blocks.is_empty() && roots.is_empty() {
            lines.push("(no roots - every block has a parent; possible orphan cycle)".to_string());
        }

        fn render_node(
            name: &str,
            depth: usize,
            seen: &mut Vec<String>,
            blocks: &HashMap<String, DecodedBlock>,
            lines: &mut Vec<String>,
        ) {
            let ind = "    ".repeat(depth);
            let b = match blocks.get(name) {
                Some(b) => b,
                None => {
                    lines.push(format!("{}[?] {} (missing block)", ind, name));
                    return;
                }
            };

            let tag = if depth == 0 {
                if b.block_type == "DefineFunction" {
                    "   (function definition)".to_string()
                } else {
                    "   (root - runs at startup)".to_string()
                }
            } else if b.parents.len() > 1 {
                let plist: Vec<String> = b.parents.iter().map(|p| format!("[{}]", p)).collect();
                format!("   (parents: {})", plist.join(", "))
            } else {
                String::new()
            };

            lines.push(format!("{}[{}] {}{}", ind, b.name, b.block_type, tag));

            for inp in &b.inputs {
                lines.push(format!(
                    "{}    {}",
                    ind,
                    pretty_wire_refs(&fmt_input(&inp.iname, inp))
                ));
            }

            for ln in fmt_output_lines(&b.outputs) {
                lines.push(format!("{}    {}", ind, pretty_wire_refs(&ln)));
            }

            for child in &b.children {
                if child.is_empty() {
                    continue;
                }
                if seen.contains(child) {
                    lines.push(format!("{}    [!] cycle back to [{}]", ind, child));
                    continue;
                }
                seen.push(child.clone());
                render_node(child, depth + 1, seen, blocks, lines);
                seen.pop();
            }

            if let Some(ref ec) = b.else_child {
                if seen.contains(ec) {
                    lines.push(format!("{}    (else:) [!] cycle back to [{}]", ind, ec));
                } else {
                    lines.push(format!("{}    (else:)", ind));
                    seen.push(ec.clone());
                    render_node(ec, depth + 1, seen, blocks, lines);
                    seen.pop();
                }
            }
        }

        for &root in &roots {
            let mut seen = vec![root.clone()];
            render_node(root, 0, &mut seen, &self.blocks, &mut lines);
        }

        lines.join("\n")
    }

    /// Renders the flat view matching python.py / MonikaOS_DecodedVS.txt
    pub fn render_flat(&self) -> String {
        let mut lines = Vec::new();
        for name in &self.block_order {
            let b = &self.blocks[name];
            lines.push(format!("[{}] {}", b.name, b.block_type));
            if b.parents.is_empty() {
                lines.push("    parents: (root)".to_string());
            } else {
                let plist: Vec<String> = b.parents.iter().map(|p| format!("[{}]", p)).collect();
                lines.push(format!("    parents: {}", plist.join(", ")));
            }
            for inp in &b.inputs {
                lines.push(format!("    {}", fmt_input(&inp.iname, inp)));
            }
            for (oname, var) in &b.outputs {
                lines.push(format!("    out {} -> ${}", oname, var));
            }
            let kids: Vec<String> = b
                .children
                .iter()
                .filter(|c| !c.is_empty())
                .map(|c| format!("[{}]", c))
                .collect();
            if !kids.is_empty() {
                lines.push(format!("    children: {}", kids.join(", ")));
            }
            if let Some(ref ec) = b.else_child {
                lines.push(format!("    else: [{}]", ec));
            }
            lines.push(String::new());
        }
        lines.join("\n")
    }

    /// Renders comments matching python.py / MonikaOS_DecodedVS.txt
    pub fn render_comments(&self) -> String {
        if self.comments.is_empty() {
            return "(none)".to_string();
        }
        let lines: Vec<String> = self
            .comments
            .iter()
            .map(|c| {
                format!(
                    "COMMENT: \"{}\"  (at {})",
                    c.text.as_deref().unwrap_or(""),
                    c.position.as_deref().unwrap_or("0,0")
                )
            })
            .collect();
        lines.join("\n")
    }

    /// Builds the full decoded report matching MonikaOS_DecodedVS.txt
    pub fn build_report(&self, label: &str) -> String {
        let mut lines = Vec::new();
        lines.push("=".repeat(70));
        lines.push(format!("SOURCE: {}", label));
        lines.push("=".repeat(70));
        lines.push(format!(
            "decoded via: v{} VisualSource | payload {} chars | deflate {} -> {} bytes ({}) | repair: {}",
            self.version,
            self.payload_len,
            self.raw_len,
            self.decompressed_len,
            self.inflate_method,
            self.repair_mode
        ));
        lines.push(format!(
            "blocks: {} | comments: {}",
            self.blocks.len(),
            self.comments.len()
        ));

        let mut hist: HashMap<&str, usize> = HashMap::new();
        for b in self.blocks.values() {
            *hist.entry(&b.block_type).or_insert(0) += 1;
        }

        let mut sorted_hist: Vec<(&str, usize)> = hist.into_iter().collect();
        // Sort by count descending, then name ascending
        sorted_hist.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

        if !sorted_hist.is_empty() {
            let hist_str: Vec<String> = sorted_hist
                .iter()
                .map(|(t, c)| format!("{} x{}", t, c))
                .collect();
            lines.push(format!("block types: {}", hist_str.join(", ")));
        }

        if !self.warnings.is_empty() {
            lines.push("warnings:".to_string());
            for w in &self.warnings {
                lines.push(format!("  - {}", w));
            }
        }

        lines.push(String::new());
        lines.push("[reading guide] numbers are stored as hex: \"3c\" = 60, \"a.1f\" = 10.031.".to_string());
        lines.push("$name = variable reference. Roots run at startup; DefineFunction roots are".to_string());
        lines.push("function definitions callable via ExecuteBlock. Inputs without an explicit".to_string());
        lines.push("type are annotated heuristically (their real types live in ProgrammingBlocksData).".to_string());
        lines.push(String::new());
        lines.push("--------------- BLOCK TREE ---------------".to_string());
        lines.push(self.render_tree());
        lines.push(String::new());
        lines.push("--------------- ALL BLOCKS (flat) ---------------".to_string());
        lines.push(self.render_flat());
        lines.push("--------------- COMMENTS ---------------".to_string());
        lines.push(self.render_comments());
        lines.push(String::new());
        lines.push("=============== RAW DECOMPRESSED STREAM ===============".to_string());
        lines.push(self.render_raw(true));
        lines.push(self.render_hexdump());

        if self.editor.camera_position.is_some() || self.editor.camera_zoom.is_some() {
            lines.push(String::new());
            lines.push(format!(
                "editor: zoom={} camera={}",
                self.editor.camera_zoom.as_deref().unwrap_or(""),
                self.editor.camera_position.as_deref().unwrap_or("")
            ));
        }

        lines.join("\n")
    }

    /// Converts this decoded report into a library VisualSource struct
    pub fn to_visual_source(&self) -> VisualSource {
        let editor = Editor {
            camera_position: VSVector2::new(),
            camera_zoom: 0.0.into(),
        };

        let mut vs_blocks = HashMap::new();
        for (name, b) in &self.blocks {
            let mut inputs = Vec::new();
            for inp in &b.inputs {
                let vis = if inp.use_var {
                    BlockInputVisibility::Variable
                } else if inp.vtype.is_some() {
                    BlockInputVisibility::Explicit
                } else {
                    BlockInputVisibility::Implicit
                };
                let val_str = if inp.use_var {
                    &inp.var
                } else {
                    inp.value.as_deref().unwrap_or("")
                };
                inputs.push(BlockInput::from((
                    VSString::from(inp.iname.clone()),
                    vis,
                    VSString::from(val_str.to_string()),
                )));
            }

            let mut outputs = Vec::new();
            for (oname, var) in &b.outputs {
                let out_val = BlockOutputValueType::String(VSString::from(var.clone()));
                if let Ok(bo) = BlockOutput::new(oname.clone(), out_val) {
                    outputs.push(bo);
                }
            }

            let block = Block {
                internal: VSString::from(b.block_type.clone()),
                name: VSString::from(name.clone()),
                visual_position: VSVector2::new(),
                child_blocks: b.children.iter().map(|c| VSString::from(c.clone())).collect(),
                else_child_block: b.else_child.as_ref().map(|ec| VSString::from(ec.clone())),
                inputs,
                outputs,
                parent_blocks: b.parents.clone(),
            };

            vs_blocks.insert(name.clone(), block);
        }

        VisualSource {
            version: self.version,
            editor,
            blocks: vs_blocks,
            comments: Vec::new(),
            unique_id_service: UniqueIdService::new(),
        }
    }
}

// ============ RAW GROUND TRUTH - walks raw_decompressed directly ============
// Nothing here re-parses. What you see is the byte stream.
// (A value containing \x1a/\x1b is unrepresentable in the format itself, so
// tokens are always treated as structural - same assumption RetroStudio makes.)

fn safe_str(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len());
    for &c in b {
        match c {
            0x20..=0x7e => s.push(c as char),
            _ => s.push_str(&format!("\\x{:02x}", c)),
        }
    }
    s
}

impl DecodedReport {
    /// One line per structural token. with_offsets=true prefixes byte offsets
    /// (they align with render_hexdump - quotable in bug reports).
    /// Use with_offsets=false when feeding diff_raw.
    pub fn render_raw(&self, with_offsets: bool) -> String {
        let b = self.raw_decompressed.as_slice();
        let mut out = String::new();
        let mut i = 0usize;
        while i < b.len() {
            let tag = if with_offsets { format!("[{:06}] ", i) } else { String::new() };
            if i + 1 < b.len() && b[i] == 0x1a && b[i + 1] == 0x1a {
                let start = i + 2;
                let mut end = start;
                while end < b.len() && b[end] != 0x1a { end += 1; }
                out.push_str(&format!("{}[REC] {}\n", tag, safe_str(&b[start..end])));
                i = end;
            } else if b[i] == 0x1a {
                let start = i + 1;
                let mut end = start;
                while end < b.len() && b[end] != 0x1a && b[end] != 0x1b { end += 1; }
                out.push_str(&format!("{}  [FIELD] {}\n", tag, safe_str(&b[start..end])));
                i = end;
            } else if b[i] == 0x1b {
                let start = i + 1;
                let mut end = start;
                while end < b.len() && b[end] != 0x1a && b[end] != 0x1b { end += 1; }
                out.push_str(&format!("{}    [VAL] {}\n", tag, safe_str(&b[start..end])));
                i = end;
            } else {
                out.push_str(&format!("{}  <STRAY 0x{:02X}>\n", tag, b[i]));
                i += 1;
            }
        }
        out
    }

    pub fn render_hexdump(&self) -> String {
        let b = self.raw_decompressed.as_slice();
        let mut out = format!("=== HEXDUMP ({} bytes) ===\n", b.len());
        for (row, chunk) in b.chunks(16).enumerate() {
            let mut hex = String::new();
            for &c in chunk { hex.push_str(&format!("{:02X} ", c)); }
            while hex.len() < 48 { hex.push(' '); }
            let ascii: String = chunk.iter()
                .map(|&c| if (0x20..=0x7e).contains(&c) { c as char } else { '.' })
                .collect();
            out.push_str(&format!("{:06X}  {} |{}|\n", row * 16, hex, ascii));
        }
        out
    }
}

/// LCS diff of the two streams' structural tokens. This IS the bug report:
/// every '-' line exists only in `la`, every '+' line only in `lb`.
pub fn diff_raw(a: &DecodedReport, la: &str, b: &DecodedReport, lb: &str) -> String {
    let a_raw = a.render_raw(false);
    let b_raw = b.render_raw(false);
    let av: Vec<&str> = a_raw.lines().collect();
    let bv: Vec<&str> = b_raw.lines().collect();
    let mut out = format!(
        "=== RAW DIFF | {} ({} tokens) vs {} ({} tokens) | '-' only in {}, '+' only in {} ===\n",
        la, av.len(), lb, bv.len(), la, lb
    );

    if av.len().max(bv.len()) > 3000 {
        let mut k = 0;
        while k < av.len().min(bv.len()) && av[k] == bv[k] { k += 1; }
        out.push_str("(too large for full diff - first divergence, context below)\n");
        let lo = k.saturating_sub(8);
        for (side, lines) in [("-", &av), ("+", &bv)] {
            for i in lo..(k + 8).min(lines.len()) {
                let mark = if i == k { ">>" } else { "  " };
                out.push_str(&format!("{}{} {}\n", mark, side, lines[i]));
            }
        }
        return out;
    }

    let (n, m) = (av.len(), bv.len());
    let mut dp = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if av[i] == bv[j] { dp[i + 1][j + 1] + 1 }
                       else { dp[i + 1][j].max(dp[i][j + 1]) };
        }
    }
    let (mut i, mut j) = (0usize, 0usize);
    while i < n && j < m {
        if av[i] == bv[j] {
            out.push_str(&format!("    {}\n", av[i]));
            i += 1; j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            out.push_str(&format!("-   {}\n", av[i])); i += 1;
        } else {
            out.push_str(&format!("+   {}\n", bv[j])); j += 1;
        }
    }
    for l in &av[i..] { out.push_str(&format!("-   {}\n", l)); }
    for l in &bv[j..] { out.push_str(&format!("+   {}\n", l)); }
    out
}

impl DecodedReport {
    /// Re-encodes this report's raw record bytes with OUR compressor/base93/header
    /// and compares against the payload text we decoded from. Exonerates or
    /// convicts the transport layer (deflate wrapper, Base93 alphabet, header).
    pub fn verify_outer_layer(&self, original_payload_text: &str) -> String {
        let comp = match deflate::compress(&self.raw_decompressed) {
            Ok(c) => c,
            Err(e) => return format!("FAIL: our compressor rejected the records: {}", e),
        };
        let ours = format!("\x1a{:016}\x1b{}", self.version, base93::encode(&comp));
        let orig: String = original_payload_text.chars()
            .filter(|c| !c.is_whitespace()).collect();

        if ours == orig {
            return "PASS: our deflate+base93+header reproduces the original payload \
                    byte-for-byte. Outer layer exonerated.".into();
        }

        let first = ours.bytes().zip(orig.bytes())
            .position(|(x, y)| x != y)
            .unwrap_or(ours.len().min(orig.len()));
        if first <= 17 {
            return format!("DIFF in HEADER at byte {} (ours {:?}, orig {:?})",
                first, ours.as_bytes().get(first), orig.as_bytes().get(first));
        }

        let (ver, orig_body) = match extract_payload(&orig) {
            Ok(v) => v,
            Err(e) => return format!("DIFF: can't re-extract original body: {}", e),
        };
        if ver != self.version {
            return format!("DIFF: version ours={} orig={}", self.version, ver);
        }
        match base93::decode(&orig_body) {
            Ok(orig_comp) if orig_comp == comp => {
                "DIFF: base93 strings differ but decode to IDENTICAL compressed bytes \
                 -> your Base93 alphabet/emitter differs from the original's.".into()
            }
            Ok(orig_comp) => {
                let fb = orig_comp.iter().zip(comp.iter()).position(|(x, y)| x != y);
                let wrap = |c: &[u8]| if c.first() == Some(&0x78) {
                    " [starts 0x78 = zlib wrapper; raw deflate expected!]".to_string()
                } else { String::new() };
                format!("DIFF: compressed bytes diverge at {:?} | orig {}B{} vs ours {}B{}",
                    fb, orig_comp.len(), wrap(&orig_comp), comp.len(), wrap(&comp))
            }
            Err(e) => format!("DIFF: original body fails our base93 decode ({}) -> alphabet mismatch.", e),
        }
    }
}
