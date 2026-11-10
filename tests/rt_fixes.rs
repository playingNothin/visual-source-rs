//! Regression tests for the round-trip fidelity fixes:
//!
//! - mixed table constructors keep Lua positional semantics
//!   (`{a = "x", 4, 5}` stores 4 at [1], not [2])
//! - nested call arguments keep their return capture (f(g(1)) runs g once)
//! - multi-arg print keeps the Roblox TAB separator
//! - generic-for keeps the loop key (for i, v in pairs(t))
//! - `#` on a string literal stays quoted (#"hello", not #hello)
//! - string escapes and long-bracket strings round-trip
//! - string methods route to the RS string blocks (:upper/:sub/:len)
//! - repeat-until parses (desugared to while true + break)
//! - method definitions and calls round-trip (obj:m() -> m(self))

use visual_source_rs::{encoder, transpile, VisualSource};

fn back(src: &str) -> String {
    let label = "rt_fixes";
    let out = transpile::transpile(src, label, true)
        .unwrap_or_else(|e| panic!("transpile failed: {}", e));
    let payload = encoder::encode_payload(&out.report).unwrap();
    let report =
        VisualSource::decode(&payload).unwrap_or_else(|e| panic!("re-decode failed: {}", e));
    visual_source_rs::transpile::detranspile::detranspile(&report).luau
}

#[test]
fn mixed_ctor_positional_indices() {
    let luau = back("local t = { a = \"x\", 4, 5 }\nprint(t[1], t[2])");
    assert!(luau.contains("[1] = 4"), "positional slot 1 lost:\n{}", luau);
    assert!(luau.contains("[2] = 5"), "positional slot 2 lost:\n{}", luau);
    assert!(!luau.contains("[2] = 4"), "4 shifted to slot 2:\n{}", luau);
}

#[test]
fn deferred_ctor_uses_positional_slot() {
    // deferred (non-literal) positional entries ride SetTableValue; the key
    // must be the positional slot, not the field index
    let luau = back("local v = 9\nlocal t = { k = \"x\", v, v }\nprint(t[1], t[2])");
    assert!(luau.contains("t[1] = v"), "deferred entry not at slot 1:\n{}", luau);
    assert!(luau.contains("t[2] = v"), "deferred entry not at slot 2:\n{}", luau);
    assert!(!luau.contains("t[3]"), "field-index key leaked:\n{}", luau);
}

#[test]
fn nested_call_args_keep_capture() {
    let luau = back("local function f(x) return x + 1 end\nlocal function g(x) return x * 2 end\nprint(f(g(1)))");
    // g runs exactly once (its result is captured and fed to f), no bare
    // discarded duplicate and no nil parameter
    assert_eq!(luau.matches("g(1)").count(), 1, "inner call duplicated:\n{}", luau);
    assert!(!luau.contains("f(nil)"), "capture lost, param became nil:\n{}", luau);
}

#[test]
fn print_multi_arg_tab_separator() {
    let luau = back("print(\"a\", \"b\")");
    assert!(luau.contains("\\t"), "tab separator missing:\n{}", luau);
}

#[test]
fn generic_for_keeps_loop_key() {
    let luau = back("local t = {10, 20}\nfor i, v in ipairs(t) do print(i, v) end");
    assert!(luau.contains("for i, v in pairs"), "loop key dropped:\n{}", luau);
}

#[test]
fn len_on_string_literal_stays_quoted() {
    let luau = back("print(#(\"hello\"))");
    assert!(luau.contains("#\"hello\""), "literal unquoted:\n{}", luau);
    assert!(!luau.contains("#hello"), "literal became a global ref:\n{}", luau);
}

#[test]
fn string_escapes_round_trip() {
    let luau = back("local s = \"a\\\"b\\\\c\"\nlocal t = 'x\\ny\\tz'\nprint(s, t)");
    assert!(luau.contains("\\\""), "double-quote escape lost:\n{}", luau);
    assert!(luau.contains("\\\\"), "backslash escape lost:\n{}", luau);
    assert!(luau.contains("\\n"), "newline escape lost:\n{}", luau);
    assert!(luau.contains("\\t"), "tab escape lost:\n{}", luau);
}

#[test]
fn long_bracket_strings_parse() {
    let luau = back("local u = [[raw\\\\n text]]\nprint(u)");
    assert!(luau.contains("raw"), "long-bracket string lost:\n{}", luau);
}

#[test]
fn string_methods_route_to_rs_blocks() {
    let luau = back("local s = \"abc\"\nprint(s:upper(), s:sub(2, 3), (#s))");
    assert!(luau.contains(":upper()"), "s:upper() not routed:\n{}", luau);
    assert!(
        luau.contains("string.sub(s, 2, 3)") || luau.contains("string.sub(\"abc\", 2, 3)"),
        "s:sub not routed:\n{}",
        luau
    );
    assert!(luau.contains("#s"), "s:len not routed:\n{}", luau);
    // literal bases get parens so the suffix is valid outside Luau's grammar
    let luau2 = back("print((\"abc\"):upper())");
    assert!(luau2.contains("(\"abc\"):upper()"), "literal base not parenthesized:\n{}", luau2);
}

#[test]
fn repeat_until_desugars_to_while_true_break() {
    let luau = back("local i = 3\nrepeat i = i - 1 until i <= 0\nprint(i)");
    assert!(luau.contains("while true"), "repeat not lowered:\n{}", luau);
    assert!(luau.contains("break"), "until condition missing:\n{}", luau);
    assert!(luau.contains("(i <= 0)"), "until condition wrong:\n{}", luau);
}

#[test]
fn method_definition_and_call_round_trip() {
    let src = "local obj = { v = 4 }\nfunction obj:get() return self.v end\nprint(obj:get())";
    let out = transpile::transpile(src, "method", true).unwrap();
    let unsupported: Vec<_> = out
        .notes
        .iter()
        .filter(|n| n.kind == "UNSUPPORTED" && n.msg.contains(":get"))
        .collect();
    assert!(unsupported.is_empty(), "obj:get() flagged unsupported: {:?}", unsupported);
    let luau = back(src);
    assert!(luau.contains("function get(self)"), "method def desugar lost:\n{}", luau);
    assert!(luau.contains("get(obj)"), "method call desugar lost:\n{}", luau);
}

#[test]
fn pcall_is_flagged_not_silently_true() {
    let out = transpile::transpile(
        "local ok, err = pcall(function() error(\"x\") end)\nprint(ok)",
        "pcall",
        true,
    )
    .unwrap();
    let flagged: Vec<_> = out
        .notes
        .iter()
        .filter(|n| n.kind == "UNSUPPORTED" && n.msg.contains("pcall"))
        .collect();
    assert_eq!(flagged.len(), 1, "pcall must be flagged unsupported: {:?}", out.notes);
}
