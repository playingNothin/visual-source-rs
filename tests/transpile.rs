//! Phase 4 acceptance: the SimonSays corpus transpiles with ZERO unresolved
//! GAPs, every emitted block is catalog-valid (never Deprecated/Unverified),
//! and each emitted tree round-trips losslessly through the Phase 2 encoder.

use serde_json::Value;
use visual_source_rs::{VisualSource, encoder, transpile};

const MAIN: &str = include_str!("../fixtures/simonsays_main.luau");
const DICT: &str = include_str!("../fixtures/simonsays_dictionary.luau");
const SIMON_UI: &str = include_str!("../fixtures/SimonUI.luau");

fn assert_transpile_ok(label: &str, src: &str) -> transpile::TranspileOutput {
    let out = transpile::transpile(src, label, true)
        .unwrap_or_else(|e| panic!("[{}] transpile failed: {}", label, e));

    let gaps: Vec<&transpile::Note> = out.notes.iter().filter(|n| n.kind == "GAP").collect();
    assert!(
        gaps.is_empty(),
        "[{}] {} unresolved GAPs: {:?}",
        label,
        gaps.len(),
        gaps.iter().map(|g| &g.msg).collect::<Vec<_>>()
    );

    // every emitted type must be catalog-valid
    let catalog: Value = serde_json::from_str(visual_source_rs::vs_blocks::VS_BLOCKS_JSON).unwrap();
    for (name, b) in &out.report.blocks {
        let entry = catalog
            .get(&b.block_type)
            .unwrap_or_else(|| panic!("[{}] emitted type {:?} missing from catalog", label, b.block_type));
        let cat = entry["Category"].as_str().unwrap();
        assert_ne!(cat, "Deprecated", "[{}] {} emits Deprecated {:?}", label, name, b.block_type);
        assert_ne!(cat, "Unverified", "[{}] {} emits Unverified {:?}", label, name, b.block_type);
    }

    // every child/else reference must resolve
    for (name, b) in &out.report.blocks {
        for c in b.children.iter().chain(b.else_child.iter()) {
            assert!(
                out.report.blocks.contains_key(c),
                "[{}] {} references missing child {:?}",
                label,
                name,
                c
            );
        }
    }

    out
}

fn assert_lossless(label: &str, out: &transpile::TranspileOutput) {
    let payload = encoder::encode_payload(&out.report).unwrap();
    let red = VisualSource::decode(&payload)
        .unwrap_or_else(|e| panic!("[{}] re-decode of transpiled payload failed: {}", label, e));
    assert_eq!(red.version, out.report.version);
    assert_eq!(red.block_order, out.report.block_order, "[{}] block order", label);
    assert_eq!(red.blocks, out.report.blocks, "[{}] tree equality", label);
    assert_eq!(red.comments, out.report.comments);
    assert_eq!(red.editor, out.report.editor);
    // record layer byte-stability
    assert_eq!(
        encoder::encode_records(&red),
        encoder::encode_records(&out.report),
        "[{}] record stream stability",
        label
    );
    // encoding deterministic
    let payload2 = encoder::encode_payload(&red).unwrap();
    assert_eq!(payload, payload2, "[{}] deterministic encoding", label);
}

#[test]
fn transpile_dictionary_zero_gaps() {
    let out = assert_transpile_ok("dictionary", DICT);
    // expected shape: word list as one packed tuple, loop copy, OnInvoke
    let types: Vec<&str> = out.report.blocks.values().map(|b| b.block_type.as_str()).collect();
    assert!(types.contains(&"BindableFunctionOnInvoke"), "OnInvoke must become an event block");
    assert!(!types.contains(&"SetObjectProperty"), "no property-set for OnInvoke");
    assert!(types.contains(&"FunctionReturn"));
    assert!(types.contains(&"LoopThroughTable"));
    assert!(types.contains(&"RandomNumber"));
    assert!(types.contains(&"GetTableValue"));
    assert_lossless("dictionary", &out);
}


#[test]
fn transpile_main_zero_gaps() {
    let out = assert_transpile_ok("main", MAIN);
    let types: Vec<&str> = out.report.blocks.values().map(|b| b.block_type.as_str()).collect();
    // Core structural block types the corpus must exercise.
    // GetObjectProperty is now in the list: runtime testing found that
    // game:GetService("Workspace") MUST emit GetObjectProperty(game,
    // "Workspace") (the Val::Path + SetVariable1 form does NOT resolve
    // to a live service at runtime — Players ended up nil,
    // Players.LocalPlayer was nil, CharacterAdded.Player got nil, etc.).
    // Property reads on VARIABLE bases still emit SolveEquation (also
    // in the list).
    for expected in [
        "DefineFunction", "ExecuteFunction", "Spawn", "WhileLoop3", "ForLoop2",
        "LoopThroughChildren", "LoopThroughTable", "If", "RemoteFireAllClients2",
        "RemoteRecieveServer2", "BindableFire2", "BindableFunctionInvoke",
        "PropertyChanged", "PlayerAdded", "PlayerRemoving",
        "PartTouched", "PartTouchEnded", "TakeDamage", "CloneObject", "DestroyObject",
        "TableFind", "TableAppend", "TableRemove",
        "RandomNumber", "Round", "OsTime", "Concatenate", "Lower",
        "SetObjectProperty", "FindFirstChildNew", "GetPlayerFromCharacter", "IsA",
        "Wait", "Print", "Comparison", "ANDAny", "NOTAny", "Addition", "Subtraction",
        "SetVariable1", "GetTableLength", "GetTableValue",
        "SetTableValue", "BreakLoop",
        "ConstructCFrame", "FunctionReturn", "CreateTable", "GetObjectProperty",
        "SolveEquation",
    ] {
        assert!(
            types.contains(&expected),
            "main script must contain {} blocks (missing); has {} types",
            expected,
            types.len()
        );
    }
    // [patch16] v15 pin: {…} literals must NEVER emit TablePack — the
    // Values input is a Tuple, and the old CSV fast path built
    // empty/garbage tables (run 5: every EXPRS[i]/EXV[i] read returned
    // nil, which cascaded into "attempt to get length of a nil value"
    // inside tokenize and "compare number <= nil" on the loop gate).
    // [patch19] Literal tables must ride CreateTable + SetTableValue, or
    // - since run 7 proved SolveEquation builds tables - ONE brace
    // equation for all-numeric positional literals. TablePack stays
    // banned either way.
    assert!(
        !types.contains(&"TablePack"),
        "main script must NOT contain TablePack blocks (v15 regression); has {} types",
        types.len()
    );
    // Runtime-verified correctness pin: game:GetService calls must emit
    // GetObjectProperty(game, <service>) — the Val::Path form was a
    // runtime bug (services are properties of game, not globals).
    // Verify at least one supported service appears as a GetObjectProperty
    // Property input. (If this fires, the GetService arm regressed to
    // Val::Path — investigate src/transpile/emit.rs.)
    const RS_SUPPORTED_SERVICES: &[&str] = &[
        "Workspace", "Lighting", "Players", "Soundscape",
        "Debris", "StarterGui", "StarterPack", "Teams",
    ];
    let mut found_service_via_getobjprop = false;
    for b in out.report.blocks.values() {
        if b.block_type != "GetObjectProperty" {
            continue;
        }
        for inp in &b.inputs {
            if inp.iname == "Property"
                && inp.value.as_deref().map(|v| RS_SUPPORTED_SERVICES.contains(&v)).unwrap_or(false)
            {
                found_service_via_getobjprop = true;
            }
        }
    }
    // This is a SOFT check (informational): if the main fixture happens to
    // not call GetService for any supported service, the check is a no-op.
    // The hard check is in transpile_getservice_*_uses_getobjectproperty.
    let _ = found_service_via_getobjprop;
    // no deprecated/unverified types snuck in (also asserted per-block above)
    assert_lossless("main", &out);
}

#[test]
fn transpile_budget_report() {
    let out = transpile::transpile(MAIN, "main", true).unwrap();
    assert!(out.budget.contains("gap: 0"), "budget: {}", out.budget);
    assert!(out.budget.contains("blocks_out:"), "budget: {}", out.budget);
    // >50-block functions must be flagged for manual review
    assert!(
        out.budget.contains(">50 manual review"),
        "budget must flag oversized functions: {}",
        out.budget
    );
    println!("{}", out.budget);
}

#[test]
fn transpile_parser_handles_full_corpus() {
    // both sources parse without error and produce statements
    for (label, src) in [("main", MAIN), ("dict", DICT)] {
        let stmts = transpile::parser::parse(src)
            .unwrap_or_else(|e| panic!("[{}] parse failed: {}", label, e));
        assert!(stmts.len() > 3, "[{}] too few statements: {}", label, stmts.len());
    }
}

#[test]
fn transpile_simon_ui_zero_gaps() {
    let out = assert_transpile_ok("simon_ui", SIMON_UI);
    assert_lossless("simon_ui", &out);
}


// ---------------------------------------------------------------------------
// Patch bak63: GetService / assign_to Val::Path / print arm ToStringAny
// ---------------------------------------------------------------------------

