//! Luau -> VisualSource V4 transpiler.
//!
//! Pipeline: lexer -> parser (restricted AST) -> block emitter -> report.
//! The emitted forest is converted into a [`DecodedReport`] so it reuses the
//! Phase 1 renderer and the Phase 2 lossless encoder unchanged.

pub mod ast;
pub mod detranspile;
pub mod emit;
pub mod lexer;
pub mod parser;
pub mod rs_support;

use std::collections::HashMap;

use crate::decoder::{DecodedBlock, DecodedComment, DecodedEditor, DecodedInput, DecodedReport};
use crate::hex::Hex;
use emit::{EmittedBlock, InputVal};
use lexer::CapturedComment;

pub use emit::Note;

#[derive(Debug)]
pub struct TranspileOutput {
    pub report: DecodedReport,
    pub notes: Vec<Note>,
    pub budget: String,
    pub gap_count: usize,
}

/// Transpiles Luau source into a VisualSource report.
pub fn transpile(source: &str, label: &str, optimize: bool) -> Result<TranspileOutput, String> {
    let (stmts, src_comments, stmt_starts) = parser::parse_with_comments(source)
        .map_err(|e| format!("parse error: {}", e))?;

    let mut t = emit::Transpiler::new(optimize);
    let roots = t.transpile(&stmts);
    let folded = t.folded;
    let cse_hits = t.cse_hits;
    let spans = t.stmt_spans();
    let (blocks, notes) = t.finish();

    let anchored = anchor_comments(&src_comments, &stmt_starts);
    let report = build_report(blocks, roots, label, source, &stmt_starts, &spans, &anchored);
    let gap_count = notes.iter().filter(|n| n.kind == "GAP").count();
    let budget = format_budget(
        source.lines().count(),
        report.blocks.len(),
        folded,
        cse_hits,
        gap_count,
        &report,
    );

    Ok(TranspileOutput { report, notes, budget, gap_count })
}

fn format_budget(
    lines: usize,
    blocks: usize,
    folded: usize,
    cse: usize,
    gaps: usize,
    report: &DecodedReport,
) -> String {
    // per-function subtree sizes (DefineFunction roots)
    let mut per_function: Vec<(String, usize)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for name in &report.block_order {
        if let Some(b) = report.blocks.get(name) {
            if b.block_type == "DefineFunction" && seen.insert(name.clone()) {
                let size = subtree_size(&report.blocks, name, &mut seen);
                // name the function by its Function output binding
                let fname = b
                    .outputs
                    .iter()
                    .find(|(n, _)| n == "Function")
                    .map(|(_, b)| b.clone())
                    .unwrap_or_else(|| name.clone());
                per_function.push((fname, size));
            }
        }
    }
    let mut text = format!(
        "blocks_out: {} (luau lines: {}) | folded: {} | cse: {} | gap: {}",
        blocks, lines, folded, cse, gaps
    );
    // top-level (root) flow size for context
    let mut all_seen = std::collections::HashSet::new();
    for name in &report.block_order {
        if let Some(b) = report.blocks.get(name) {
            if b.block_type == "DefineFunction" && all_seen.insert(name.clone()) {
                subtree_size(&report.blocks, name, &mut all_seen);
            }
        }
    }
    let main_size = report
        .block_order
        .iter()
        .filter(|n| report.blocks.get(*n).map(|b| b.parents.is_empty()).unwrap_or(false))
        .count();
    text.push_str(&format!(" | main-flow roots: {}", main_size));
    per_function.sort_by(|a, b| b.1.cmp(&a.1));
    if !per_function.is_empty() {
        text.push_str(" | functions: ");
        let parts: Vec<String> = per_function
            .iter()
            .map(|(n, c)| {
                let flag = if *c > 50 { " (!>50 manual review)" } else { "" };
                format!("{}={}{}", if n.is_empty() { "?" } else { n }, c, flag)
            })
            .collect();
        text.push_str(&parts.join(", "));
    }
    text
}

