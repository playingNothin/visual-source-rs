//! v7 regression tests: else-branch continuations after while/for/if blocks
//! must stay INSIDE the enclosing DefineFunction subtree - never promoted to
//! startup roots ([patched: else-chain v7]).
//!
//! The finish pass sequences every root under one Comment2 container, so the
//! climb from any block ends at Comment1. The invariant is therefore: every
//! block's parent climb must PASS THROUGH a DefineFunction (function body),
//! unless the block itself is a legitimate top-level statement (top-level
//! `local t = {}` CreateTable, etc.). The pre-fix bug left post-while else
//! statements unparented; they climbed straight to Comment1 without ever
//! passing a DefineFunction and executed at spawn with unset variables.

use serde_json::Value;
use visual_source_rs::{VisualSource, encoder, transpile};

fn tr(label: &str, src: &str) -> transpile::TranspileOutput {
    let out = transpile::transpile(src, label, true)
        .unwrap_or_else(|e| panic!("[{}] transpile failed: {}", label, e));
    let catalog: Value = serde_json::from_str(visual_source_rs::vs_blocks::VS_BLOCKS_JSON).unwrap();
    for (name, b) in &out.report.blocks {
        let entry = catalog.get(&b.block_type).unwrap_or_else(|| {
            panic!("[{}] emitted type {:?} missing from catalog", label, b.block_type)
        });
        let cat = entry["Category"].as_str().unwrap();
        assert_ne!(cat, "Deprecated", "[{}] emits Deprecated {:?}", label, name);
        assert_ne!(cat, "Unverified", "[{}] emits Unverified {:?}", label, name);
    }
    out
}

/// Assert every block either lives inside a DefineFunction subtree (its
/// parent climb passes through a DefineFunction) or is one of the allowed
/// top-level block types.
fn assert_all_within_functions_or_top_level(
    out: &transpile::TranspileOutput,
    label: &str,
    allowed_top_level: &[&str],
) {
    let mut parents: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
    for (n, b) in &out.report.blocks {
        for c in &b.children {
            parents.insert(c.as_str(), n.as_str());
        }
        if let Some(ec) = &b.else_child {
            parents.insert(ec.as_str(), n.as_str());
        }
    }
    for (n, b) in &out.report.blocks {
        if allowed_top_level.contains(&b.block_type.as_str())
            || b.block_type == "Comment2"
            || b.block_type == "DefineFunction"
        {
            continue;
        }
        let mut cur: &str = n;
        let mut in_function = false;
        let mut depth = 0;
        while let Some(p) = parents.get(cur) {
            cur = p;
            depth += 1;
            assert!(depth < 2000, "[{}] parent cycle at {}", label, n);
            if out.report.blocks[cur].block_type == "DefineFunction" {
                in_function = true;
                break;
            }
        }
        assert!(
            in_function,
            "[{}] block {} ({}) is not inside any DefineFunction subtree - \
             it climbs straight to the root container and would execute at \
             spawn with unset variables",
            label,
            n,
            b.block_type
        );
    }
}

fn assert_lossless(label: &str, out: &transpile::TranspileOutput) {
    let payload = encoder::encode_payload(&out.report).unwrap();
    let red = VisualSource::decode(&payload)
        .unwrap_or_else(|e| panic!("[{}] re-decode failed: {}", label, e));
    assert_eq!(red.blocks, out.report.blocks, "[{}] tree equality", label);
    let payload2 = encoder::encode_payload(&red).unwrap();
    assert_eq!(payload, payload2, "[{}] deterministic encoding", label);
}

const ELSE_WHILE: &str = r#"
local squares = {}
local function updateGrid(fPos, fSize, doInterlace)
    for _, ij in ipairs(squares) do
        local i, j = ij[1], ij[2]
        if doInterlace and ((i + j) % 2) ~= 1 then
            squares[i][j] = 1
        else
            local x = fPos.X + (i - 0.5) * 0.1
            local r = 0
            local depth = 0
            while depth < 4 do
                local hit = workspace:Raycast(Vector3.new(0, 0, 0), Vector3.new(1, 0, 0))
                if not hit then break end
                r = r + hit.Transparency
                depth = depth + 1
            end
            local acc = (r == 0) and 0.5 or r
            squares[i][j] = acc
        end
    end
end
"#;

#[test]
fn transpile_else_while_continuation_stays_in_function() {
    // USER-CAUGHT (v7): the raycast half of updateGrid lives in the else of
    // the interlace check. Statements after the while loop (acc ternary +
    // cell write) were orphaned by the B6 windows(2) linker, promoted to
    // startup roots, and executed once at spawn: Get Table Value39
    // "Table does not exist" + Solve Equation115-118 "sub on number and nil".
    let out = tr("else_while", ELSE_WHILE);
    assert_all_within_functions_or_top_level(&out, "else_while", &["CreateTable"]);
    assert_lossless("else_while", &out);
}

const ELSE_IF_TAIL: &str = r#"
local t = {}
local function f(a, b)
    for _, v in ipairs(t) do
        if a > 1 then
            t[v] = a
        else
            local acc = (b == 0) and 1 or b
            if acc > 2 then
                t[v] = acc
            end
            t[v] = acc + 1
        end
    end
end
"#;

#[test]
fn transpile_else_if_tail_stays_in_function() {
    // An if INSIDE the else branch followed by more statements: the
    // continuation after the inner if must not be orphaned either.
    let out = tr("else_if_tail", ELSE_IF_TAIL);
    assert_all_within_functions_or_top_level(&out, "else_if_tail", &["CreateTable"]);
    assert_lossless("else_if_tail", &out);
}

const ELSEIF_GATES: &str = r#"
local threshold = 0
local function updateDefaultColor(posDiff, rotDiff)
    local phases = 1
    if posDiff > 1.5 or rotDiff > 0.4 then
        threshold = 0.02
        phases = 2
    elseif posDiff > 0.2 or rotDiff > 0.1 then
        threshold = 0.01
    end
    return phases
end
"#;

#[test]
fn transpile_elseif_gates_stay_linked() {
    // B6 regression: elseif gate blocks must stay inside the function
    // (pre-B6 they were unparented roots; the B6 windows loop fixed leaves
    // but v7 keeps that guarantee through chain_walk).
    let out = tr("elseif_gates", ELSEIF_GATES);
    assert_all_within_functions_or_top_level(&out, "elseif_gates", &["SetVariable1"]);
    assert_lossless("elseif_gates", &out);
}