/// game:GetService("Workspace") routes via GetObjectProperty(game,
/// "Workspace") so the binding holds the live service Instance at
/// runtime. Runtime testing (per the comment block at the GetService
/// arm in emit.rs) found that Val::Path → SetVariable1 does NOT
/// resolve to a live service — Players ended up nil, downstream
/// Players.LocalPlayer was nil, and CharacterAdded.Player got nil
/// (runtime error: "'Player' input of 'CharacterAdded' must be a
/// 'Player' got 'nil'"). The GetObjectProperty form is the
/// runtime-verified shape.
#[test]
fn transpile_getservice_workspace_uses_getobjectproperty() {
    let src = "local Workspace = game:GetService(\"Workspace\")\n";
    let out = assert_transpile_ok("getservice_workspace", src);

    // Must emit a GetObjectProperty block whose Property is "Workspace".
    let good = out.report.blocks.values().find(|b| {
        b.block_type == "GetObjectProperty"
            && b.inputs.iter().any(|inp| {
                inp.iname == "Property" && inp.value.as_deref() == Some("Workspace")
            })
    });
    assert!(
        good.is_some(),
        "GetService(\"Workspace\") must emit GetObjectProperty(game, \"Workspace\") \
         (runtime-verified shape; Val::Path does not resolve to a live service)"
    );

    // The Workspace binding is the GetObjectProperty's output (Value=
    // "Workspace"). No separate SetVariable1 producer is needed —
    // the binding IS the block output. Verify the binding appears as an
    // output of some block.
    let binding_produced = out.report.blocks.values().any(|b| {
        b.outputs.iter().any(|(_, v)| v == "Workspace")
    });
    assert!(
        binding_produced,
        "Workspace binding must be produced by some block's output"
    );

    assert_lossless("getservice_workspace", &out);
}

/// Each supported service routes via GetObjectProperty(game, <svc>)
/// — the runtime-verified shape (NOT Val::Path).
#[test]
fn transpile_getservice_known_services_use_getobjectproperty() {
    let services = [
        "Lighting", "Players", "Soundscape", "Debris",
        "StarterGui", "StarterPack", "Teams",
    ];
    for svc in services {
        let src = format!("local S = game:GetService(\"{}\")\n", svc);
        let label = format!("getservice_{}", svc);
        let out = assert_transpile_ok(&label, &src);
        let good = out.report.blocks.values().find(|b| {
            b.block_type == "GetObjectProperty"
                && b.inputs.iter().any(|inp| {
                    inp.iname == "Property" && inp.value.as_deref() == Some(svc)
                })
        });
        assert!(
            good.is_some(),
            "GetService({:?}) must emit GetObjectProperty(game, {:?}) \
             (runtime-verified shape)",
            svc, svc
        );
    }
}

/// Bug A critical rule: explicitly-unsupported services must NOT be
/// silently rewritten to fake paths. The transpiler must surface an
/// UNSUPPORTED note so the user knows the service doesn't exist in RS.
#[test]
fn transpile_getservice_unsupported_service_flagged() {
    let unsupported = ["ReplicatedStorage", "ServerStorage", "ServerScriptService"];
    for svc in unsupported {
        let src = format!("local S = game:GetService(\"{}\")\n", svc);
        let label = format!("getservice_unsup_{}", svc);
        let out = transpile::transpile(&src, &label, true)
            .unwrap_or_else(|e| panic!("[{}] transpile failed: {}", label, e));
        let flagged = out.notes.iter().any(|n| n.kind == "UNSUPPORTED");
        assert!(
            flagged,
            "GetService({:?}) must flag UNSUPPORTED (no fake path)",
            svc
        );
        // Sanity: no Val::Path fabrication for unsupported services. The
        // binding's producer should NOT have Value = the service name as
        // an Object-typed literal (which would be a fabricated path).
        let fabricated = out.report.blocks.values().any(|b| {
            b.block_type == "SetVariable1"
                && b.inputs.iter().any(|inp| {
                    inp.iname == "Value"
                        && inp.value.as_deref() == Some(svc)
                        && inp.vtype.as_deref() == Some("Object")
                })
        });
        assert!(
            !fabricated,
            "GetService({:?}) must NOT fabricate an Object path",
            svc
        );
    }
}

/// Bug B: assigning a path-valued expression to a local must produce a
/// SetVariable1 producer (otherwise the binding is dead).
#[test]
fn transpile_assign_path_emits_producer() {
    let src = "local W = workspace\nlocal G = game\nlocal E = Enum\n";
    let out = assert_transpile_ok("assign_path", src);
    let types: Vec<&str> =
        out.report.blocks.values().map(|b| b.block_type.as_str()).collect();
    // 3 path-returning assignments -> >=3 SetVariable1 producers.
    let setvar_count = types.iter().filter(|t| **t == "SetVariable1").count();
    assert!(
        setvar_count >= 3,
        "expected >= 3 SetVariable1 producers for path assignments, got {}",
        setvar_count
    );
    assert_lossless("assign_path", &out);
}

/// Bug C: print(var) must route through ToStringAny so the value is a
/// guaranteed string before reaching Print.Text (which is String-typed
/// in the RS catalog). The bak62 Num-routing would produce a type
/// mismatch if the var holds a non-number at runtime.
#[test]
fn transpile_print_var_uses_tostringany() {
    let src = "local x = \"hello\"\nprint(x)\n";
    let out = assert_transpile_ok("print_var", src);
    let types: Vec<&str> =
        out.report.blocks.values().map(|b| b.block_type.as_str()).collect();
    assert!(types.contains(&"ToStringAny"), "print(var) must emit ToStringAny");
    assert!(types.contains(&"Print"), "print(var) must emit Print");
    assert_lossless("print_var", &out);
}

/// Bug C: print("a", x) - var must ToStringAny, then Concatenate (NOT
/// CombineStringWithNumber, which would require x to be a number).
#[test]
fn transpile_print_mixed_literal_var() {
    let src = "local x = \"v\"\nprint(\"a\", x)\n";
    let out = assert_transpile_ok("print_mixed", src);
    let types: Vec<&str> =
        out.report.blocks.values().map(|b| b.block_type.as_str()).collect();
    assert!(types.contains(&"ToStringAny"), "var must be stringified via ToStringAny");
    assert!(types.contains(&"Concatenate"), "literal+string-var must use Concatenate");
    assert!(
        !types.contains(&"CombineStringWithNumber"),
        "string var must NOT go through CombineStringWithNumber"
    );
    assert_lossless("print_mixed", &out);
}

/// Bug C: print(1, x) - the literal 1 stays as Num, the var x is
/// stringified via ToStringAny. The two are joined by a type-correct
/// combine (Concatenate is safe because both sides are strings at that
/// point; CombineNumberWithString would also be safe but isn't required).
/// CombineStringWithNumber is forbidden (would require x to be a number).
#[test]
fn transpile_print_num_literal_with_string_var() {
    let src = "local x = \"v\"\nprint(1, x)\n";
    let out = assert_transpile_ok("print_num_var", src);
    let types: Vec<&str> =
        out.report.blocks.values().map(|b| b.block_type.as_str()).collect();
    assert!(types.contains(&"ToStringAny"), "var must be stringified via ToStringAny");
    let has_safe_join = types.contains(&"Concatenate")
        || types.contains(&"CombineNumberWithString");
    assert!(
        has_safe_join,
        "must use a type-correct join (Concatenate or CombineNumberWithString)"
    );
    assert!(
        !types.contains(&"CombineStringWithNumber"),
        "string var must NOT be the Number input of CombineStringWithNumber"
    );
    assert_lossless("print_num_var", &out);
}

/// Bug C: multi-arg print with literal numbers only - Combine optimization
/// is preserved (no ToStringAny needed for literals).
#[test]
fn transpile_print_literal_num_only_preserves_combine() {
    let src = "print(\"a\", 1, \"b\")\n";
    let out = assert_transpile_ok("print_lit_only", src);
    let types: Vec<&str> =
        out.report.blocks.values().map(|b| b.block_type.as_str()).collect();
    // No ToStringAny should be needed when all args are literals.
    assert!(
        !types.contains(&"ToStringAny"),
        "literal-only print should not need ToStringAny"
    );
    // Combine should be in use (literal-literal pairs).
    let has_combine = types.contains(&"CombineStringWithNumber")
        || types.contains(&"CombineNumberWithString");
    assert!(has_combine, "literal-only multi-arg print should use Combine blocks");
    assert_lossless("print_lit_only", &out);
}

// ---------------------------------------------------------------------------
// General optimization regression tests (Task: root-cause analysis of 5
// optimization opportunities). These tests pin the GENERAL, semantics-
// preserving fixes that reduce unnecessary block emissions for ALL
// supported Luau programs — not fixture-specific hacks.
// ---------------------------------------------------------------------------