fn subtree_size(
    blocks: &HashMap<String, crate::decoder::DecodedBlock>,
    name: &str,
    seen: &mut std::collections::HashSet<String>,
) -> usize {
    let mut total = 1;
    if let Some(b) = blocks.get(name) {
        let mut kids = b.children.clone();
        if let Some(ec) = &b.else_child {
            kids.push(ec.clone());
        }
        for k in kids {
            if k.is_empty() {
                continue;
            }
            if seen.insert(k.clone()) {
                total += subtree_size(blocks, &k, seen);
            }
        }
    }
    total
}

fn hexs(v: f64) -> String {
    Hex::to_hex_string(v)
}

fn input_to_decoded(name: &str, iv: &InputVal) -> DecodedInput {
    match iv {
        InputVal::Literal { value, vtype } => DecodedInput {
            iname: name.to_string(),
            use_var: false,
            var: String::new(),
            vtype: Some(vtype.clone()),
            value: Some(value.clone()),
        },
        InputVal::Implicit(v) => DecodedInput {
            iname: name.to_string(),
            use_var: false,
            var: String::new(),
            vtype: None,
            value: Some(v.clone()),
        },
        InputVal::Var(b) => DecodedInput {
            iname: name.to_string(),
            use_var: true,
            var: b.clone(),
            vtype: None,
            value: None,
        },
    }
}