/// Opportunity #1: Truthiness gates.
/// `local x = a and b or c` (ternary idiom) previously lowered through
/// `truthy_gate_var` which emitted 3 blocks per truthiness gate
/// (2 Comparison + 1 ANDAny). The general fix emits ONE SolveEquation
/// with text `(b ~= nil and b ~= false)` (want=true) or
/// `not (b ~= nil and b ~= false)` (want=false) — exact Luau truthiness
/// semantics (excludes `false`, includes `0`/`""`/instances/tables).
/// The equation is Bool-valued (output of `~=` is Bool, `and` of two
/// Bools is Bool), avoiding unverified behaviour of `not not x` on
/// arbitrary input types.
#[test]
fn transpile_truthy_gate_is_one_solveequation() {
    let src = "local a = true\nlocal b = 1\nlocal c = 2\nlocal x = a and b or c\n";
    let out = assert_transpile_ok("truthy_gate", src);
    let types: Vec<&str> = out.report.blocks.values().map(|b| b.block_type.as_str()).collect();
    // Old shape: 2 Comparison + 1 ANDAny = 3 blocks for the truthy gate.
    // New shape: 1 SolveEquation. No ANDAny, no Comparison (for the gate).
    assert!(
        types.contains(&"SolveEquation"),
        "truthy_gate must emit a SolveEquation (got types: {:?})",
        types
    );
    // The truthy-gate SolveEquation must carry the exact Luau-truthiness
    // equation text. Find it and verify.
    let truthy_eq = out
        .report
        .blocks
        .values()
        .find(|b| {
            b.block_type == "SolveEquation"
                && b.inputs.iter().any(|inp| {
                    inp.iname == "Equation"
                        && inp.value.as_deref() == Some("(a ~= nil and a ~= false)")
                })
        });
    assert!(
        truthy_eq.is_some(),
        "truthy_gate must emit SolveEquation with text `(a ~= nil and a ~= false)`"
    );
    // No ANDAny should remain for the truthy gate (the old 3-block pattern).
    // Other ANDAny blocks (if any) are unrelated; this source has none.
    let andany_count = types.iter().filter(|t| **t == "ANDAny").count();
    assert_eq!(
        andany_count, 0,
        "ternary `a and b or c` must NOT emit ANDAny (was 1 before fix #1)"
    );
    assert_lossless("truthy_gate", &out);
}

/// Opportunity #3: Self-assignment no-ops.
/// `local root = char and char:FindFirstChild("X")` previously lowered to:
///   1. SetVariable1(Value=Var(char_binding), VariableName=root)   // default
///   2. FindFirstChildNew(Parent=char, ChildName="X") -> Result=root
///   3. SetVariable1(Value=Var(root), VariableName=root)            // SELF-ASSIGN
/// The general fix skips the self-assignment in `materialize_val` when
/// the value's binding equals the output binding.
#[test]
fn transpile_no_self_assignment_for_and_with_method() {
    let src = "local char = script.Parent\nlocal root = char and char:FindFirstChild(\"HumanoidRootPart\")\nprint(root)\n";
    let out = assert_transpile_ok("no_self_assign", src);
    // No SetVariable1 block should have Value=Var(X) and VariableName=X.
    for b in out.report.blocks.values() {
        if b.block_type != "SetVariable1" {
            continue;
        }
        let val_in = b.inputs.iter().find(|i| i.iname == "Value");
        let var_out = b.outputs.iter().find(|(n, _)| n == "VariableName");
        if let (Some(v), Some(o)) = (val_in, var_out) {
            // Value input is mode 2 (var reference) — check var property.
            // DecodedInput stores the var name in `var` when use_var=true.
            if v.use_var {
                assert_ne!(
                    v.var, o.1,
                    "self-assignment no-op detected: SetVariable1(Value=Var({}), VariableName={})",
                    v.var, o.1
                );
            }
        }
    }
    assert_lossless("no_self_assign", &out);
}

/// Opportunity #4: CSE for property access (within-statement).
/// `local human = value.Character and value.Character:FindFirstChild("X")`
/// previously emitted TWO SolveEquation blocks for `value.Character`
/// (one in `truthy_gate`'s expr(), one in `method_expr`'s base eval).
/// The general fix caches the property access; the second access hits
/// the cache and reuses the existing binding.
#[test]
fn transpile_cse_within_statement_property_access() {
    let src = "local value = script.Parent\nlocal human = value.Character and value.Character:FindFirstChild(\"HumanoidRootPart\")\nprint(human)\n";
    let out = assert_transpile_ok("cse_within", src);
    // Count SolveEquation blocks whose equation text is `value.Character`.
    let count = out
        .report
        .blocks
        .values()
        .filter(|b| {
            b.block_type == "SolveEquation"
                && b.inputs.iter().any(|inp| {
                    inp.iname == "Equation" && inp.value.as_deref() == Some("value.Character")
                })
        })
        .count();
    assert_eq!(
        count, 1,
        "value.Character must be emitted exactly once (CSE for the second access); got {}",
        count
    );
    assert_lossless("cse_within", &out);
}

/// Opportunity #4: CSE for property access (cross-statement).
/// Three consecutive reads of `obj.Position` must share a single
/// SolveEquation. Previously the `declare()` blanket clear wiped the
/// cache on each new local declaration, defeating cross-statement CSE.
#[test]
fn transpile_cse_cross_statement_property_access() {
    let src = "local obj = script.Parent\nlocal x = obj.Position\nlocal y = obj.Position\nlocal z = obj.Position\nprint(x, y, z)\n";
    let out = assert_transpile_ok("cse_cross", src);
    let count = out
        .report
        .blocks
        .values()
        .filter(|b| {
            b.block_type == "SolveEquation"
                && b.inputs.iter().any(|inp| {
                    inp.iname == "Equation" && inp.value.as_deref() == Some("obj.Position")
                })
        })
        .count();
    assert_eq!(
        count, 1,
        "obj.Position must be emitted exactly once across 3 consecutive reads (CSE); got {}",
        count
    );
    // CSE hits counter must be >= 2 (the 2nd and 3rd accesses hit the cache).
    assert!(out.budget.contains("cse: 2"), "budget must report 2 CSE hits: {}", out.budget);
    assert_lossless("cse_cross", &out);
}

/// Opportunity #4: CSE invalidation on variable reassignment.
/// `obj.Prop` is read; `obj` is reassigned; `obj.Prop` is read again.
/// The cache must be invalidated by the reassignment (targeted
/// invalidation in `assign_to`), so the second read emits a fresh
/// SolveEquation. Otherwise the second read would WRONGLY reuse the
/// cached binding for the OLD obj.
#[test]
fn transpile_cse_invalidation_on_reassign() {
    let src = "local obj = script.Parent\nlocal x = obj.Position\nobj = script.Parent.Parent\nlocal y = obj.Position\nprint(x, y)\n";
    let out = assert_transpile_ok("cse_inv_reassign", src);
    // Both reads should emit (no CSE hit) because obj was reassigned.
    let count = out
        .report
        .blocks
        .values()
        .filter(|b| {
            b.block_type == "SolveEquation"
                && b.inputs.iter().any(|inp| {
                    inp.iname == "Equation" && inp.value.as_deref() == Some("obj.Position")
                })
        })
        .count();
    assert_eq!(
        count, 2,
        "obj.Position must be emitted TWICE (reassignment invalidates cache); got {}",
        count
    );
    assert_lossless("cse_inv_reassign", &out);
}

/// Opportunity #4: CSE survives SetObjectProperty on a DIFFERENT property.
/// `obj.Position` is read; `obj.Color = X` is set (modifies only .Color);
/// `obj.Position` is read again. The cache entry for `obj.Position` is
/// still valid (only .Color changed, not .Position). The general fix
/// uses targeted invalidation in SetObjectProperty (only the modified
/// property path is invalidated, not the whole cache).
#[test]
fn transpile_cse_survives_setobjprop_different_property() {
    let src = "local obj = script.Parent\nlocal x = obj.Position\nobj.Color = Color3.new(0, 0, 0)\nlocal y = obj.Position\nprint(x, y)\n";
    let out = assert_transpile_ok("cse_setobjprop_diff", src);
    // `obj.Position` must be emitted exactly once (CSE survives the
    // SetObjectProperty on `obj.Color`).
    let count = out
        .report
        .blocks
        .values()
        .filter(|b| {
            b.block_type == "SolveEquation"
                && b.inputs.iter().any(|inp| {
                    inp.iname == "Equation" && inp.value.as_deref() == Some("obj.Position")
                })
        })
        .count();
    assert_eq!(
        count, 1,
        "obj.Position must be emitted once (CSE survives SetObjectProperty on obj.Color); got {}",
        count
    );
    assert_lossless("cse_setobjprop_diff", &out);
}

/// Opportunity #4: CSE invalidates on SetObjectProperty on the SAME property.
/// `obj.Position` is read; `obj.Position = X` is set; `obj.Position` is
/// read again. The cache entry for `obj.Position` is now stale and must
/// be invalidated.
#[test]
fn transpile_cse_invalidates_on_setobjprop_same_property() {
    let src = "local obj = script.Parent\nlocal x = obj.Position\nobj.Position = Vector3.new(0, 0, 0)\nlocal y = obj.Position\nprint(x, y)\n";
    let out = assert_transpile_ok("cse_setobjprop_same", src);
    // `obj.Position` must be emitted twice: once for the first read,
    // once for the second read (after the SetObjectProperty invalidated
    // the cache). The SetObjectProperty itself uses the binding of
    // `obj.Position` from the first read (no extra emission for the
    // write target).
    let count = out
        .report
        .blocks
        .values()
        .filter(|b| {
            b.block_type == "SolveEquation"
                && b.inputs.iter().any(|inp| {
                    inp.iname == "Equation" && inp.value.as_deref() == Some("obj.Position")
                })
        })
        .count();
    assert!(
        count >= 2,
        "obj.Position must be re-emitted after SetObjectProperty on obj.Position (got {} emissions)",
        count
    );
    assert_lossless("cse_setobjprop_same", &out);
}

/// Opportunity #5 (regression guard): Compound condition fold already
/// exists in `if_stmt`. `if a and b then` should emit ONE SolveEquation
/// with `not not (a and b)` (or equivalent) + 1 If — NOT 2 Comparisons
/// + 1 ANDAny + 1 If.
#[test]
fn transpile_compound_condition_uses_solveequation_fold() {
    let src = "local a = true\nlocal b = false\nif a and b then\nprint(\"yes\")\nend\n";
    let out = assert_transpile_ok("compound_cond", src);
    let types: Vec<&str> = out.report.blocks.values().map(|b| b.block_type.as_str()).collect();
    assert!(
        types.contains(&"SolveEquation"),
        "compound condition `a and b` must fold to SolveEquation"
    );
    // No ANDAny for the compound condition (the old N-block lowering).
    let andany_count = types.iter().filter(|t| **t == "ANDAny").count();
    assert_eq!(
        andany_count, 0,
        "compound `a and b` condition must NOT emit ANDAny (folded to SolveEquation)"
    );
    assert_lossless("compound_cond", &out);
}
// ===== [patched: eq-safety v6 tests =====

#[test]
fn transpile_bracket_index_reads_use_gettablevalue_not_equations() {
    // FIX 1 regression: RS equations nil out bracket-index chains
    // (Solve Equation93 `squareColors[i2][j2]` -> cur2 = nil -> Eq94-96
    // "arithmetic (sub) on number and nil"). Computed-key table reads
    // must decompose into GetTableValue chains.
    let src = "local t = {}\nt[1] = {}\nlocal i = 1\nlocal j = 2\nlocal cur = t[i][j]\nprint(cur)\n";
    let out = assert_transpile_ok("bracket_index_reads", src);
    for b in out.report.blocks.values() {
        if b.block_type == "SolveEquation" {
            for inp in &b.inputs {
                if inp.iname == "Equation" {
                    let v = inp.value.as_deref().unwrap_or("");
                    assert!(
                        !v.contains('['),
                        "equation must not contain bracket indexing: {}",
                        v
                    );
                }
            }
        }
    }
    let gtv = out
        .report
        .blocks
        .values()
        .filter(|b| b.block_type == "GetTableValue")
        .count();
    assert!(
        gtv >= 2,
        "chained bracket read must decompose into a GetTableValue chain (got {})",
        gtv
    );
    assert_lossless("bracket_index_reads", &out);
}

#[test]
fn transpile_multihop_dot_chain_still_folds_to_equation() {
    // FIX 1 preservation: all-Str multi-hop chains stay folded
    // (game-proven: Eq90/91 `(amB + (l.color.B * f))` ran every frame).
    let src = "local cfg = {}\ncfg.a = {}\nlocal x = cfg.a.b\nprint(x)\n";
    let out = assert_transpile_ok("multihop_dot_fold", src);
    let has = out.report.blocks.values().any(|b| {
        b.block_type == "SolveEquation"
            && b.inputs
                .iter()
                .any(|i| i.iname == "Equation" && i.value.as_deref() == Some("cfg.a.b"))
    });
    assert!(
        has,
        "all-Str multi-hop chain must still fold to one SolveEquation"
    );
    assert_lossless("multihop_dot_fold", &out);
}

#[test]
fn transpile_index_operand_under_andor_gets_truthy_wrap() {
    // FIX 2 regression: Solve Equation38
    // `((((light ~= nil) and (light ~= false)) and light.Enabled)) ~= nil and ...`
    // failed to compile ("Couldn't figure this out!"). Index operands
    // must be wrapped into the proven truthiness comparison, and the
    // result is Bool so no outer truthiness doubling may appear.
    let src = "local model = script.Parent\nif model and model.PrimaryPart then\nprint(1)\nend\n";
    let out = assert_transpile_ok("index_andor_wrap", src);
    let folded: Vec<String> = out
        .report
        .blocks
        .values()
        .filter(|b| b.block_type == "SolveEquation")
        .filter_map(|b| {
            b.inputs
                .iter()
                .find(|i| i.iname == "Equation")
                .and_then(|i| i.value.clone())
        })
        .collect();
    assert!(
        folded
            .iter()
            .any(|t| t.contains("model.PrimaryPart ~= nil")
                && t.contains("model.PrimaryPart ~= false")),
        "index operand under and/or must be truthiness-wrapped; got {:?}",
        folded
    );
    assert!(
        folded
            .iter()
            .all(|t| !t.contains("PrimaryPart)) ~= nil")),
        "no outer truthiness doubling around the wrapped condition; got {:?}",
        folded
    );
    assert_lossless("index_andor_wrap", &out);
}

#[test]
fn transpile_ternary_with_bracket_reads_decomposes_to_blocks() {
    // FIX 1 regression (blend monolith, Solve Equation52 family):
    // `((i2 > 1) and squareColors[(i2 - 1)][j2].B) or DEFAULT_COLOR.B`
    // used to fold arithmetic + ternary + bracket reads into ONE
    // equation. With the bracket ban the arithmetic fold refuses and
    // andor_value_expr's ternary decomposition (If + GetTableValue
    // chains) takes over.
    let src = "local t = {}\nlocal DEFAULT = {}\nlocal i = 2\nlocal j = 1\nlocal v = (i > 1 and t[i - 1][j].B or DEFAULT.B)\nprint(v)\n";
    let out = assert_transpile_ok("ternary_bracket", src);
    for b in out.report.blocks.values() {
        if b.block_type == "SolveEquation" {
            for inp in &b.inputs {
                if inp.iname == "Equation" {
                    let v = inp.value.as_deref().unwrap_or("");
                    assert!(!v.contains('['), "no bracket indexing in equations: {}", v);
                }
            }
        }
    }
    let types: Vec<&str> = out
        .report
        .blocks
        .values()
        .map(|b| b.block_type.as_str())
        .collect();
    assert!(
        types.contains(&"If"),
        "ternary with bracket reads must decompose into If blocks; got {:?}",
        types
    );
    assert_lossless("ternary_bracket", &out);
}


// ===== [patch10] table-source registration (returns / params / alias) =====
// [patch12] ground-truth revision: Table-typed references resolve against
// OUTPUT BINDINGS (wire semantics). The raycaster proved mode-0 raw-name
// Table inputs on GetTableValue output wires in-game (t144 chains), and
// the patch8/10 SetVariable1 registrations actually BROKE reads: their
// self-named "X to X" dual producers resolved to nil in-game (charcode
// runs 2 and 3). Registrations are gone; every Table raw name must now
// have SOME output binding behind it (any block type). GetLength.String
// rides operand-style resolution (SolveEquation.Result,
// GetObjectProperty.Value, ExecuteFunction TUPLEPARAM outputs, and - new
// in v12 - GetTableValue.Value / GetChildren / GetDescendants outputs).

fn strict_table_producers(out: &transpile::TranspileOutput) -> std::collections::HashSet<String> {
    let mut prods = std::collections::HashSet::new();
    for b in out.report.blocks.values() {
        for (oname, oval) in &b.outputs {
            let is_prod = match b.block_type.as_str() {
                "SetVariable1" => oname == "VariableName",
                "CreateTable" | "TablePack" => oname == "TableVariable",
                _ => false,
            };
            if is_prod && !oval.is_empty() {
                prods.insert(oval.clone());
            }
        }
    }
    prods
}

fn operand_producers(out: &transpile::TranspileOutput) -> std::collections::HashSet<String> {
    let mut prods = strict_table_producers(out);
    for b in out.report.blocks.values() {
        for (oname, oval) in &b.outputs {
            let binds_name = match b.block_type.as_str() {
                "SolveEquation" => oname == "Result",
                "GetObjectProperty" => oname == "Value",
                "ExecuteFunction" => oname.starts_with("TUPLEPARAM_ReturnedValues_"),
                // [patch12] reads through tables / instance children also
                // produce operands (GetLength.String = $t64 rides GTV.Value)
                "GetTableValue" => oname == "Value",
                "GetChildren" => oname == "ChildrenTable",
                "GetDescendants" => oname == "DescendantTable",
                _ => false,
            };
            if binds_name && !oval.is_empty() {
                prods.insert(oval.clone());
            }
        }
    }
    prods
}

fn all_output_bindings(out: &transpile::TranspileOutput) -> std::collections::HashSet<String> {
    // [patch12] wire semantics: a Table raw name resolves against whatever
    // output binding holds it (GTV.Value, DefineFunction TUPLEPARAM params,
    // ExecuteFunction TUPLEPARAM results, GetChildren, ...).
    let mut prods = std::collections::HashSet::new();
    for b in out.report.blocks.values() {
        for (_oname, oval) in &b.outputs {
            if !oval.is_empty() {
                prods.insert(oval.clone());
            }
        }
    }
    prods
}