fn build_report(
    blocks: Vec<EmittedBlock>,
    _roots: Vec<String>,
    label: &str,
    source: &str,
    stmt_starts: &[usize],
    spans: &[(usize, usize)],
    anchored: &[AnchoredComment],
) -> DecodedReport {
    let mut block_order = Vec::new();
    let mut map = HashMap::new();
    let mut comment_records: Vec<DecodedComment> = Vec::new();
    // [v24] per-statement source line ranges for the "Line N:" labels
    let stmt_lines = stmt_line_ranges(source, stmt_starts);
    let source_lines: Vec<&str> = source.lines().collect();

    // layout: v29 - measured from the user's handmade files (function_example
    // v0.1 + the 668-block script), decoded with the CORRECT RS hex rule
    // (fraction = hex-integer / 1000 thousandths; the v26-v28 analysis read
    // fractions as hex mantissa bits, which scaled every ground truth wrong):
    // - y grows DOWNWARD, chain pitch LEVEL = 0.75 per generation; 542 of the
    //   564 parent->child edges in the 668-block handmade file are exactly
    //   (dx=0, dy=0.75): a single child sits DIRECTLY below its parent, so
    //   linear chains read top to bottom like the source
    // - multiple children form a row one level below; the parent is centered
    //   over the children's OWN positions; an ELSE branch sits on the same
    //   row right of the children (user ground truth, T9 image 2)
    // - top-level statements are SIBLINGS on one row (ROOT fans out to them;
    //   they are not chained to each other), flowing left-to-right in source
    //   order; sibling columns are separated by block width + 0.125 + the
    //   statement's label width (function_example: 2.875 = 1.0 + 0.125 + 1.75
    //   and 3.875 = 1.0 + 0.125 + 2.75 exactly)
    // - the ROOT Comment2 container sits on its own row 1.25 above the
    //   statement row, x over the middle statement (function_example: ROOT x
    //   = middle child's x)
    // - comment boxes: one text line = 0.5 tall, ~0.040 units per char
    //   (44 chars in a 1.75-wide box in function_example), stacked beside
    //   their statement's column and stretched over its chain (v25)
    // - positions assigned PRE-ORDER; records pushed POST-ORDER (RS order)
    let mut order: Vec<String> = Vec::new();
    let mut pos: HashMap<String, (f64, f64)> = HashMap::new();
    {
        let index: HashMap<&str, &EmittedBlock> =
            blocks.iter().map(|b| (b.name.as_str(), b)).collect();
        let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();

        // [v29] all geometry in RS units (no column abstraction)
        const LEVEL: f64 = 0.75; // chain pitch (y per generation)
        const BLOCK_W: f64 = 1.0; // block visual width (handmade: ~1.0)
        const COL_GAP: f64 = 0.125; // gap between sibling columns (handmade)

        // subtree width in units. Chains are vertical (dx=0), so width comes
        // only from branching rows and else columns.
        fn subtree_w(
            name: &str,
            index: &HashMap<&str, &EmittedBlock>,
        ) -> f64 {
            fn go(
                name: &str,
                index: &HashMap<&str, &EmittedBlock>,
                seen: &mut std::collections::HashSet<String>,
            ) -> f64 {
                if !seen.insert(name.to_string()) {
                    return 0.0;
                }
                match index.get(name) {
                    None => BLOCK_W,
                    Some(b) => {
                        let kids: Vec<&String> = b
                            .children
                            .iter()
                            .filter(|c| !c.is_empty())
                            .collect();
                        let mut w = 0.0f64;
                        for (i, c) in kids.iter().enumerate() {
                            if i > 0 {
                                w += COL_GAP;
                            }
                            w += go(c, index, seen);
                        }
                        if kids.is_empty() {
                            w = BLOCK_W;
                        }
                        if let Some(ec) = &b.else_child {
                            if !ec.is_empty() {
                                let ew = go(ec, index, seen);
                                if ew > 0.0 {
                                    w += COL_GAP + ew;
                                }
                            }
                        }
                        w
                    }
                }
            }
            go(name, index, &mut std::collections::HashSet::new())
        }

        // place a subtree whose left edge is `x_left` at row `y`
        fn place(
            name: &str,
            x_left: f64,
            y: f64,
            index: &HashMap<&str, &EmittedBlock>,
            visited: &mut std::collections::HashSet<String>,
            pos: &mut HashMap<String, (f64, f64)>,
            order: &mut Vec<String>,
        ) {
            if !visited.insert(name.to_string()) {
                return;
            }
            let (kids, els) = match index.get(name) {
                Some(b) => (b.children.clone(), b.else_child.clone()),
                None => (Vec::new(), None),
            };
            let mut col = x_left;
            let mut centers: Vec<f64> = Vec::new();
            let mut placed_kids = 0usize;
            for c in &kids {
                if c.is_empty() {
                    continue;
                }
                if placed_kids > 0 {
                    col += COL_GAP;
                }
                place(c, col, y + LEVEL, index, visited, pos, order);
                // [v28->v29] center over the children's OWN positions - the
                // handmade ground truth (dx=0 for a single child)
                if let Some((cx, _)) = pos.get(c.as_str()) {
                    centers.push(*cx);
                }
                col += subtree_w(c, index);
                placed_kids += 1;
            }
            if centers.is_empty() {
                pos.insert(name.to_string(), (x_left, y));
            } else {
                let cx = (centers[0] + centers[centers.len() - 1]) / 2.0;
                pos.insert(name.to_string(), (cx, y));
            }
            if let Some(ec) = &els {
                if !ec.is_empty() {
                    if placed_kids > 0 {
                        col += COL_GAP;
                    }
                    // same row as the parent, immediately right of its children row
                    place(ec, col, y, index, visited, pos, order);
                }
            }
            // RS convention: child records appear BEFORE their
            // parents in the stream (RetroStudio ground truth).
            order.push(name.to_string());
        }

        let mut is_child: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for b in &blocks {
            for c in &b.children {
                is_child.insert(c.as_str());
            }
            if let Some(ec) = &b.else_child {
                is_child.insert(ec.as_str());
            }
        }
        let roots: Vec<String> = blocks
            .iter()
            .filter(|b| !is_child.contains(b.name.as_str()))
            .map(|b| b.name.clone())
            .collect();

        // [v29] label specs first: each statement's widest box reserves room
        // in the row (COL_GAP + box width between statement columns)
        const ROOT_GAP: f64 = 1.25; // ROOT row above the statement row
        let specs = all_statement_specs(anchored, spans.len(), &stmt_lines, &source_lines);
        if spans.is_empty() {
            if roots.len() == 1 {
                let r = &roots[0];
                let total_w = subtree_w(r, &index);
                place(
                    r,
                    -total_w / 2.0,
                    0.0,
                    &index,
                    &mut visited,
                    &mut pos,
                    &mut order,
                );
            } else {
                let total_w: f64 = roots.iter().map(|r| subtree_w(r, &index)).sum();
                let mut col = -total_w / 2.0;
                for r in &roots {
                    place(r, col, 0.0, &index, &mut visited, &mut pos, &mut order);
                    col += subtree_w(r, &index) + COL_GAP;
                }
            }
        } else {
            // [v29] statement row - the handmade pattern (function_example
            // v0.1): the top-level statements are SIBLINGS on one row, read
            // left-to-right in source order; ROOT fans out to them. Between
            // two statement columns: block width + 0.125 + the statement's
            // widest label box (handmade: 2.875 = 1.0 + 0.125 + 1.75). No
            // uniform gutters, no bands, no forced stacking.
            let mut row: Vec<(String, f64)> = Vec::new();
            for (i, s) in spans.iter().enumerate() {
                if s.0 < s.1 && s.0 < blocks.len() {
                    let n = blocks[s.0].name.clone();
                    let gw = specs[i].iter().map(|s| s.2).fold(0.0f64, f64::max);
                    match row.iter_mut().find(|(n2, _)| n2 == &n) {
                        Some((_, g)) => *g = g.max(gw),
                        None => row.push((n, gw)),
                    }
                }
            }
            let mut row_xs: Vec<f64> = Vec::new();
            let mut cursor = 0.0f64;
            for (name, gap) in &row {
                place(name, cursor, 0.0, &index, &mut visited, &mut pos, &mut order);
                if let Some((x, _)) = pos.get(name.as_str()) {
                    row_xs.push(*x);
                }
                // +0.003: positions are written as hex thousandths; the
                // rounding must never eat into the next column
                cursor += subtree_w(name, &index) + COL_GAP + gap + 0.003;
            }
            // the ROOT Comment2 container sits on its OWN row above the
            // statement row (handmade: ROOT y = row y - 1.25, x = the middle
            // statement's x) - v28 parked it ON the first statement, which
            // stacked two blocks on the same spot
            let row_names: std::collections::HashSet<&str> =
                row.iter().map(|(n, _)| n.as_str()).collect();
            let container = roots.iter().find(|r| {
                !row_names.contains(r.as_str())
                    && index
                        .get(r.as_str())
                        .map(|b| b.children.iter().any(|c| row_names.contains(c.as_str())))
                        .unwrap_or(false)
            });
            if let Some(rc) = container {
                let rx = if row_xs.is_empty() {
                    0.0
                } else if row_xs.len() % 2 == 1 {
                    row_xs[row_xs.len() / 2]
                } else {
                    (row_xs[row_xs.len() / 2 - 1] + row_xs[row_xs.len() / 2]) / 2.0
                };
                visited.insert(rc.clone());
                pos.insert(rc.clone(), (rx, -ROOT_GAP));
                order.push(rc.clone());
            }
        }

        // leftovers (cycles, stray roots): park them to the right
        {
            let right = pos
                .values()
                .map(|p| p.0)
                .fold(0.0f64, f64::max)
                + 3.0;
            let mut ocol = right;
            for b in &blocks {
                if !visited.contains(&b.name) {
                    place(&b.name, ocol, 0.0, &index, &mut visited, &mut pos, &mut order);
                    ocol += subtree_w(&b.name, &index) + 2.0;
                }
            }
        }

        // layout: RS-style tidy rows, centered on the camera start (0,0)
        // center the whole canvas on the camera start (0,0)
        {
            let minx = pos.values().map(|p| p.0).fold(f64::INFINITY, f64::min);
            let maxx = pos
                .values()
                .map(|p| p.0 + BLOCK_W)
                .fold(f64::NEG_INFINITY, f64::max);
            let miny = pos.values().map(|p| p.1).fold(f64::INFINITY, f64::min);
            let maxy = pos
                .values()
                .map(|p| p.1 + LEVEL)
                .fold(f64::NEG_INFINITY, f64::max);
            let dx = -(minx + maxx) / 2.0;
            let dy = -(miny + maxy) / 2.0;
            for (x, y) in pos.values_mut() {
                *x += dx;
                *y += dy;
            }
        }

        for name in &order {
            if let Some(b) = index.get(name.as_str()) {
                let (x, y) = pos.get(name).copied().unwrap_or((0.0, 0.0));
                let db = DecodedBlock {
                    name: b.name.clone(),
                    block_type: b.btype.clone(),
                    visual_position: Some(format!("{},{}", hexs(x), hexs(y))),
                    children: b.children.clone(),
                    else_child: b.else_child.clone(),
                    parents: Vec::new(),
                    inputs: b
                        .inputs
                        .iter()
                        .map(|(n, iv)| input_to_decoded(n, iv))
                        .collect(),
                    outputs: b.outputs.clone(),
                };
                block_order.push(b.name.clone());
                map.insert(b.name.clone(), db);
            }
        }

        // [v23] editor Comment records: one per source comment, placed next
        // to the block column of the statement it annotates
        // [v24] plus a gray "Line N:" label for every unannotated statement
        // [v29] specs were built before the block layout (the row spacing
        // reserves each statement's label width)
        comment_records = place_comments(&specs, spans, &blocks, &pos);
    }

    // reconstruct parents exactly like the decoder does (children/else links)
    let mut blocks_map = map;
    for name in &block_order {
        let children = blocks_map
            .get(name)
            .map(|b| {
                let mut v = b.children.clone();
                if let Some(ec) = &b.else_child {
                    v.push(ec.clone());
                }
                v
            })
            .unwrap_or_default();
        for child in children {
            if child.is_empty() {
                continue;
            }
            if let Some(target) = blocks_map.get_mut(&child) {
                target.parents.push(name.clone());
            }
        }
    }

    DecodedReport {
        version: 4,
        payload_len: 0,
        raw_len: 0,
        decompressed_len: 0,
        inflate_method: String::new(),
        repair_mode: format!("transpiled: {}", label),
        block_order,
        blocks: blocks_map,
        comments: comment_records,
        editor: DecodedEditor {
            camera_position: Some("0,0".into()),
            camera_zoom: Some("0.45".into()),
        },
        raw_decompressed: Vec::new(),
        warnings: Vec::new(),
    }
}