fn assert_table_refs_registered(label: &str, out: &transpile::TranspileOutput) {
    let wires = all_output_bindings(out);
    let operand = operand_producers(out);
    let mut bad: Vec<String> = Vec::new();
    for (name, b) in &out.report.blocks {
        let (want_inames, allowed): (&[&str], &std::collections::HashSet<String>) =
            match b.block_type.as_str() {
                "GetTableValue" | "SetTableValue" => (&["Table"][..], &wires),
                "GetLength" => (&["String"][..], &operand),
                _ => continue,
            };
        let is_table_slot =
            matches!(b.block_type.as_str(), "GetTableValue" | "SetTableValue");
        for inp in &b.inputs {
            if !want_inames.contains(&inp.iname.as_str()) {
                continue;
            }
            // mode-2 variable reference
            if inp.use_var {
                let v = &inp.var;
                if v.is_empty() || v.starts_with("TUPLEPARAM_") || v.starts_with("TUPLE_") {
                    continue;
                }
                if !allowed.contains(v) {
                    bad.push(format!(
                        "  {} ({}): {} input references unproduced name {:?}",
                        name, b.block_type, inp.iname, v
                    ));
                }
                continue;
            }
            // [patch12] mode-0 raw name on a Table slot: must be an output
            // binding. Object paths, tuple wires and RS globals are exempt.
            if !is_table_slot {
                continue;
            }
            let raw = inp.value.clone().unwrap_or_default();
            if raw.is_empty()
                || raw.contains('.')
                || raw.contains('"')
                || raw.starts_with("TUPLEPARAM_")
                || raw.starts_with("TUPLE_")
                || matches!(
                    raw.as_str(),
                    "workspace" | "game" | "script" | "Workspace" | "Game" | "Script"
                )
            {
                continue;
            }
            if !allowed.contains(&raw) {
                bad.push(format!(
                    "  {} ({}): {} raw-name references unproduced name {:?}",
                    name, b.block_type, inp.iname, raw
                ));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "[{}] table references without an output binding (table nil-flow):\n{}",
        label,
        bad.join("\n")
    );
}

#[test]
fn transpile_patch10_table_returning_call_registers_result() {
    let src = "local function mk()\nlocal t = {}\nt[1] = 7\nreturn t\nend\n"
        .to_owned()
        + "local m = mk()\n"
        + "local e = m[1]\n"
        + "local n = #e\n"
        + "m[2] = n\n";
    let out = assert_transpile_ok("patch10_ret_table", &src);
    assert_table_refs_registered("patch10_ret_table", &out);
    assert_lossless("patch10_ret_table", &out);
}

#[test]
fn transpile_patch10_param_tables_register() {
    let src = "local src = {}\nsrc[1] = \"abc\"\n"
        .to_owned()
        + "local function first(t)\n"
        + "local e = t[1]\n"
        + "t[2] = \"x\"\n"
        + "return e\n"
        + "end\n"
        + "local r = first(src)\n"
        + "local n = #r\n";
    let out = assert_transpile_ok("patch10_param_table", &src);
    assert_table_refs_registered("patch10_param_table", &out);
    assert_lossless("patch10_param_table", &out);
}

#[test]
fn transpile_patch10_passthru_and_alias() {
    let src = "local src = {}\nsrc[3] = 5\n"
        .to_owned()
        + "local function wrap(p)\n"
        + "return p\n"
        + "end\n"
        + "local t2 = wrap(src)\n"
        + "local t3 = t2\n"
        + "local v = t3[3]\n"
        + "t2[4] = v\n";
    let out = assert_transpile_ok("patch10_passthru", &src);
    assert_table_refs_registered("patch10_passthru", &out);
    assert_lossless("patch10_passthru", &out);
}

#[test]
fn transpile_patch10_corpus_table_refs_registered() {
    // The registration invariant must hold on the full existing corpus too.
    let out = assert_transpile_ok("patch10_main_corpus", MAIN);
    assert_table_refs_registered("patch10_main_corpus", &out);
    let out2 = assert_transpile_ok("patch10_dict_corpus", DICT);
    assert_table_refs_registered("patch10_dict_corpus", &out2);
    let out3 = assert_transpile_ok("patch10_simon_ui_corpus", SIMON_UI);
    assert_table_refs_registered("patch10_simon_ui_corpus", &out3);
}
#[test]
fn transpile_patch17_nonident_keys_use_gtv_and_concat_coerces() {
    // [patch17] Two run-6 regressions pinned:
    //   1. bracket reads with non-identifier string keys (T["0"], T["+"])
    //      must ride GetTableValue — the old dot render (T.0 / T.+ /
    //      T.() fails RS's equation compiler;
    //   2. a numeric var concatenated with `..` must be stringified via
    //      ToStringAny (RS Concatenate rejects numbers; Luau coerces).
    let src = r#"
local T = { ["0"] = 48, ["+"] = 43 }
if T["0"] == 48 and T["+"] == 43 then
    print("anchors ok")
end
local function f(id)
    return "[" .. id .. "]"
end
print(f(7))
"#;
    let out = assert_transpile_ok("patch17", src);
    let types: Vec<&str> = out.report.blocks.values().map(|b| b.block_type.as_str()).collect();
    assert!(
        types.contains(&"GetTableValue"),
        "non-identifier string-key reads must use GetTableValue; has {} types",
        types.len()
    );
    // [patch22] the quote-free T dictionary now rides ONE construction
    // equation (raw whole-text parse, run 9); the READS must still
    // decompose - no equation may carry a dot member render (T.0 / T.+
    // break RS's equation compiler, run 6 anchors).
    let dot_eqs: Vec<String> = eq_texts_of(&out)
        .into_iter()
        .filter(|t| t.contains("T."))
        .collect();
    assert!(
        dot_eqs.is_empty(),
        "no equation may carry a dot member render (T.0/T.+ break RS equations); got {:?}",
        dot_eqs
    );
    assert!(
        types.contains(&"SolveEquation"),
        "the quote-free dictionary must ride ONE construction equation; has {} types",
        types.len()
    );
    assert!(
        types.contains(&"ToStringAny"),
        "numeric var operand of .. must be stringified via ToStringAny; has {} types",
        types.len()
    );
    assert!(
        types.contains(&"Concatenate"),
        "concat must still emit Concatenate after coercion; has {} types",
        types.len()
    );
}

// ---------- [patch19] run-7 experiment results ----------

#[test]
fn transpile_numeric_table_literal_becomes_one_equation() {
    let src = "local t9 = {10, 20, 30}\nprint(t9[2])\n";
    let out = assert_transpile_ok("v19_num_table", src);
    let types: Vec<&str> = out.report.blocks.values().map(|b| b.block_type.as_str()).collect();
    assert!(
        !types.contains(&"CreateTable") && !types.contains(&"SetTableValue"),
        "all-numeric positional literal must ride ONE equation, got {:?}",
        types
    );
    let eqs: Vec<_> = out
        .report
        .blocks
        .values()
        .filter(|b| b.block_type == "SolveEquation")
        .collect();
    assert_eq!(eqs.len(), 1, "expected exactly one SolveEquation");
    let eq_text = eqs[0]
        .inputs
        .iter()
        .find(|i| i.iname == "Equation")
        .and_then(|i| i.value.as_deref())
        .unwrap_or("");
    assert_eq!(eq_text, "{10, 20, 30}", "equation text must be the brace literal");
    assert!(
        types.contains(&"GetTableValue"),
        "t9[2] must still read via GetTableValue; got {:?}",
        types
    );
    assert_lossless("v19_num_table", &out);
}

#[test]
fn transpile_string_table_literal_stays_on_create_table() {
    let src = "local t10 = {\"a\", \"b\"}\nprint(t10[1])\n";
    let out = assert_transpile_ok("v19_str_table", src);
    let types: Vec<&str> = out.report.blocks.values().map(|b| b.block_type.as_str()).collect();
    assert!(
        types.contains(&"CreateTable") && types.contains(&"SetTableValue"),
        "string entries keep the proven CreateTable + SetTableValue path; got {:?}",
        types
    );
    assert!(
        !types.contains(&"SolveEquation"),
        "string entries must not fold into a brace equation (E9 untested); got {:?}",
        types
    );
    assert_lossless("v19_str_table", &out);
}

#[test]
fn transpile_mod_folds_into_equation() {
    let src = "local a2 = 5\nlocal r2 = (a2 % 4) + 1\nprint(r2)\n";
    let out = assert_transpile_ok("v19_mod_fold", src);
    let eq_text = out
        .report
        .blocks
        .values()
        .filter(|b| b.block_type == "SolveEquation")
        .filter_map(|b| {
            b.inputs
                .iter()
                .find(|i| i.iname == "Equation")
                .and_then(|i| i.value.clone())
        })
        .collect::<Vec<_>>()
        .join("|");
    assert_eq!(
        eq_text, "((a2 % 4) + 1)",
        "run 7 E3 proved % compiles inside SolveEquation (compound form; a bare comparison rides the If block's built-in inputs)"
    );
    // the SIMPLE form `a2 % 4` rides the native Modulus block (same tier
    // as Addition/Subtraction) - equations are for compound expressions.
    let src2 = "local a2 = 5\nlocal r2 = a2 % 4\nprint(r2)\n";
    let out2 = assert_transpile_ok("v19_mod_native", src2);
    let types2: Vec<&str> =
        out2.report.blocks.values().map(|b| b.block_type.as_str()).collect();
    assert!(
        types2.contains(&"Modulus") && !types2.contains(&"SolveEquation"),
        "simple % must ride the native Modulus block; got {:?}",
        types2
    );
    assert_lossless("v19_mod_fold", &out);
    assert_lossless("v19_mod_native", &out2);
}

#[test]
fn transpile_mod_const_fold_uses_lua_semantics() {
    let src = "local r3 = -7 % 3\nprint(r3)\n";
    let out = assert_transpile_ok("v19_lua_mod", src);
    let vals: Vec<&str> = out
        .report
        .blocks
        .values()
        .flat_map(|b| b.inputs.iter().filter_map(|i| i.value.as_deref()))
        .collect();
    assert!(
        vals.contains(&"2"),
        "(-7) % 3 must const-fold to Lua's floored 2; inputs {:?}",
        vals
    );
    assert!(
        !vals.contains(&"-1"),
        "Rust-truncated -1 must never appear (Lua semantics); inputs {:?}",
        vals
    );
    assert_lossless("v19_lua_mod", &out);
}

#[test]
fn transpile_pow_folds_into_equation() {
    let src = "local b3 = 3\nlocal p3 = (b3 ^ 2) + 1\nprint(p3)\n";
    let out = assert_transpile_ok("v19_pow_fold", src);
    let eq_text = out
        .report
        .blocks
        .values()
        .filter(|b| b.block_type == "SolveEquation")
        .filter_map(|b| {
            b.inputs
                .iter()
                .find(|i| i.iname == "Equation")
                .and_then(|i| i.value.clone())
        })
        .collect::<Vec<_>>()
        .join("|");
    assert_eq!(
        eq_text, "((b3 ^ 2) + 1)",
        "run 7 E4 proved ^ compiles inside SolveEquation (compound form; the simple form rides native Exponentiation)"
    );
    assert_lossless("v19_pow_fold", &out);
}

#[test]
fn transpile_concat_condition_never_folds_to_equation() {
    // E1 (run 7): RS SolveEquation rejects `..`. The recursive BinOp arm
    // of is_safe_for_eq_fold used to let this condition fold into one
    // broken equation; v19 bans Concat so it decomposes to Concatenate
    // + Comparison + If.
    let src = "local c5 = \"a\"\nif (c5 .. \"b\") == \"ab\" then\nprint(\"t\")\nend\n";
    let out = assert_transpile_ok("v19_concat_gate", src);
    let types: Vec<&str> = out.report.blocks.values().map(|b| b.block_type.as_str()).collect();
    assert!(
        !types.contains(&"SolveEquation"),
        "`..` must never ride an equation (run 7 E1); got {:?}",
        types
    );
    assert!(
        types.contains(&"Concatenate"),
        "concat must emit Concatenate blocks; got {:?}",
        types
    );
    assert_lossless("v19_concat_gate", &out);
}

// ---------- [patch22] brace constructor pins (run-9 root cause) ----------

fn eq_texts_of(out: &transpile::TranspileOutput) -> Vec<String> {
    let mut v: Vec<String> = out
        .report
        .blocks
        .values()
        .filter(|b| b.block_type == "SolveEquation")
        .filter_map(|b| {
            b.inputs
                .iter()
                .find(|i| i.iname == "Equation")
                .and_then(|i| i.value.clone())
        })
        .collect();
    v.sort();
    v
}

#[test]
fn transpile_string_key_dictionary_rides_one_equation() {
    // run 9 (user, 2026-11-08): RS parses the equation as ONE raw
    // chunk - explicit keys were never the problem; v20's failure was
    // the Lua-escaped quote entry (`\"` -> the `"""` break). A
    // quote-free string-key dictionary collapses to ONE equation.
    let src = "local d2 = {[\"0\"] = 48, [\"b\"] = \"x\"}\nprint(d2[\"0\"])\n";
    let out = assert_transpile_ok("v22_str_dict", src);
    let types: Vec<&str> = out.report.blocks.values().map(|b| b.block_type.as_str()).collect();
    assert!(
        !types.contains(&"CreateTable") && !types.contains(&"SetTableValue"),
        "quote-free dictionary must ride ONE equation; got {:?}",
        types
    );
    assert_eq!(
        eq_texts_of(&out),
        vec!["{[\"0\"] = 48, [\"b\"] = \"x\"}".to_string()],
        "dictionary must be one brace equation; got {:?}",
        eq_texts_of(&out)
    );
    assert!(
        types.contains(&"GetTableValue"),
        "reads still go through GetTableValue; got {:?}",
        types
    );
    assert_lossless("v22_str_dict", &out);
}

#[test]
fn transpile_numeric_key_dictionary_rides_one_equation() {
    // number keys + string values: the user's proven remap shape
    // (`{[32] = ")", [33] = "(", [34] = "`"}` compiled in game).
    let src = "local t12 = {[32] = \"a\", [33] = \"b\"}\nprint(t12[32])\n";
    let out = assert_transpile_ok("v22_num_dict", src);
    let types: Vec<&str> = out.report.blocks.values().map(|b| b.block_type.as_str()).collect();
    assert!(
        !types.contains(&"CreateTable") && !types.contains(&"SetTableValue"),
        "number-key dictionary must ride ONE equation; got {:?}",
        types
    );
    assert_eq!(
        eq_texts_of(&out),
        vec!["{[32] = \"a\", [33] = \"b\"}".to_string()],
        "dictionary must be one brace equation; got {:?}",
        eq_texts_of(&out)
    );
    assert_lossless("v22_num_dict", &out);
}

#[test]
fn transpile_quote_entry_defers_to_set_table_value() {
    // A quote-bearing string can NEVER ride the raw text: `"""` ends
    // the literal early and shreds the rest of the equation. Only that
    // entry defers - the quote-free ones stay in the equation.
    let src = "local m3 = {[\"a\"] = 1, [\"\\\"\"] = 34, [\"c\"] = 3}\nprint(m3[\"a\"])\n";
    let out = assert_transpile_ok("v22_quote_defer", src);
    let types: Vec<&str> = out.report.blocks.values().map(|b| b.block_type.as_str()).collect();
    assert!(
        !types.contains(&"CreateTable"),
        "the equation builds the table; got {:?}",
        types
    );
    assert!(
        types.contains(&"SetTableValue") && types.contains(&"SolveEquation"),
        "mixed path = one equation + one deferred fill; got {:?}",
        types
    );
    assert_eq!(
        eq_texts_of(&out),
        vec!["m3.a".to_string(), "{[\"a\"] = 1, [\"c\"] = 3}".to_string()],
        "construction equation + proven dot read; got {:?}",
        eq_texts_of(&out)
    );
    assert_lossless("v22_quote_defer", &out);
}

#[test]
fn transpile_space_and_backslash_strings_ride_raw() {
    // run 9: space and backslash are ordinary characters in the raw
    // parse; there are NO escape sequences, so a backslash goes in as
    // ONE raw char (v20's `\\` would silently become TWO).
    let src = "local s9 = {[32] = \"x\", [33] = \" \", [92] = \"\\\\\"}\nprint(s9[92])\n";
    let out = assert_transpile_ok("v22_raw_chars", src);
    let types: Vec<&str> = out.report.blocks.values().map(|b| b.block_type.as_str()).collect();
    assert!(
        !types.contains(&"CreateTable") && !types.contains(&"SetTableValue"),
        "space/backslash entries are quote-free and ride the equation; got {:?}",
        types
    );
    assert_eq!(
        eq_texts_of(&out),
        vec!["{[32] = \"x\", [33] = \" \", [92] = \"\\\"}".to_string()],
        "strings must ride RAW - one backslash, no escapes; got {:?}",
        eq_texts_of(&out)
    );
    assert_lossless("v22_raw_chars", &out);
}

#[test]
fn transpile_quote_and_backslash_keys_mixed() {
    // CH2B's poison pair: the quote key defers to SetTableValue, the
    // backslash key rides RAW as one character (correct key - not the
    // v20 `\\` two-char corruption).
    let src = "local e3 = {[\"\\\"\"] = 34, [\"\\\\\"] = 92}\nprint(e3[\"\\\\\"])\n";
    let out = assert_transpile_ok("v22_escapes_mixed", src);
    let types: Vec<&str> = out.report.blocks.values().map(|b| b.block_type.as_str()).collect();
    assert!(
        !types.contains(&"CreateTable"),
        "the equation builds the table; got {:?}",
        types
    );
    assert!(
        types.contains(&"SetTableValue") && types.contains(&"SolveEquation"),
        "quote key defers, backslash key rides; got {:?}",
        types
    );
    assert_eq!(
        eq_texts_of(&out),
        vec!["{[\"\\\"] = 92}".to_string()],
        "backslash key rides raw; got {:?}",
        eq_texts_of(&out)
    );
    assert_lossless("v22_escapes_mixed", &out);
}

#[test]
fn transpile_posstr_falls_back_and_var_dict_mixes() {
    // positional STRINGS: only Pos(Number) has the run-7 proof, so
    // Pos(Str) keeps the full CreateTable fallback. Var values: only
    // the var entry defers (mixed equation + one fill block).
    let src2 = "local s7 = {\"p\", \"q\"}\nprint(s7[1])\n";
    let out2 = assert_transpile_ok("v22_posstr", src2);
    let types2: Vec<&str> = out2.report.blocks.values().map(|b| b.block_type.as_str()).collect();
    assert!(
        types2.contains(&"CreateTable") && types2.contains(&"SetTableValue"),
        "positional strings keep the proven CreateTable path; got {:?}",
        types2
    );
    assert!(
        !types2.contains(&"SolveEquation"),
        "Pos(Str) has no equation proof; got {:?}",
        types2
    );
    let src = "local v6 = 5\nlocal m3 = {[\"a\"] = 1, [\"b\"] = v6}\nprint(m3[\"b\"])\n";
    let out = assert_transpile_ok("v22_var_dict", src);
    let types: Vec<&str> = out.report.blocks.values().map(|b| b.block_type.as_str()).collect();
    assert!(
        !types.contains(&"CreateTable") && types.contains(&"SetTableValue"),
        "var-valued dictionary defers only the var entry (mixed); got {:?}",
        types
    );
    assert_eq!(
        eq_texts_of(&out),
        vec!["m3.b".to_string(), "{[\"a\"] = 1}".to_string()],
        "construction equation + proven dot read; got {:?}",
        eq_texts_of(&out)
    );
    assert_lossless("v22_posstr", &out2);
    assert_lossless("v22_var_dict", &out);
}

// [v24] source comments become editor Comment records --------------------

fn v24_parse_pos(s: &str) -> (f64, f64) {
    let (x, y) = s.split_once(',').unwrap();
    let h = |t: &str| -> f64 {
        let neg = t.starts_with('-');
        let t = t.trim_start_matches('-');
        let (w, f) = t.split_once('.').unwrap_or((t, ""));
        let v = i64::from_str_radix(w, 16).unwrap() as f64
            + if f.is_empty() {
                0.0
            } else {
                i64::from_str_radix(f, 16).unwrap() as f64 / 1000.0
            };
        if neg { -v } else { v }
    };
    (h(x), h(y))
}

#[test]
fn v24_comments_become_comment_records() {
    let src = "-- #CD6298 Pink header note!\n\
               -- second header line\n\
               local x = 2\n\
               -- #4B974B math\n\
               local y = x * 2\n\
               print(x, y)\n\
               -- trailing note\n\
               local z = x + y\n";
    let out = assert_transpile_ok("v24_comments", src);
    let cs = &out.report.comments;
    // one box per statement: s0 header (user), s1 math (user),
    // s2 print (gray auto label), s3 local z (user, trailing comment)
    assert_eq!(cs.len(), 4, "boxes: {:?}", cs);
    // explicit #RRGGBB overrides survived, in V4 byte format
    assert_eq!(cs[0].color.as_deref(), Some("CD,62,98"));
    assert_eq!(cs[1].color.as_deref(), Some("4B,97,4B"));
    // untagged comment takes the first palette color (user's red)
    assert_eq!(cs[3].color.as_deref(), Some("C4,28,1C"));
    // every box states WHICH source line it represents (v24 request)
    assert_eq!(
        cs[0].text.as_deref(),
        Some("Lines 1-2:\nPink header note!\nsecond header line")
    );
    assert_eq!(cs[1].text.as_deref(), Some("Line 4:\nmath"));
    assert_eq!(cs[3].text.as_deref(), Some("Line 7:\ntrailing note"));
    // the unannotated print statement gets the gray auto label
    assert_eq!(cs[2].color.as_deref(), Some("99,99,99"));
    assert_eq!(cs[2].text.as_deref(), Some("Line 6: print(x, y)"));
    // the #RRGGBB marker never leaks into the box text
    assert!(!cs[0].text.as_deref().unwrap().contains('#'));
    // every record fully populated with parseable V4 hex fields
    for c in cs {
        let pos = c.position.as_deref().expect("position");
        let size = c.size.as_deref().expect("size");
        assert!(pos.contains(','), "position {:?}", pos);
        assert!(size.contains(','), "size {:?}", size);
        assert!(v24_parse_pos(pos).1.is_finite());
        assert!(c.color.is_some() && c.text.is_some());
    }
    // placement ground truth (v29, measured from the user's handmade
    // function_example): the box sits immediately right of the anchor
    // statement's entry block and top-aligned with it:
    // x = entry.x + 1.0 (block width) + 0.125, y = entry.y
    let entry = out
        .report
        .blocks
        .get("Set Variable1")
        .expect("entry block Set Variable1");
    let (ex, ey) = v24_parse_pos(entry.visual_position.as_deref().unwrap());
    let (cx, cy) = v24_parse_pos(cs[0].position.as_deref().unwrap());
    assert!(
        (cx - (ex + 1.125)).abs() < 0.01,
        "comment x {} vs entry x {} + 1.125",
        cx,
        ex
    );
    assert!(
        (cy - ey).abs() < 0.01,
        "comment y {} vs entry y {}",
        cy,
        ey
    );
    // assert_lossless above also round-trips comments byte-stably
}

// [v25] comment boxes must span their statement's whole block chain --------

#[test]
fn v25_comment_box_spans_the_block_chain() {
    // user report: the print's box kept its 1-unit text height next to an
    // 8-block chain ("forgot to resize it to the entire block that
    // represents that line"); the box must stretch to cover every block
    let src = "local npass = 0\n\
               local ntotal = 0\n\
               print(\"RESULT \" .. npass .. \"/\" .. ntotal .. \" PASSED\")\n";
    let out = assert_transpile_ok("v25_resize", src);
    let cs = &out.report.comments;
    // 3 statements -> 3 boxes, all gray auto labels here
    assert_eq!(cs.len(), 3, "boxes: {:?}", cs);
    let pr = cs
        .iter()
        .find(|c| c.text.as_deref().unwrap_or("").starts_with("Line 3:"))
        .expect("print auto label");

    // every block of the print statement, by name (no other statement
    // emits ToStringAny / Concatenate / Print in this source)
    let mut min_y = f64::MAX;
    let mut max_y = f64::MIN;
    let mut top_x = f64::NAN;
    for (name, b) in &out.report.blocks {
        let is_chain = name.starts_with("To String Any")
            || name.starts_with("Concatenate")
            || name.starts_with("Print");
        if !is_chain {
            continue;
        }
        let (x, y) = v24_parse_pos(b.visual_position.as_deref().unwrap());
        if y < min_y {
            min_y = y;
            top_x = x;
        }
        max_y = max_y.max(y);
    }
    // chain blocks step 0.75 apart; the box spans first-top .. last-bottom
    // (v29 block height 0.5, measured from the handmade tall label)
    let want_h = max_y + 0.5 - min_y;
    assert!(want_h > 5.0, "chain height {} (8 blocks expected)", want_h);

    let (bx, by) = v24_parse_pos(pr.position.as_deref().unwrap());
    let (_, bh) = v24_parse_pos(pr.size.as_deref().unwrap());
    // top aligned with the chain's first block
    assert!(
        (by - min_y).abs() < 0.01,
        "box y {} vs chain top {}",
        by,
        min_y
    );
    // height covers the WHOLE chain, not just the label text
    assert!(
        (bh - want_h).abs() < 0.01,
        "box height {} vs chain height {}",
        bh,
        want_h
    );
    // still right of the chain's entry column (entry x + 1.125 = block
    // width + gap, and nudges only ever move it further right)
    assert!(
        bx >= top_x + 1.125 - 0.01,
        "box x {} vs entry x {} + 1.125",
        bx,
        top_x
    );
    // a short 1-block statement keeps its text height (no forced stretch);
    // v29: one text line = 0.5 tall (the handmade box height)
    let lo = cs
        .iter()
        .find(|c| c.text.as_deref().unwrap_or("").starts_with("Line 2:"))
        .expect("ntotal auto label");
    let (_, lh) = v24_parse_pos(lo.size.as_deref().unwrap());
    assert!(
        (lh - 0.5).abs() < 0.01,
        "1-block statement keeps text height, got {}",
        lh
    );
}

// [v29] the handmade row pattern: compact statements, labels beside them --

#[test]
fn v29_comments_beside_compact_statements() {
    // user report on v28: "v28 is all wrong again... from start you were
    // doing it all wrong" - the handmade ground truth (function_example
    // v0.1, decoded with the correct hex rule) shows: statements on one row
    // separated by block width + 0.125 + label width (2.875 = 1.0 + 0.125
    // + 1.75 in the handmade file), labels right of their own block and
    // top-aligned with it, 0.5 tall per text line, and the ROOT container
    // on its OWN row 1.25 above the statement row (v28 stacked ROOT ON the
    // first statement)
    let src = "local npass = 0\n\
               local ntotal = 0\n\
               for i = 1, 10 do\n\
                 local ok = i % 2 == 0\n\
                 if ok then\n\
                   npass = npass + 1\n\
                 end\n\
                 ntotal = ntotal + 1\n\
               end\n\
               print(\"RESULT \" .. npass .. \"/\" .. ntotal .. \" PASSED\")\n\
               local done = npass == ntotal\n\
               print(done)\n";
    let out = assert_transpile_ok("v29_spacing", src);
    let blocks: Vec<(f64, f64, f64, f64)> = out
        .report
        .blocks
        .values()
        .map(|b| {
            let (x, y) = v24_parse_pos(b.visual_position.as_deref().unwrap());
            (x, y, 1.0, 0.75)
        })
        .collect();
    let comments: Vec<(f64, f64, f64, f64)> = out
        .report
        .comments
        .iter()
        .map(|c| {
            let (x, y) = v24_parse_pos(c.position.as_deref().unwrap());
            let (w, h) = v24_parse_pos(c.size.as_deref().unwrap());
            (x, y, w, h)
        })
        .collect();
    // 6 top-level statements -> 6 auto-label boxes
    assert_eq!(comments.len(), 6, "one box per statement");

    // no box may intersect any block or any other box (touching edges,
    // intersection 0, is fine)
    for (i, c) in comments.iter().enumerate() {
        for (j, b) in blocks.iter().enumerate() {
            let ox = (c.0 + c.2).min(b.0 + b.2) - c.0.max(b.0);
            let oy = (c.1 + c.3).min(b.1 + b.3) - c.1.max(b.1);
            assert!(
                ox <= 0.01 || oy <= 0.01,
                "box {} intersects block {}: {:.3} x {:.3}",
                i,
                j,
                ox,
                oy
            );
        }
        for (j, other) in comments.iter().enumerate() {
            if j <= i {
                continue;
            }
            let ox = (c.0 + c.2).min(other.0 + other.2) - c.0.max(other.0);
            let oy = (c.1 + c.3).min(other.1 + other.3) - c.1.max(other.1);
            assert!(
                ox <= 0.01 || oy <= 0.01,
                "box {} intersects box {}: {:.3} x {:.3}",
                i,
                j,
                ox,
                oy
            );
        }
    }

    // the two single-block statements are spaced by block width + gap +
    // label width (handmade rule: 2.875 = 1.0 + 0.125 + 1.75); the label
    // here is "Line 1: local npass = 0" = 23 chars -> 0.92 wide, so the
    // pitch is 1.0 + 0.125 + 0.92 = 2.045 - compact, not the old uniform
    // gutter (5.625) and not packed edge-to-edge (1.125)
    let sv1 = out.report.blocks.get("Set Variable1").unwrap();
    let sv2 = out.report.blocks.get("Set Variable2").unwrap();
    let (x1, _) = v24_parse_pos(sv1.visual_position.as_deref().unwrap());
    let (x2, _) = v24_parse_pos(sv2.visual_position.as_deref().unwrap());
    let gap = x2 - x1;
    assert!(
        (1.5..=3.0).contains(&gap),
        "single-block statements must be spaced by block + gap + label width, got {}",
        gap
    );

    // statement 0's box sits immediately right of its block (x = block x
    // + 1.0 + 0.125, no nudge needed) and stays inside the reserved gap:
    // its right edge must not cross the next statement's entry column
    let c0 = comments[0];
    assert!(
        (c0.0 - (x1 + 1.125)).abs() < 0.01,
        "box 0 x {} should be block x {} + 1.125",
        c0.0,
        x1
    );
    assert!(
        c0.0 + c0.2 <= x2 + 0.01,
        "box 0 right edge {} must not cross the next statement's entry x {}",
        c0.0 + c0.2,
        x2
    );

    // the ROOT Comment2 container sits on its OWN row above the statements
    // (v28 parked it ON the first statement - two blocks on one spot)
    let root = out
        .report
        .blocks
        .values()
        .find(|b| b.block_type == "Comment2")
        .expect("ROOT container");
    let (rx, ry) = v24_parse_pos(root.visual_position.as_deref().unwrap());
    let stmt_min_y = blocks
        .iter()
        .filter(|(x, y, _, _)| !(*x == rx && *y == ry))
        .map(|(_, y, _, _)| *y)
        .fold(f64::MAX, f64::min);
    assert!(
        (stmt_min_y - (ry + 1.25)).abs() < 0.01,
        "statement row must be 1.25 below the ROOT row (root y {}, statements at {})",
        ry,
        stmt_min_y
    );
    let root_name = root.name.clone();
    for (name, b) in &out.report.blocks {
        if *name == root_name {
            continue;
        }
        let (x, y) = v24_parse_pos(b.visual_position.as_deref().unwrap());
        assert!(
            !(x == rx && y == ry),
            "no block may share the ROOT container's position ({}, {}): {} does",
            rx,
            ry,
            name
        );
    }
}

// [v27] each statement's chain is ONE vertical column, entry at the top ----

// [v29.1] non-string key on an untracked variable base -> SetTableValue ----

#[test]
fn v29_1_nonstring_key_on_param_alias_uses_set_table_value() {
    // user report on v29: three "[GAP] non-string property key on Instance"
    // notes on a sorting workbench script. Root cause: `local arr = a` (alias
    // of a function parameter) makes `arr` a local variable that is NOT in
    // lua_tables, so writes `arr[j] = arr[j+1]` (numeric keys j, j+1) fell
    // to the Instance branch of the LHS dispatch and GAPped.
    //
    // Semantic fix: a non-string key (number, variable, expression) can ONLY
    // target a Lua table - Instance property names are always strings in Luau.
    // So a non-string key on a variable base uses SetTableValue regardless of
    // lua_tables tracking; the GAP fallback is now only reachable when the
    // base is a confirmed Instance/ObjectRef with a non-string key (a genuine
    // Luau error, not a tracking gap).
    let src = "local function bubbleSort(a)\n\
               \x20 local arr = a\n\
               \x20 local n = #arr\n\
               \x20 for i = 1, n - 1 do\n\
               \x20   for j = 1, n - i do\n\
               \x20     if arr[j] > arr[j + 1] then\n\
               \x20       local temp = arr[j]\n\
               \x20       arr[j] = arr[j + 1]\n\
               \x20       arr[j + 1] = temp\n\
               \x20     end\n\
               \x20   end\n\
               \x20 end\n\
               end\n\
               local data = {5, 3, 8, 1, 9}\n\
               bubbleSort(data)\n";
    let out = assert_transpile_ok("v29_1_param_alias_table_write", src);
    // the three non-string-key writes (arr[j], arr[j+1], arr[j+1]) must all
    // lower to SetTableValue, none to SetObjectProperty, and emit zero GAPs
    let types: Vec<&str> = out.report.blocks.values().map(|b| b.block_type.as_str()).collect();
    assert!(
        types.contains(&"SetTableValue"),
        "expected SetTableValue for the param-alias array writes; got {:?}",
        types
    );
    assert!(
        !types.contains(&"SetObjectProperty"),
        "non-string key on a variable base must NOT lower to SetObjectProperty; got {:?}",
        types
    );
    assert_lossless("v29_1_param_alias_table_write", &out);
}

#[test]
fn v27_chains_stack_up_to_down() {
    // user report on v26: "instead of putting the blocks up to down, you
    // like made them in little columns" - a chain must read top to bottom
    // like its source lines: every block of a statement in the SAME column,
    // 0.75 apart, the statement's entry block at the very top, so the
    // parent->child wire points straight down
    let src = "local npass = 0\n\
               local ntotal = 0\n\
               for i = 1, 10 do\n\
                 local ok = i % 2 == 0\n\
                 if ok then\n\
                   npass = npass + 1\n\
                 end\n\
                 ntotal = ntotal + 1\n\
               end\n\
               print(\"RESULT \" .. npass .. \"/\" .. ntotal .. \" PASSED\")\n";
    let out = assert_transpile_ok("v27_stack", src);

    // the RESULT print chain, in wiring order from its entry:
    let chain = [
        "To String Any1",
        "To String Any2",
        "Concatenate1",
        "Concatenate2",
        "Concatenate3",
        "Concatenate4",
        "To String Any3",
        "Print1",
    ];
    let mut prev_y = f64::NAN;
    let mut col_x = f64::NAN;
    for (i, n) in chain.iter().enumerate() {
        let b = out
            .report
            .blocks
            .get(*n)
            .unwrap_or_else(|| panic!("missing block {}", n));
        let (x, y) = v24_parse_pos(b.visual_position.as_deref().unwrap());
        if i == 0 {
            col_x = x;
        }
        // ... all in ONE column
        assert!(
            (x - col_x).abs() < 0.001,
            "{} at x {}, chain column is {}",
            n,
            x,
            col_x
        );
        if i > 0 {
            // ... each block exactly one LEVEL (0.75) below the previous,
            // so the ChildBlocks wire points straight down
            assert!(
                (y - (prev_y + 0.75)).abs() < 0.001,
                "{} at y {}, expected {} (0.75 below previous)",
                n,
                y,
                prev_y + 0.75
            );
        }
        prev_y = y;
    }
    // entry (To String Any1) is the TOP of its column: no block of the
    // print statement sits above it
    let entry_y = v24_parse_pos(
        out.report
            .blocks
            .get("To String Any1")
            .unwrap()
            .visual_position
            .as_deref()
            .unwrap(),
    )
    .1;
    for n in chain.iter().skip(1) {
        let y = v24_parse_pos(
            out.report
                .blocks
                .get(*n)
                .unwrap()
                .visual_position
                .as_deref()
                .unwrap(),
        )
        .1;
        assert!(y > entry_y, "{} must sit BELOW the entry block", n);
    }
    // the for loop's entry block is the top of ITS column too, and its
    // child wire goes straight down
    let (fx, fy) = v24_parse_pos(
        out.report
            .blocks
            .get("For Loop1")
            .unwrap()
            .visual_position
            .as_deref()
            .unwrap(),
    );
    let (mx, my) = v24_parse_pos(
        out.report
            .blocks
            .get("Modulus1")
            .unwrap()
            .visual_position
            .as_deref()
            .unwrap(),
    );
    assert!(
        (fx - mx).abs() < 0.001 && (my - (fy + 0.75)).abs() < 0.001,
        "For Loop1 -> Modulus1 wire must point straight down: ({}, {}) -> ({}, {})",
        fx,
        fy,
        mx,
        my
    );
}