/// [v23] A source comment processed for emission: attached to the top-level
/// statement index `stmt`, text cleaned and joined, color resolved (explicit
/// `#RRGGBB` prefix or the rotated user palette).
/// [v24] also carries the comment's own source line range so its box gets a
/// "Line N:" reference prefix.
#[derive(Debug, Clone)]
pub struct AnchoredComment {
    pub stmt: usize,
    pub text: String,
    pub color: (u32, u32, u32),
    pub line_start: usize,
    pub line_end: usize,
}

const COMMENT_PALETTE: [(u32, u32, u32); 6] = [
    (0xC4, 0x28, 0x1C), // red     (user's own example)
    (0xF5, 0xCD, 0x30), // yellow  (user's own example)
    (0x4B, 0x97, 0x4B), // green   (user's own example)
    (0xCD, 0x62, 0x98), // pink    (user's own example)
    (0x30, 0x62, 0xF5), // blue
    (0xB0, 0x62, 0xF5), // purple
];

/// Joins runs of consecutive `--` line comments into one multi-line comment,
/// attaches every comment to the first top-level statement that starts AFTER
/// it (trailing comments go to the last statement), and resolves colors.
pub fn anchor_comments(
    src: &[CapturedComment],
    stmt_starts: &[usize],
) -> Vec<AnchoredComment> {
    if stmt_starts.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<AnchoredComment> = Vec::new();
    let mut rot: usize = 0;

    let mut i = 0usize;
    while i < src.len() {
        let run_start = src[i].line_start;
        let mut line_end = src[i].line_end;
        // merge consecutive single-line comments into one box
        let mut parts: Vec<String> = Vec::new();
        if src[i].long {
            parts.push(clean_long_comment(&src[i].text));
        } else {
            parts.push(src[i].text.trim().to_string());
            let mut j = i + 1;
            while j < src.len()
                && !src[j].long
                && src[j].line_start == line_end + 1
            {
                parts.push(src[j].text.trim().to_string());
                line_end = src[j].line_end;
                j += 1;
            }
            i = j - 1;
        }
        let joined = parts
            .into_iter()
            .filter(|p| !p.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        if !joined.is_empty() {
            let (text, color) = split_color_prefix(&joined).unwrap_or_else(|| {
                let c = COMMENT_PALETTE[rot % COMMENT_PALETTE.len()];
                rot += 1;
                (joined, c)
            });
            // first statement that starts strictly after the comment
            let stmt = stmt_starts
                .iter()
                .position(|&l| l > line_end)
                .unwrap_or(stmt_starts.len() - 1);
            out.push(AnchoredComment {
                stmt,
                text,
                color,
                line_start: run_start,
                line_end,
            });
        }
        i += 1;
    }
    out
}

/// Trims a long-comment body: strip blank leading/trailing lines and
/// trailing spaces on each line.
fn clean_long_comment(text: &str) -> String {
    let lines: Vec<String> = text
        .lines()
        .map(|l| l.trim_end().to_string())
        .collect();
    let first = lines.iter().position(|l| !l.trim().is_empty());
    let last = lines.iter().rposition(|l| !l.trim().is_empty());
    match (first, last) {
        (Some(a), Some(b)) => lines[a..=b].join("\n"),
        _ => String::new(),
    }
}

/// If the text starts with `#RRGGBB` followed by whitespace or a newline,
/// returns (rest_of_text, color). Otherwise None.
fn split_color_prefix(text: &str) -> Option<(String, (u32, u32, u32))> {
    let t = text.trim_start();
    let bytes = t.as_bytes();
    if bytes.first() != Some(&b'#') || bytes.len() < 7 {
        return None;
    }
    let hex6 = &t[1..7];
    if !hex6.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let boundary = bytes[7].is_ascii_whitespace();
    if !boundary {
        return None;
    }
    let v = u32::from_str_radix(hex6, 16).ok()?;
    let rest = t[7..].trim_start().to_string();
    Some((rest, ((v >> 16) & 0xFF, (v >> 8) & 0xFF, v & 0xFF)))
}

/// Lays out the editor Comment records (v24: one box per top-level
/// statement, each stating WHICH source line it represents).
///
/// Placement (v29, measured from the user's handmade function_example):
/// - the box sits immediately RIGHT of the statement's own column:
///   x = rightmost block x + 1.0 (block width) + 0.125, top y = the
///   statement's chain top - exactly where the user puts it
///   (their 'Line 1' box: right of the block, top-aligned, 0.5 tall)
/// - the block layout reserved COL_GAP + the box width between this
///   statement's column and the next one, so the box normally fits
///   without nudging; the nudge loop stays as a safety net
/// - the first box stretches so the statement's boxes together span the
///   chain's full height (v25; the user's handmade tall 'line 3' box)
/// - RetroStudio wraps text inside the box; one text line = 0.5 tall,
///   width ~0.040 per char (44 chars in the handmade 1.75-wide box)
/// - a collision nudge slides the box right in 0.125 steps; after 40
///   blocked steps it falls back BELOW the statement's subtree
///
/// Box content (v24, user request "comments should show the line they are"):
/// - statements WITHOUT a user comment get a gray auto label:
///     `Line 130: while n > 0 do` / `Lines 40-51: local function f(a)`
///     plus `(+K more lines)` when the statement spans extra lines
/// - statements WITH user comment(s) get those boxes, each prefixed with
///   the comment's own line reference; no auto label for them
fn place_comments(
    specs: &[Vec<(String, (u32, u32, u32), f64, f64)>],
    spans: &[(usize, usize)],
    blocks: &[EmittedBlock],
    pos: &HashMap<String, (f64, f64)>,
) -> Vec<DecodedComment> {
    const BLOCK_W: f64 = 1.0;
    const GAP: f64 = 0.125;
    // [v29] block visual height: the handmade tall label for a 3-block
    // chain is 2.0 = 2x0.75 pitch + 0.5 block height
    const FOOT_H: f64 = 0.5;
    const NUDGE_CAP: usize = 40;

    if pos.is_empty() || spans.is_empty() {
        return Vec::new();
    }

    // footprints of every block, for collision avoidance
    let mut occupied: Vec<(f64, f64, f64, f64)> = pos
        .values()
        .map(|(x, y)| (*x, *y, BLOCK_W, FOOT_H))
        .collect();

    let mut out = Vec::new();
    for (stmt, spec) in specs.iter().enumerate() {
        if spec.is_empty() {
            continue;
        }
        // span of this statement; fall back outward for empty spans
        // (user comments still anchor to a neighbor; auto labels skip)
        let mut span = spans.get(stmt).copied().unwrap_or((0, 0));
        if span.0 >= span.1 {
            let mut s = stmt;
            while s < spans.len() && span.0 >= span.1 {
                s += 1;
                span = spans.get(s).copied().unwrap_or(span);
            }
            if span.0 >= span.1 {
                s = stmt;
                while s > 0 && span.0 >= span.1 {
                    s -= 1;
                    span = spans.get(s).copied().unwrap_or(span);
                }
            }
        }
        if span.0 >= span.1 || span.0 >= blocks.len() {
            continue;
        }
        let entry = match pos.get(&blocks[span.0].name) {
            Some(p) => *p,
            None => continue,
        };
        // bottom of the statement's subtree, for the below-fallback
        let span_bottom = blocks[span.0..span.1]
            .iter()
            .filter_map(|b| pos.get(&b.name))
            .map(|p| p.1)
            .fold(entry.1, f64::max)
            + FOOT_H;
        // [v25] full vertical extent of the statement's block chain: the
        // statement's boxes must together span it (user: "resize it to the
        // entire block that represents that line" - a 1-unit text box next
        // to an 8-block print chain looked broken)
        let chain_top = blocks[span.0..span.1]
            .iter()
            .filter_map(|b| pos.get(&b.name))
            .map(|p| p.1)
            .fold(entry.1, f64::min);
        let chain_h = (span_bottom - chain_top).max(0.0);

        // size every box, then stretch the stack so the statement's boxes
        // together span the chain height (leftover goes to the first box),
        // stacking downward from the chain top
        let mut sized: Vec<(String, (u32, u32, u32), f64, f64)> = spec.clone();
        let text_total = sized.iter().map(|s| s.3).sum::<f64>()
            + GAP * sized.len().saturating_sub(1) as f64;
        if let Some(first) = sized.first_mut() {
            first.3 += (chain_h - text_total).max(0.0);
        }
        let mut y = chain_top;
        // stack/fallback slack: same hex-thousandths rounding guard
        const SLACK: f64 = 0.003;
        for (text, color, w, h) in sized {
            // candidate 1: right of the statement's own column, nudge right
            let cluster_right = blocks[span.0..span.1]
                .iter()
                .filter_map(|b| pos.get(&b.name))
                .map(|p| p.0)
                .fold(entry.0, f64::max)
                + BLOCK_W;
            let mut cx = cluster_right + GAP;
            let mut cy = y;
            let mut tries = 0;
            while tries < NUDGE_CAP && rect_hit(&occupied, cx, cy, w, h) {
                cx += GAP;
                tries += 1;
            }
            if tries == NUDGE_CAP {
                // candidate 2: below the statement's subtree, nudge down
                cx = entry.0;
                cy = span_bottom + GAP + SLACK;
                let mut down = 0;
                while down < 60 && rect_hit(&occupied, cx, cy, w, h) {
                    cy += GAP;
                    down += 1;
                }
            }
            occupied.push((cx, cy, w, h));
            out.push(DecodedComment {
                text: Some(text),
                position: Some(format!("{},{}", hexs(cx), hexs(cy))),
                size: Some(format!("{},{}", hexs(w), hexs(h))),
                color: Some(format!("{:X},{:X},{:X}", color.0, color.1, color.2)),
            });
            y += h + GAP + SLACK;
        }
    }
    out
}

/// [v29] The comment boxes for every top-level statement, built once so the
/// block layout can reserve each statement's label width before placing
/// blocks. Per statement: the anchored user comments (each prefixed with its
/// line reference), or one gray auto label naming the statement's source
/// line. (text, color, width, height)
fn all_statement_specs(
    anchored: &[AnchoredComment],
    n_stmts: usize,
    stmt_lines: &[(usize, usize)],
    source_lines: &[&str],
) -> Vec<Vec<(String, (u32, u32, u32), f64, f64)>> {
    const LABEL_COLOR: (u32, u32, u32) = (0x99, 0x99, 0x99);
    let mut out = Vec::new();
    let mut k = 0usize;
    for stmt in 0..n_stmts {
        let end = anchored[k..]
            .iter()
            .position(|a| a.stmt != stmt)
            .map(|p| k + p)
            .unwrap_or(anchored.len());
        let has_user = k < end;
        let mut spec = Vec::new();
        if has_user {
            for a in &anchored[k..end] {
                let prefix = line_ref(a.line_start, a.line_end);
                let text = format!("{}\n{}", prefix, a.text);
                let (w, h) = comment_box_size(&text);
                spec.push((text, a.color, w, h));
            }
        } else if let Some(&(ls, le)) = stmt_lines.get(stmt) {
            let code = source_lines
                .get(ls - 1)
                .map(|l| truncate_code(l.trim()))
                .unwrap_or_default();
            if !code.is_empty() {
                let mut text = format!("{} {}", line_ref(ls, le), code);
                let more = le - ls;
                if more > 0 {
                    text.push_str(&format!(
                        "\n(+{} more line{})",
                        more,
                        if more == 1 { "" } else { "s" }
                    ));
                }
                let (w, h) = comment_box_size(&text);
                spec.push((text, LABEL_COLOR, w, h));
            }
        }
        k = end;
        out.push(spec);
    }
    out
}

/// "Line 130:" for a single line, "Lines 130-135:" for a range.
fn line_ref(a: usize, b: usize) -> String {
    if a == b {
        format!("Line {}:", a)
    } else {
        format!("Lines {}-{}:", a, b)
    }
}

/// First source line shown in an auto label, capped so the box stays short.
fn truncate_code(s: &str) -> String {
    if s.chars().count() <= 60 {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(57).collect();
        t.push_str("...");
        t
    }
}

fn rect_hit(
    occupied: &[(f64, f64, f64, f64)],
    x: f64,
    y: f64,
    w: f64,
    h: f64,
) -> bool {
    // exact decimal touching (a box that exactly fills its reserved gap)
    // must not count as a hit: binary floats overshoot by ~1e-16
    const EPS: f64 = 1e-9;
    occupied.iter().any(|(ox, oy, ow, oh)| {
        x + EPS < ox + ow && *ox + EPS < x + w && y + EPS < oy + oh && *oy + EPS < y + h
    })
}

/// Source line range of every top-level statement: from its start line to
/// the line before the next statement, minus trailing blank/comment-only
/// filler lines between statements.
fn stmt_line_ranges(source: &str, starts: &[usize]) -> Vec<(usize, usize)> {
    let lines: Vec<&str> = source.lines().collect();
    let total = lines.len();
    let is_fill = |l: &str| l.trim().is_empty() || l.trim_start().starts_with("--");
    starts
        .iter()
        .enumerate()
        .map(|(i, &s)| {
            let raw_end = if i + 1 < starts.len() {
                starts[i + 1] - 1
            } else {
                total
            };
            let mut end = raw_end.min(total);
            while end > s && lines.get(end - 1).map(|l| is_fill(l)).unwrap_or(true) {
                end -= 1;
            }
            (s, end.max(s))
        })
        .collect()
}

/// Box size for a comment text. [v29] measured from the user's handmade
/// function_example: 'Line 1 - print("function_example v0.1 loaded")' = 44
/// chars in a 1.75-wide, 0.5-tall box -> ~0.040 units per char, one text
/// line = 0.5 tall; width grows with the longest line, height with the
/// estimated wrapped line count.
fn comment_box_size(text: &str) -> (f64, f64) {
    let lines: Vec<&str> = text.split('\n').collect();
    let maxchars = lines
        .iter()
        .map(|l| l.chars().count())
        .max()
        .unwrap_or(8)
        .max(8) as f64;
    let width = (maxchars * 0.040).clamp(0.75, 4.0);
    let cpl = ((width / 0.040) as usize).max(16);
    let wrapped: usize = lines
        .iter()
        .map(|l| (l.chars().count() + cpl - 1) / cpl)
        .sum::<usize>()
        .max(lines.len());
    let height = (0.5 * wrapped as f64).max(0.5);
    (width, height)
}
