// [patched: compound fold returns_bool v1]
// [patched: all-in-one v1]
// [patched: compound fold v1 - And/Or/Not -> single SolveEquation]
// [patched: math alias inlining v1 - math_X(args) -> math.X(args) via SolveEquation]
// [patched: truthy fusion v1 - no body duplication]
// [patched: eq-fold optimization v1 - whole BinOp tree -> single SolveEquation]
// [patched: runtime fixes v1 - Lua-table LHS dispatch + GetService->GetObjectProperty]
// [patched: task.delay Spawn+Parameters v1]
// [patched: not-not -> truthy pattern v1]
//! Block emitter: restricted Luau AST -> VisualSource V4 block forest.
//!
//! Conventions (docs/FORMAT.md, docs/TRANSPILE.md):
//! - Dataflow: a block output binds to a variable name (serialized binding);
//!   inputs reference variables via mode 2. Assignments rebind the producing
//!   block's output, so `x = a + b` costs one block, no SetVariable1.
//! - Control flow: statements append blocks to an ordered chain; parents link
//!   chains via ChildBlocks (else branches via ElseChildBlock).
//! - Tuples: all-literal tuples serialize comma-joined (gist rule); tuples
//!   built from variables go CreateTable + SetTableValue.. + TableUnpack;
//!   tuple ELEMENT access goes TablePack + GetTableValue (1-based).
//! - While-loop conditions with computed operands become a gate variable
//!   recomputed at the end of each body (liveness).

use std::collections::HashMap;

use super::ast::*;
use crate::hex::Hex;
use crate::vs_blocks;

#[derive(Debug, Clone)]
pub enum InputVal {
    /// mode 1: explicit typed literal
    Literal { value: String, vtype: String },
    /// mode 0: implicit (object path references, empty tuples)
    Implicit(String),
    /// mode 2: variable reference
    Var(String),
}

#[derive(Debug, Clone)]
pub struct EmittedBlock {
    pub name: String,
    pub btype: String,
    pub inputs: Vec<(String, InputVal)>,
    /// output name (as serialized, "TUPLE_X" for tuples) -> binding
    pub outputs: Vec<(String, String)>,
    pub children: Vec<String>,
    pub else_child: Option<String>,
}

impl EmittedBlock {
    pub fn is_pure(&self) -> bool {
        !crate::transpile::emit::effectful(&self.btype)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum LitVal {
    Num(f64),
    Bool(bool),
    Str(String),
    Nil,
    ObjectRef(String),
    BrickColor(f64),
    UDim2(f64, f64, f64, f64),
    Vector3(f64, f64, f64),
    Vector2(f64, f64),
    Color3(f64, f64, f64),
}

#[derive(Debug, Clone)]
enum Val {
    Lit(LitVal),
    Path(String),
    Var(String),
    Tuple(Option<String>),
}

#[derive(Debug, Clone)]
pub struct Note {
    pub kind: &'static str, // "VERIFY" | "WORKAROUND" | "INFO"
    pub msg: String,
}

/// Blocks with observable side effects (never dead-code eliminated).
pub fn effectful(btype: &str) -> bool {
    !matches!(
        btype,
        "Addition" | "Subtraction" | "Multiplication" | "Division" | "Modulus"
            | "Exponentiation" | "SolveEquation" | "Comparison" | "ANDAny" | "ORAny" | "NOTAny"
            | "GetObjectProperty" | "FindFirstChildNew" | "FindFirstChildOfClassNew"
            | "FindFirstChildWhichIsANew" | "WaitForChild" | "GetChildren" | "GetDescendants"
            | "GetTableValue" | "GetTableLength" | "TableFind" | "ConstructVector3"
            | "ConstructVector2" | "ConstructUDim2" | "ConstructCFrame" | "ConstructColor3"
            | "Angles" | "RandomNumber" | "Round" | "MathLibrarySingleInput"
            | "MathLibraryDoubleInput" | "OsTime" | "OsClock" | "GetServerTimeNow"
            | "Concatenate" | "Lower" | "Upper" | "Substring" | "GSubString" | "ToStringAny"
            | "ToNumberAny" | "IsA" | "GetPlayerFromCharacter" | "CreateTable" | "TablePack"
            | "TableUnpack" | "TypeOf" | "GetModelBoundingBox" | "Lerp1" | "InverseCFrame1"
            | "ToObjectSpace1" | "ToWorldSpace1" | "DistanceBetweenPoints" | "GetLength"
            | "FindString"
    )
}

pub struct Transpiler {
    pub(crate) blocks: Vec<EmittedBlock>,
    pub(crate) index: HashMap<String, usize>,
    display_counts: HashMap<String, usize>,
    scopes: Vec<HashMap<String, String>>,
    temp_counter: usize,
    notes: Vec<Note>,
    cse_map: HashMap<String, String>,
    pub folded: usize,
    pub cse_hits: usize,
    pending_event_params: Vec<(String, String)>,
    optimize: bool,
    catalog: serde_json::Value,
    nil_ref_binding: Option<String>,
    fn_returns: HashMap<String, usize>,
    /// [patch10] functions PROVEN to return a Lua table (a Return expr that
    /// is a tracked table binding). Call sites use this to register the
    /// bound result as a real variable + lua_tables member: the
    /// ExecuteFunction TUPLEPARAM output is a wire, not a variable, so
    /// Table-typed raw-name reads on it resolve to nil (patch8 ground truth).
    fn_ret_table: std::collections::HashSet<String>,
    /// [patch10] functions whose single return passes a parameter through
    /// 1:1 (`return t`); lets a call site propagate table-ness from its
    /// argument when the body is otherwise opaque.
    fn_ret_passthru: HashMap<String, usize>,
    /// [patch10] bindings of the functions whose bodies are currently being
    /// emitted, so the Return arm can attribute table-ness.
    fn_stack: Vec<String>,
    // RUNTIME BUG FIX: track each DefineFunction's source parameter list by
    // function NAME, so that `:Connect(setupPlayer)` (a named function
    // reference rather than an inline closure) can use setupPlayer's
    // parameter list to bind the event's outputs and pass them through to
    // the function call. Without this, PlayerAdded:Connect(setupPlayer)
    // emitted a PlayerAdded block with NO Player output binding, then
    // called setupPlayer() with no args - so setupPlayer's `player`
    // parameter was nil. That cascaded to "'Player' input of
    // 'CharacterAdded' must be a 'Player' got 'nil'" at runtime.
    function_params: HashMap<String, Vec<String>>,
    pub(crate) unsup_emitted: std::collections::HashSet<String>,
    pub(crate) lua_tables: std::collections::HashSet<String>,
    /// Math-method alias map: binding -> method name. Records
    /// `local math_X = math.<method>` patterns so that calls
    /// `math_X(args)` can be inlined back to `math.<method>(args)`
    /// and routed through SolveEquation (1 block) instead of the
    /// general ExecuteFunction path (2+ blocks). Verified safe
    /// because math.* is pure and SolveEquation resolves math.*
    /// at runtime (G1/G4-proven).
    pub(crate) math_aliases: HashMap<String, String>,
    /// [patch9] true-discard signal: Stmt::ExprStmt sets this before
    /// evaluating a bare call so call_expr emits the T8-proven count-0
    /// shape. EVERYWHERE else (call args, concat/binop operands,
    /// conditions) the result is consumed, so calls must bind
    /// TUPLEPARAM_ReturnedValues_1 - previously bind=None fell through
    /// to count-0 AND returned the unbound temp, producing dangling
    /// references (charcode `$t68..t74` in the charFromByte concat
    /// chain had no producer -> nil concat at runtime).
    exprstmt_discard: bool,
    /// [v23] span recording gate: only the transpile()-driven top-level
    /// stmts() call records block spans; nested (body) calls never do.
    span_record: bool,
    /// [v23] one block-index span per TOP-LEVEL statement (indexed by
    /// statement position - guard-restructured statements share the
    /// consuming span). Lets mod.rs anchor source comments to the
    /// blocks each statement produced.
    pub stmt_spans: Vec<(usize, usize)>,
}

fn hexs(v: f64) -> String {
    Hex::to_hex_string(v)
}

impl Transpiler {
    pub fn new(optimize: bool) -> Self {
        let catalog: serde_json::Value =
            serde_json::from_str(vs_blocks::VS_BLOCKS_JSON).expect("vs_blocks.json parses");
        Self {
            blocks: Vec::new(),
            index: HashMap::new(),
            display_counts: HashMap::new(),
            scopes: vec![HashMap::new()],
            temp_counter: 0,
            notes: Vec::new(),
            cse_map: HashMap::new(),
            folded: 0,
            cse_hits: 0,
            pending_event_params: Vec::new(),
            optimize,
            catalog,
            nil_ref_binding: None,
            fn_returns: HashMap::new(),
            fn_ret_table: std::collections::HashSet::new(),
            fn_ret_passthru: HashMap::new(),
            fn_stack: Vec::new(),
            function_params: HashMap::new(),
            unsup_emitted: std::collections::HashSet::new(),
            lua_tables: std::collections::HashSet::new(),
            math_aliases: HashMap::new(),
            exprstmt_discard: false,
            span_record: false,
            stmt_spans: Vec::new(),
        }
    }

    /// [v23] Block-index span of every top-level statement, indexed by
    /// statement position (call BEFORE finish(), which consumes self).
    pub fn stmt_spans(&self) -> Vec<(usize, usize)> {
        self.stmt_spans.clone()
    }

    pub(crate) fn note(&mut self, kind: &'static str, msg: String) {
        // dedup identical notes
        if !self.notes.iter().any(|n| n.msg == msg) {
            self.notes.push(Note { kind, msg });
        }
    }

    pub fn notes(&self) -> &[Note] {
        &self.notes
    }

    pub(crate) fn temp(&mut self) -> String {
        self.temp_counter += 1;
        format!("t{}", self.temp_counter)
    }

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }
    fn pop_scope(&mut self) {
        self.scopes.pop();
    }
    fn declare(&mut self, lua: &str, binding: String) {
        self.scopes.last_mut().unwrap().insert(lua.to_string(), binding.clone());
        // TARGETED CSE INVALIDATION (Opportunity #4): previously a
        // blanket `cse_map.clear()` that wiped ALL cached property
        // accesses on every new local declaration, defeating cross-
        // statement CSE for property reads. Now we only invalidate
        // entries whose key references the shadowed `lua` name (BinOp
        // CSE keys use lua names via expr_key) or the new `binding`
        // (property-access CSE keys use bindings). For a fresh local
        // declaration, the binding is brand new so no entries match
        // it — only the lua-name check fires (for shadowing).
        self.cse_invalidate(lua, &binding);
    }

    /// Targeted CSE invalidation (Opportunity #4): remove cache entries
    /// whose key references `lua` (a lua name, for BinOp CSE keys
    /// generated by expr_key) or `binding` (for property-access CSE
    /// keys generated by index_expr). A "name segment" is a maximal
    /// run of [A-Za-z0-9_] characters in the key. We never under-
    /// invalidate (every entry referencing the shadowed or reassigned
    /// identifier is removed); may over-invalidate (rare substring
    /// collisions across distinct identifiers), which only costs a
    /// redundant re-emission — never wrong semantics.
    fn cse_invalidate(&mut self, lua: &str, binding: &str) {
        if lua.is_empty() && binding.is_empty() {
            self.cse_map.clear();
            return;
        }
        let to_remove: Vec<String> = self
            .cse_map
            .keys()
            .filter(|k| {
                for seg in k.split(|c: char| !(c.is_alphanumeric() || c == '_')) {
                    if seg == lua || seg == binding {
                        return true;
                    }
                }
                false
            })
            .cloned()
            .collect();
        for k in to_remove {
            self.cse_map.remove(&k);
        }
    }
    fn lookup(&self, lua: &str) -> Option<String> {
        for s in self.scopes.iter().rev() {
            if let Some(b) = s.get(lua) {
                return Some(b.clone());
            }
        }
        None
    }

    pub(crate) fn display_name(&mut self, btype: &str) -> String {
        let base: String = {
            let t = btype.trim_end_matches(|c: char| c.is_ascii_digit());
            let mut out = String::new();
            let mut prev_lower = false;
            for c in t.chars() {
                if c.is_uppercase() {
                    if prev_lower {
                        out.push(' ');
                    }
                    out.push(c);
                    prev_lower = false;
                } else {
                    prev_lower = c.is_lowercase() || c.is_ascii_digit();
                    out.push(c);
                }
            }
            out
        };
        let n = self.display_counts.entry(base.clone()).or_insert(0);
        *n += 1;
        format!("{}{}", base, n)
    }

    /// Creates a block, appends it to `chain`, returns its display name.

    /// Normalizes input modes according to vs_blocks.json:
    /// Only inputs with value_type == "Any" use Mode 1 (Literal with vtype).
    /// All fixed types (String, Number, Bool, Object, etc.) MUST use Mode 0 (Implicit).
        fn normalize_inputs(&self, btype: &str, inputs: Vec<(String, InputVal)>) -> Vec<(String, InputVal)> {
        let entry = match self.catalog.get(btype) {
            Some(e) => e,
            None => return inputs,
        };
        let empty_arr = Vec::new();
        let schema_inputs = entry["Inputs"].as_array().unwrap_or(&empty_arr);

        inputs
            .into_iter()
            .map(|(iname, ival)| {
                // TUPLEPARAM_/TUPLE_ auxiliary inputs carry tuple elements
                // with their own explicit wire form - never normalize them.
                if iname.starts_with("TUPLEPARAM_") || iname.starts_with("TUPLE_") {
                    return (iname, ival);
                }
                let schema_type = schema_inputs
                    .iter()
                    .find(|si| si["name"].as_str() == Some(&iname))
                    .and_then(|si| si["value_type"].as_str())
                    .map(|s| s.to_string());
                match ival {
                    InputVal::Var(b) => match schema_type.as_deref() {
                        // reference schemas: bindings ride as raw NAMES, mode 0
                        // (ground truth: Object 0 frame, Function 0 onClick,
                        //  Parent 0 nil_ref, Table 0 t1, GUIButton 0 btn).
                        // [patch12] mode-0 raw names resolve against OUTPUT
                        // BINDINGS (wire semantics - the raycaster proved
                        // GTV -> t144 -> SetTableValue in-game), so Table
                        // slots stay mode-0 for ALL variables. The mode-2
                        // Table ride v11 introduced is used by ZERO
                        // hand-made RS files and correlates with the
                        // Coding IDE crash on click.
                        Some("Object") | Some("Object?") | Some("Function")
                        | Some("Table") | Some("Player") => {
                            (iname, InputVal::Implicit(b))
                        }
                        // everything else - INCLUDING String - keeps the
                        // variable reference, mode 2 (ground truth:
                        // Print.Text 2 t4, Concatenate.SecondString 2 n).
                        // String literals stay mode 0; the rule is
                        // asymmetric per user hand-fixes.
                        _ => (iname, InputVal::Var(b)),
                    },
                    InputVal::Literal { value, vtype } => {
                        match schema_type.as_deref() {
                            // String-schema literals ride raw (RS ground
                            // truth: Print.Text, Property names)
                            Some("String") => (iname, InputVal::Implicit(value)),
                            // fixed schemas (Number/Bool/...): mode 0 raw
                            // text - PROVEN (T8/T9 WaitForChild 'TimeOut 0 5';
                            // the hand-fixed SimonUI export). Mode 1 here hits
                            // pairs(nil) at PowerlangImporter:182 - the
                            // SimonUI import failure had 25+ such inputs.
                            // (The T6 garbage case was VARIABLES -> mode 2,
                            // handled above; literals were never broken.)
                            Some(t) if t != "Any" => {
                                (iname, InputVal::Implicit(value))
                            }
                            // Any / array / unknown: keep shape; an "Any"
                            // token on a plain value becomes a raw string
                            // (proven: destructured parts)
                            _ => {
                                if vtype == "Any" {
                                    (iname, InputVal::Implicit(value))
                                } else {
                                    (iname, InputVal::Literal { value, vtype })
                                }
                            }
                        }
                    }
                    other => (iname, other),
                }
            })
            .collect()
    }


    pub(crate) fn emit(
        &mut self,
        btype: &str,
        inputs: Vec<(String, InputVal)>,
        outputs: Vec<(String, String)>,
        chain: &mut Vec<String>,
    ) -> String {
        let inputs = self.normalize_inputs(btype, inputs);
        self.validate(btype, &inputs);
        let name = self.display_name(btype);
        self.blocks.push(EmittedBlock {
            name: name.clone(),
            btype: btype.to_string(),
            inputs,
            outputs,
            children: Vec::new(),
            else_child: None,
        });
        self.index.insert(name.clone(), self.blocks.len() - 1);
        chain.push(name.clone());
        name
    }

    /// Creates a standalone root block (DefineFunction / event blocks).
    fn emit_root(
        &mut self,
        btype: &str,
        inputs: Vec<(String, InputVal)>,
        outputs: Vec<(String, String)>,
        roots: &mut Vec<String>,
    ) -> String {
        let inputs = self.normalize_inputs(btype, inputs);
        self.validate(btype, &inputs);
        let name = self.display_name(btype);
        self.blocks.push(EmittedBlock {
            name: name.clone(),
            btype: btype.to_string(),
            inputs,
            outputs,
            children: Vec::new(),
            else_child: None,
        });
        self.index.insert(name.clone(), self.blocks.len() - 1);
        roots.push(name.clone());
        name
    }

    fn set_children(&mut self, name: &str, children: Vec<String>) {
        if let Some(i) = self.index.get(name).copied() {
            self.blocks[i].children = children;
        }
    }

    fn set_else(&mut self, name: &str, else_first: String) {
        if let Some(i) = self.index.get(name).copied() {
            self.blocks[i].else_child = Some(else_first);
        }
    }

    pub(crate) fn validate(&self, btype: &str, inputs: &[(String, InputVal)]) {
        let entry = match self.catalog.get(btype) {
            Some(e) => e,
            None => panic!("emitted block type {:?} is not in the catalog", btype),
        };
        let cat = entry["Category"].as_str().unwrap_or("");
        assert_ne!(cat, "Deprecated", "emitted {:?} is Deprecated", btype);
        assert_ne!(cat, "Unverified", "emitted {:?} is Unverified", btype);
        let names: Vec<&str> = entry["Inputs"]
            .as_array()
            .map(|a| a.iter().map(|i| i["name"].as_str().unwrap_or("")).collect())
            .unwrap_or_default();
        for (n, iv) in inputs {
            assert!(
                names.contains(&n.as_str()) || n.starts_with("TUPLEPARAM_") || n.starts_with("TUPLE_"),
                "input {:?} not on block {:?} (catalog: {:?})",
                n,
                btype,
                names
            );
            // allowed-types arrays: a mode-1 literal must carry a listed type
            if let InputVal::Literal { vtype, .. } = iv {
                if let Some(si) = entry["Inputs"]
                    .as_array()
                    .and_then(|a| a.iter().find(|si| si["name"].as_str() == Some(n.as_str())))
                {
                    if let Some(allowed) = si["value_type"].as_array() {
                        let ok = allowed.iter().any(|t| t.as_str() == Some(vtype.as_str()));
                        assert!(
                            ok,
                            "input {:?} on {:?}: literal type {:?} not in allowed types {:?}",
                            n,
                            btype,
                            vtype,
                            allowed.iter().filter_map(|t| t.as_str()).collect::<Vec<_>>()
                        );
                    }
                }
            }
        }
    }

    fn binding_used(&self, b: &str) -> bool {
        !b.is_empty()
            && self
                .blocks
                .iter()
                .any(|blk| blk.outputs.iter().any(|(_, binding)| binding == b))
    }

    fn unique_binding(&mut self, lua: &str) -> String {
        let mut b = sanitize(lua);
        let mut i = 1;
        while self.binding_used(&b) {
            i += 1;
            b = format!("{}{}", sanitize(lua), i);
        }
        b
    }

    // ----- literals -----

    fn lit_input(&self, v: &LitVal) -> InputVal {
        match v {
            LitVal::Num(n) => InputVal::Literal { value: hexs(*n), vtype: "Number".into() },
            LitVal::Bool(b) => InputVal::Literal {
                value: if *b { "1".into() } else { "0".into() },
                vtype: "Bool".into(),
            },
            LitVal::Str(s) => InputVal::Literal { value: s.clone(), vtype: "String".into() },
            LitVal::Nil => InputVal::Literal { value: String::new(), vtype: "Nil".into() },
            LitVal::ObjectRef(p) => InputVal::Literal { value: p.clone(), vtype: "Object".into() },
            LitVal::BrickColor(n) => {
                InputVal::Literal { value: hexs(*n), vtype: "BrickColor".into() }
            }
            LitVal::UDim2(a, b, c, d) => InputVal::Literal {
                value: format!("{},{},{},{}", hexs(*a), hexs(*b), hexs(*c), hexs(*d)),
                vtype: "UDim2".into(),
            },
            LitVal::Vector3(x, y, z) => InputVal::Literal {
                value: format!("{},{},{}", hexs(*x), hexs(*y), hexs(*z)),
                vtype: "Vector3".into(),
            },
            LitVal::Vector2(x, y) => InputVal::Literal {
                value: format!("{},{}", hexs(*x), hexs(*y)),
                vtype: "Vector2".into(),
            },
            LitVal::Color3(r, g, b) => InputVal::Literal {
                // RS wire format: integer RGB bytes 0-255, hex ("FF,3C,3C" = white)
                value: format!(
                    "{:X},{:X},{:X}",
                    (r.clamp(0.0, 1.0) * 255.0).round() as u8,
                    (g.clamp(0.0, 1.0) * 255.0).round() as u8,
                    (b.clamp(0.0, 1.0) * 255.0).round() as u8
                ),
                vtype: "Color3".into(),
            },
        }
    }

    fn val_input(&self, v: &Val, hint: &str) -> InputVal {
        match v {
            Val::Lit(l) => {
                let mut i = self.lit_input(l);
                if hint == "Number" {
                    if let InputVal::Literal { vtype, .. } = &mut i {
                        *vtype = "Number".into();
                    }
                }
                i
            }
            Val::Var(b) => InputVal::Var(b.clone()),
            Val::Path(p) => InputVal::Implicit(p.clone()),
            Val::Tuple(Some(b)) => InputVal::Var(b.clone()),
            Val::Tuple(None) => InputVal::Implicit(String::new()),
        }
    }

    // ----- top level -----

    pub fn transpile(&mut self, stmts: &[Stmt]) -> Vec<String> {
        let mut roots: Vec<String> = Vec::new();
        // [v23] only THIS call records statement spans (nested stmts()
        // calls are loop/if/function bodies, never top-level statements)
        self.span_record = true;
        self.stmts(stmts, &mut roots);
        self.span_record = false;
        // RS runs DISCONNECTED roots in arbitrary order (user-verified
        // failure mode: DefineFunction/event roots were never chained).
        // Sequence every top-level root under one inert Comment2 container:
        // its ChildBlocks order IS the run order, and comments run nothing.
        // F3: chain siblings everywhere first (execution order, closed at
        // definition/event roots) - user ground truth layouts T1/T2/T5.
        {
            let all: Vec<String> = self.blocks.iter().map(|b| b.name.clone()).collect();
            for name in &all {
                let i = self.index[name];
                let kids = self.blocks[i].children.clone();
                if kids.len() > 1 {
                    let chained = chain_walk(kids, &self.index, &mut self.blocks);
                    self.blocks[i].children = chained;
                }
            }
        }
        let mut is_child: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        for b in &self.blocks {
            for c in &b.children {
                is_child.insert(c.clone());
            }
            if let Some(ec) = &b.else_child {
                is_child.insert(ec.clone());
            }
        }
        let root_names: Vec<String> = self
            .blocks
            .iter()
            .filter(|b| !is_child.contains(b.name.as_str()))
            .map(|b| b.name.clone())
            .collect();
        // [v29] wire each top-level statement's OWN blocks into its chain
        // (chain_walk's leaf runs, but bounded by the statement span), then
        // leave every statement as an UNPARENTED sibling: the ROOT Comment2
        // container below fans out to all of them in source order, exactly
        // like the user's handmade files (function_example v0.1: ROOT
        // ChildBlocks = Print1, Define Function1, Execute Function1).
        // chain_walk used to run over ALL top-level blocks at once, so its
        // leaf runs crossed statement boundaries and chained the statements
        // into one long snake (SetVariable1 -> SetVariable2 -> print) - the
        // wiring the user rejected ("from start you were doing it all
        // wrong"). Execution is unchanged: a block's ChildBlocks run as a
        // sequence, so ROOT's children run in source order; body-level
        // sequencing (the v7 sequence model) is untouched.
        let mut root_names: Vec<String> = Vec::new();
        {
            let mut seen_ranges: std::collections::HashSet<(usize, usize)> =
                std::collections::HashSet::new();
            let mut covered: std::collections::HashSet<usize> = std::collections::HashSet::new();
            for r in &self.stmt_spans {
                // guard-restructured statements alias one consuming range
                if !seen_ranges.insert(*r) {
                    continue;
                }
                let (s, e) = *r;
                if s < e && e <= self.blocks.len() {
                    // only the blocks that are STILL unparented get wired
                    // here: a statement's span also covers pre-wired inner
                    // structures (loop/if bodies, function bodies) whose
                    // wiring must not be touched - re-walking them created
                    // duplicate edges and diamond parents that blew up the
                    // decoder on the big samples
                    let names: Vec<String> = (s..e)
                        .map(|i| self.blocks[i].name.clone())
                        .filter(|n| !is_child.contains(n.as_str()))
                        .collect();
                    if !names.is_empty() {
                        root_names.extend(chain_walk(names, &self.index, &mut self.blocks));
                    }
                    for i in s..e {
                        covered.insert(i);
                    }
                }
            }
            // blocks outside every span (strays): their own sibling heads
            for (i, b) in self.blocks.iter().enumerate() {
                if !covered.contains(&i) && !is_child.contains(b.name.as_str()) {
                    root_names.push(b.name.clone());
                }
            }
        }
        if root_names.len() > 1 {
            let name = self.display_name("Comment2");
            let inputs = vec![
                ("Color".into(), InputVal::Implicit("3E9".into())),
                ("Comment".into(), InputVal::Implicit("ROOT".into())),
            ];
            self.validate("Comment2", &inputs);
            self.blocks.push(EmittedBlock {
                name: name.clone(),
                btype: "Comment2".into(),
                inputs,
                outputs: Vec::new(),
                children: root_names,
                else_child: None,
            });
            self.index.insert(name.clone(), self.blocks.len() - 1);
            self.note(
                "INFO",
                "top-level roots sequenced under a ROOT Comment2 container (RS run order)".into(),
            );
            return vec![name];
        }
        root_names
    }

    pub fn blocks_out(&self) -> usize {
        self.blocks.len()
    }

    fn stmts(&mut self, stmts: &[Stmt], chain: &mut Vec<String>) {
        // [v23] span recording: statement i's blocks = stmt_spans[i].
        // Guard-restructured statements consume the rest of the block,
        // so filler spans alias the consuming range.
        let span_top = self.span_record;
        self.span_record = false;
        let mut span0;
        let mut i = 0;
        while i < stmts.len() {
            span0 = if span_top { self.blocks.len() } else { 0 };
            // Detect early-return guard pattern: `if cond then return end`
            // (single arm, no else, body = exactly [Return(empty)]) followed
            // by more statements. RS's If block doesn't early-exit for
            // Nil-valued FunctionReturn, so we restructure to:
            //   if !cond then <rest> end
            // This puts the rest INSIDE the If's body with the inverted
            // condition. Only matches BARE returns (no values) — value-
            // returning guards work fine via FunctionReturn (which DOES
            // exit in RS when it has real values).
            if let Stmt::If(arms, els) = &stmts[i] {
                if arms.len() == 1 && els.is_none() {
                    let (cond, body) = &arms[0];
                    let is_bare_return_guard = body.len() == 1
                        && matches!(&body[0], Stmt::Return(e) if e.is_empty());
                    // RUNTIME BUG FIX (PATCH 10): also detect `break` guards.
                    // `if cond then break end` + rest must be restructured to
                    // `if cond then break else <rest> end` so the rest only
                    // runs when cond is FALSE. Without this, the rest is
                    // emitted as SIBLINGS of the If, and RS executes them
                    // even after BreakLoop - causing nil arithmetic errors
                    // when the raycast misses (raycastResult is nil, but
                    // raycastResult.Instance etc. still execute).
                    let is_bare_break_guard = body.len() == 1
                        && matches!(&body[0], Stmt::Break);
                    if is_bare_return_guard && i + 1 < stmts.len() {
                        let inverted = Self::invert_cond(cond);
                        let rest = &stmts[i + 1..];
                        let inputs = match self.direct_if_inputs(&inverted, chain) {
                            Some(inp) => inp,
                            None => {
                                let cv = self.condition(&inverted, chain);
                                Self::if_inputs(&cv)
                            }
                        };
                        let if_name = self.emit("If", inputs, vec![], chain);
                        self.push_scope();
                        let mut body_chain = Vec::new();
                        self.stmts(rest, &mut body_chain);
                        self.set_children(&if_name, body_chain);
                        self.pop_scope();
                        // [v23] the guard consumed the rest of the block:
                        // record its span + filler spans for the eaten stmts
                        if span_top {
                            self.stmt_spans.push((span0, self.blocks.len()));
                            for _ in (i + 1)..stmts.len() {
                                self.stmt_spans.push((span0, self.blocks.len()));
                            }
                        }
                        return;
                    }
                    if is_bare_break_guard && i + 1 < stmts.len() {
                        // Restructure: `if cond then break end` + rest →
                        // `if cond then break else <rest> end`
                        // Keep the ORIGINAL condition (not inverted).
                        // BreakLoop in then-branch exits the loop when
                        // cond is true. Rest in else-branch runs only
                        // when cond is false (raycastResult ~= nil).
                        let rest = &stmts[i + 1..];
                        let inputs = match self.direct_if_inputs(cond, chain) {
                            Some(inp) => inp,
                            None => {
                                let cv = self.condition(cond, chain);
                                Self::if_inputs(&cv)
                            }
                        };
                        let if_name = self.emit("If", inputs, vec![], chain);
                        // then-branch: BreakLoop
                        self.push_scope();
                        let mut then_chain = Vec::new();
                        self.emit("BreakLoop", vec![], vec![], &mut then_chain);
                        self.set_children(&if_name, then_chain);
                        self.pop_scope();
                        // else-branch: rest of the body
                        let mut else_chain = Vec::new();
                        self.push_scope();
                        self.stmts(rest, &mut else_chain);
                        self.pop_scope();
                        // Use chain_walk to properly link the else chain.
                        // The previous for-w loop skipped blocks that already
                        // had children (like nested If blocks), orphaning the
                        // blocks after them. chain_walk handles BODY blocks
                        // by continuing through the container.
                        if else_chain.len() > 1 {
                            else_chain = chain_walk(else_chain, &self.index, &mut self.blocks);
                        }
                        if let Some(first) = else_chain.first().cloned() {
                            if let Some(i) = self.index.get(&if_name).copied() {
                                self.blocks[i].else_child = Some(first);
                            }
                        }
                        // [v23] same filler-span bookkeeping as the
                        // bare-return guard above
                        if span_top {
                            self.stmt_spans.push((span0, self.blocks.len()));
                            for _ in (i + 1)..stmts.len() {
                                self.stmt_spans.push((span0, self.blocks.len()));
                            }
                        }
                        return;
                    }
                }
            }
            // Detect bare `return` as the LAST statement of a block.
            // Emit a FunctionReturn with 1 Nil value (T8-proven 1-input
            // shape). The original code dropped bare returns entirely,
            // which was harmless at end-of-function but emitted a noisy
            // WORKAROUND note. Emitting the Nil-valued FunctionReturn is
            // safe at end-of-function (no continuation to worry about).
            if i == stmts.len() - 1 {
                if let Stmt::Return(exprs) = &stmts[i] {
                    if exprs.is_empty() {
                        self.emit(
                            "FunctionReturn",
                            vec![
                                ("Returns".into(),
                                 InputVal::Implicit("TUPLEPARAM_Returns_1".into())),
                                ("TUPLEPARAM_Returns_1".into(),
                                 InputVal::Literal { value: String::new(), vtype: "Nil".into() }),
                            ],
                            vec![],
                            chain,
                        );
                        // [v23] record span for this bare-return statement
                        if span_top {
                            self.stmt_spans.push((span0, self.blocks.len()));
                        }
                        i += 1;
                        continue;
                    }
                }
            }
            self.stmt(&stmts[i], chain);
            // [v23] record span for this statement
            if span_top {
                self.stmt_spans.push((span0, self.blocks.len()));
            }
            i += 1;
        }
    }

    /// Invert a BinOp comparison for early-return guard restructure.
    /// == <-> ~=
    /// <  <-> >=
    /// <= <-> >
    /// >  <-> <=
    /// >= <-> <
    /// For non-BinOp conds, returns the cond unchanged.
    fn invert_cond(cond: &Expr) -> Expr {
        if let Expr::BinOp(op, a, b) = cond {
            let inverted_op = match op {
                BinOp::Eq => BinOp::Ne,
                BinOp::Ne => BinOp::Eq,
                BinOp::Lt => BinOp::Ge,
                BinOp::Le => BinOp::Gt,
                BinOp::Gt => BinOp::Le,
                BinOp::Ge => BinOp::Lt,
                other => *other,
            };
            Expr::BinOp(inverted_op, a.clone(), b.clone())
        } else {
            cond.clone()
        }
    }

    /// Fuses a plain comparison into If inputs when both operands are plain
    /// values (vars/literals): no Comparison block, no gate variable.
    /// Returns None for compound conditions - those keep the gate path.
    fn direct_if_inputs(
        &mut self,
        cond: &Expr,
        chain: &mut Vec<String>,
    ) -> Option<Vec<(String, InputVal)>> {
        if let Expr::BinOp(op, a, b) = cond {
            if matches!(
                op,
                BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge
            ) {
                let av = self.expr(a, chain, None);
                let bv = self.expr(b, chain, None);
                let plain = |v: &Val| matches!(v, Val::Var(_) | Val::Lit(_));
                if plain(&av) && plain(&bv) {
                    let ct = match op {
                        BinOp::Eq => "==",
                        BinOp::Ne => "~=",
                        BinOp::Lt => "<",
                        BinOp::Le => "<=",
                        BinOp::Gt => ">",
                        BinOp::Ge => ">=",
                        _ => unreachable!(),
                    };
                    // input order per user ground truth: Value 1, Value 2, Type
                    return Some(vec![
                        ("Value 1".into(), self.val_input(&av, "Any")),
                        ("Value 2".into(), self.val_input(&bv, "Any")),
                        (
                            "ComparisonType".into(),
                            InputVal::Literal { value: ct.into(), vtype: "String".into() },
                        ),
                    ]);
                }
            }
        }
        None
    }

    fn if_inputs(var: &str) -> Vec<(String, InputVal)> {
        vec![
            ("Value 1".into(), InputVal::Var(var.to_string())),
            (
                "ComparisonType".into(),
                InputVal::Literal { value: "==".into(), vtype: "String".into() },
            ),
            ("Value 2".into(), InputVal::Literal { value: "1".into(), vtype: "Bool".into() }),
        ]
    }

    fn stmt(&mut self, s: &Stmt, chain: &mut Vec<String>) {
        match s {
            Stmt::Local(names, exprs) => {
                if exprs.is_empty() {
                    for n in names {
                        let b = self.unique_binding(n);
                        self.emit(
                            "SetVariable1",
                            vec![(
                                "Value".into(),
                                InputVal::Literal { value: String::new(), vtype: "String".into() },
                            )],
                            vec![("VariableName".into(), b.clone())],
                            chain,
                        );
                        self.declare(n, b);
                    }
                    return;
                }
                if names.len() > 1 && exprs.len() == 1 {
                    // local a, b = f(): destructure the returned tuple.
                    // RS multi-return convention (T8 single-return ground
                    // truth + T5 params symmetry): the CALL outputs
                    // TUPLEPARAM_ReturnedValues_i, one per name - no unpack.
                    if let Expr::Call(f, args) = &exprs[0] {
                        if let Expr::Name(fnname) = &**f {
                            let fb = self.lookup(fnname);
                            let arity = fb
                                .as_ref()
                                .and_then(|b| self.fn_returns.get(b))
                                .copied();
                            if let (Some(fb), Some(arity)) = (fb, arity) {
                                if arity == names.len() && arity >= 1 {
                                    let mut inputs = vec![(
                                        "Function".into(),
                                        InputVal::Var(fb.clone()),
                                    )];
                                    match args.len() {
                                        0 => {
                                            inputs.push((
                                                "Parameters".into(),
                                                InputVal::Implicit(String::new()),
                                            ));
                                        }
                                        1 => {
                                            let av =
                                                self.expr(&args[0], chain, None);
                                            let iv = self.val_input(&av, "Any");
                                            inputs.push((
                                                "Parameters".into(),
                                                InputVal::Implicit(
                                                    "TUPLEPARAM_Parameters_1".into(),
                                                ),
                                            ));
                                            inputs.push((
                                                "TUPLEPARAM_Parameters_1".into(),
                                                iv,
                                            ));
                                        }
                                        _ => {
                                            // B9 tuple-INPUT convention (user
                                            // decode: the legacy TableUnpack
                                            // shape broke the importer - this
                                            // arm served getMaterial's 2-arg
                                            // destructured call)
                                            let pnames: Vec<String> = (1
                                                ..=args.len())
                                                .map(|ai| {
                                                    format!(
                                                        "TUPLEPARAM_Parameters_{}",
                                                        ai
                                                    )
                                                })
                                                .collect();
                                            inputs.push((
                                                "Parameters".into(),
                                                InputVal::Implicit(
                                                    pnames.join(","),
                                                ),
                                            ));
                                            for (ai, e) in args.iter().enumerate()
                                            {
                                                let v =
                                                    self.expr(e, chain, None);
                                                let iv =
                                                    self.val_input(&v, "Any");
                                                inputs.push((
                                                    pnames[ai].clone(),
                                                    iv,
                                                ));
                                            }
                                        }
                                    }
                                    let mut outs = vec![(
                                        "TUPLE_ReturnedValues".into(),
                                        arity.to_string(),
                                    )];
                                    let mut binds = Vec::new();
                                    for (i, nm) in names.iter().enumerate() {
                                        let b = self.unique_binding(nm);
                                        outs.push((
                                            format!(
                                                "TUPLEPARAM_ReturnedValues_{}",
                                                i + 1
                                            ),
                                            b.clone(),
                                        ));
                                        binds.push((nm.clone(), b));
                                    }
                                    self.emit(
                                        "ExecuteFunction",
                                        inputs,
                                        outs,
                                        chain,
                                    );
                                    // [patch10] destructured tuple members ride
                                    // TUPLEPARAM wires too - register every
                                    // member when the function returns a table.
                                    // [patch12] no registration blocks: the
                                    // TUPLEPARAM bindings ARE the variables
                                    // (wire semantics). Track table-ness for
                                    // SetTableValue dispatch only.
                                    if self.fn_ret_table.contains(&fb) {
                                        for (_nm, b) in binds.iter() {
                                            self.lua_tables.insert(b.clone());
                                        }
                                    }
                                    for (nm, b) in binds {
                                        self.declare(&nm, b);
                                    }
                                    return;
                                }
                            }
                        }
                    }
                    // fallback: unknown/mismatched arity - old unpack path
                    self.note(
                        "GAP",
                        "multi-return destructure of unknown-arity call;                          using table unpack (unverified wire shape)".into(),
                    );
                    let tuple_in = self.tuple_input(exprs, chain);
                    match tuple_in {
                        InputVal::Var(tv) => {
                            let pack_tbl = self.temp();
                            self.emit(
                                "TablePack",
                                vec![("Values".into(), InputVal::Var(tv))],
                                vec![("TableVariable".into(), pack_tbl.clone())],
                                chain,
                            );
                            for (i, n) in names.iter().enumerate() {
                                let pb = self.unique_binding(n);
                                self.emit(
                                    "GetTableValue",
                                    vec![
                                        ("Table".into(), InputVal::Var(pack_tbl.clone())),
                                        (
                                            "Key".into(),
                                            InputVal::Literal {
                                                value: hexs((i + 1) as f64),
                                                vtype: "Number".into(),
                                            },
                                        ),
                                    ],
                                    vec![("Value".into(), pb.clone())],
                                    chain,
                                );
                                self.declare(n, pb);
                            }
                        }
                        InputVal::Literal { value, .. } => {
                            // comma-joined literal tuple
                            let parts: Vec<&str> = value.split(',').collect();
                            for (i, n) in names.iter().enumerate() {
                                let part = parts.get(i).copied().unwrap_or("").to_string();
                                let pb = self.unique_binding(n);
                                self.emit(
                                    "SetVariable1",
                                    vec![(
                                        "Value".into(),
                                        InputVal::Literal { value: part, vtype: "Any".into() },
                                    )],
                                    vec![("VariableName".into(), pb.clone())],
                                    chain,
                                );
                                self.declare(n, pb);
                            }
                        }
                        _ => {
                            self.note("GAP", "multi-name local from non-tuple value".into());
                        }
                    }
                    return;
                }
                for (n, e) in names.iter().zip(exprs.iter()) {
                    self.assign_to(n, e, chain);
                }
            }
            Stmt::LocalFunction(name, params, body) => {
                self.function(name, params, body, chain);
            }
            Stmt::Assign(target, value) => self.assign_expr(target, value, chain),
            Stmt::MultiAssign(targets, values) => {
                for (t, v) in targets.iter().zip(values.iter()) {
                    self.assign_expr(t, v, chain);
                }
            }
            Stmt::If(arms, els) => self.if_stmt(arms, els, chain),
            Stmt::While(cond, body) => self.while_stmt(cond, body, chain),
            Stmt::NumericFor(var, from, to, step, body) => {
                self.numeric_for(var, from, to, step.as_ref(), body, chain)
            }
            Stmt::GenericFor(names, exprs, body) => self.generic_for(names, exprs, body, chain),
            Stmt::Return(exprs) => {
                // RS multi-return convention (user hand-fix, B5):
                // Returns = comma-joined TUPLEPARAM_Returns_i names (mode 0,
                // reference-before-declaration per T8); one TUPLEPARAM_
                // Returns_i input per value. N=1 is the T8-proven shape.
                if exprs.is_empty() {
                    // bare 'return': the empty-inputs FunctionReturn breaks
                    // the importer (pairs(nil) - T8 proved only 1/2-input
                    // shapes). SKIPPED for now: the block graph continues
                    // after this point, so token-guard patterns degrade.
                    // TODO: hand-fix one in RS, export, match the shape.
                    self.note(
                        "WORKAROUND",
                        "bare 'return' dropped: RS shape for empty FunctionReturn \
                         unproven; execution continues in the block graph".into(),
                    );
                } else {
                    let names: Vec<String> = (1..=exprs.len())
                        .map(|i| format!("TUPLEPARAM_Returns_{}", i))
                        .collect();
                    let mut inputs = vec![(
                        "Returns".into(),
                        InputVal::Implicit(names.join(",")),
                    )];
                    for (i, e) in exprs.iter().enumerate() {
                        let v = self.expr(e, chain, None);
                        // [patch10] attribute table-ness to the enclosing
                        // function so call sites register the bound result
                        // as a real variable (TUPLEPARAM wires are not
                        // variables - patch8 ground truth).
                        if self.val_is_table(&v) {
                            if let Some(fb) = self.fn_stack.last().cloned() {
                                self.fn_ret_table.insert(fb);
                            }
                        }
                        let iv = self.val_input(&v, "Any");
                        inputs.push((names[i].clone(), iv));
                    }
                    self.emit("FunctionReturn", inputs, vec![], chain);
                }
            }
            Stmt::Break => {
                self.emit("BreakLoop", vec![], vec![], chain);
            }
            Stmt::Continue => {
                self.emit("ContinueLoop", vec![], vec![], chain);
            }
            Stmt::ExprStmt(e) => {
                // bare task.wait() at end of a loop body is implicit in block loops
                if is_bare_task_wait(e) {
                    self.note(
                        "INFO",
                        "bare task.wait() dropped: WhileLoop3/ForLoop2 yield between iterations \
                         (matches the reconstructed originals)"
                            .into(),
                    );
                    return;
                }
                // [patch9] mark true-discard so a bare call keeps the
                // T8 count-0 shape; consumed results elsewhere must bind.
                self.exprstmt_discard = true;
                self.expr(e, chain, None);
            }
        }
    }

    fn assign_to(&mut self, lua: &str, e: &Expr, chain: &mut Vec<String>) {
        let binding = match self.lookup(lua) {
            Some(b) => b.clone(),
            None => self.unique_binding(lua),
        };
        // TARGETED CSE INVALIDATION (Opportunity #4): previously a
        // blanket `cse_map.clear()` that wiped unrelated cached
        // subexpressions too. Now we only invalidate entries whose
        // key references `lua` (lua name) or `binding` (binding).
        self.cse_invalidate(lua, &binding);
        // Math-method alias tracking: `local math_X = math.<method>`
        // records the alias so call sites `math_X(args)` can be inlined
        // back to `math.<method>(args)` and routed through SolveEquation
        // (1 block) instead of the general ExecuteFunction path (2+
        // blocks). If the assignment is NOT a math.X pattern, clear any
        // existing alias for this binding (it was reassigned to a
        // non-aliased value).
        if let Expr::Index(base, key) = e {
            if let (Expr::Name(ns), Expr::Str(m)) = (&**base, &**key) {
                if ns == "math" && matches!(m.as_str(),
                    "abs" | "floor" | "ceil" | "sqrt" | "min" | "max" |
                    "clamp" | "sin" | "cos" | "tan" | "pow" | "exp" |
                    "log" | "huge" | "pi" | "noise" | "random" |
                    "randomseed" | "round" | "asin" | "acos" | "atan" |
                    "atan2" | "fmod" | "modf" | "sign") {
                    self.math_aliases.insert(binding.clone(), m.clone());
                } else {
                    self.math_aliases.remove(&binding);
                }
            } else {
                self.math_aliases.remove(&binding);
            }
        } else {
            self.math_aliases.remove(&binding);
        }
        // pure-literal assignment: one SetVariable1 block so the binding
        // always has a producer
        if let Some(l) = self.const_lit(e) {
            let input = match l {
                LitVal::Num(n) => InputVal::Literal { value: hexs(n), vtype: "Number".into() },
                LitVal::Bool(b) => InputVal::Literal {
                    value: if b { "1".into() } else { "0".into() },
                    vtype: "Bool".into(),
                },
                LitVal::Str(s) => {
                    InputVal::Literal { value: s, vtype: "String".into() }
                }
                LitVal::BrickColor(n) => {
                    InputVal::Literal { value: hexs(n), vtype: "BrickColor".into() }
                }
                LitVal::UDim2(a, b, c, d) => InputVal::Literal {
                    value: format!("{},{},{},{}", hexs(a), hexs(b), hexs(c), hexs(d)),
                    vtype: "UDim2".into(),
                },
                LitVal::Vector3(x, y, z) => InputVal::Literal {
                    value: format!("{},{},{}", hexs(x), hexs(y), hexs(z)),
                    vtype: "Vector3".into(),
                },
                LitVal::Vector2(x, y) => InputVal::Literal {
                    value: format!("{},{}", hexs(x), hexs(y)),
                    vtype: "Vector2".into(),
                },
                LitVal::Color3(r, g, b) => InputVal::Literal {
                    // RS wire format: integer RGB bytes 0-255, hex ("FF,3C,3C" = white)
                    value: format!(
                        "{:X},{:X},{:X}",
                        (r.clamp(0.0, 1.0) * 255.0).round() as u8,
                        (g.clamp(0.0, 1.0) * 255.0).round() as u8,
                        (b.clamp(0.0, 1.0) * 255.0).round() as u8
                    ),
                    vtype: "Color3".into(),
                },
                LitVal::Nil => InputVal::Literal { value: String::new(), vtype: "Nil".into() },
                LitVal::ObjectRef(p) => InputVal::Literal { value: p.clone(), vtype: "Object".into() },
            };
            self.emit(
                "SetVariable1",
                vec![("Value".into(), input)],
                vec![("VariableName".into(), binding.clone())],
                chain,
            );
        } else {
            let v = self.expr(e, chain, Some(&binding));
            // Val::Path -> Val::Lit(LitVal::ObjectRef) so the Lit branch
            // below emits a SetVariable1 producer with Value =
            // InputVal::Literal { value: path, vtype: "Object" } (mode 1
            // type=Object, same shape as the T2-proven
            // SetObjectProperty.Value handling at line ~1001).
            //
            // Without this conversion, path-returning assignments
            //   local Workspace = game:GetService("Workspace")  (Bug A fix)
            //   local W = workspace
            //   local G = game
            // would fall through BOTH the Lit and Var branches and leave
            // the binding dead (no producer block) - downstream references
            // render as "?" in RetroStudio (user ground truth T2).
            let v = match v {
                Val::Path(p) => Val::Lit(LitVal::ObjectRef(p)),
                other => other,
            };
            // Constructor literals (Color3/UDim2/Vector3/BrickColor/math.huge/
            // enum items) return Val::Lit without emitting a producer block.
            // Give the binding one so references don't dangle (user ground
            // truth T2: unproduced vars rendered as "?" in RetroStudio).
            if let Val::Lit(l) = &v {
                let input = self.lit_input(l);
                self.emit(
                    "SetVariable1",
                    vec![("Value".into(), input)],
                    vec![("VariableName".into(), binding.clone())],
                    chain,
                );
            }
            // variable-to-variable copy (user-caught SimonUI): 'maxT = value',
            // 'local myToken = token' emitted NO block - the target binding
            // dangled and every later reference was dead. Luau copies the
            // value; emit an explicit copy block.
            if let Val::Var(b) = &v {
                if b != &binding {
                    self.emit(
                        "SetVariable1",
                        vec![("Value".into(), InputVal::Var(b.clone()))],
                        vec![("VariableName".into(), binding.clone())],
                        chain,
                    );
                }
                // [patch10] aliases inherit table-ness so SetTableValue
                // dispatch keeps working through variable-to-variable
                // copies (`local t2 = t`).
                if self.lua_tables.contains(b) {
                    self.lua_tables.insert(binding.clone());
                }
            }
        }
        if self.scopes.last().unwrap().get(lua).is_none() {
            self.declare(lua, binding);
        } else {
            // `lua` was already declared in this scope; we just reassigned
            // its binding above. The targeted cse_invalidate in this
            // function's prologue already removed stale entries. (Was a
            // blanket `cse_map.clear()` — too conservative.)
        }
    }

    fn assign_expr(&mut self, target: &Expr, value: &Expr, chain: &mut Vec<String>) {
        // BindableFunction.OnInvoke = function(...) ... end
        if let (Expr::Index(base, key), Expr::Function(params, body)) = (target, value) {
            if let Expr::Str(prop) = &**key {
                if prop == "OnInvoke" {
                    let basev = self.expr(base, chain, None);
                    let mut roots: Vec<String> = Vec::new();
                    let mut outputs = Vec::new();
                    if let Some(p0) = params.first() {
                        if p0 != "_" {
                            let b = self.unique_binding(p0);
                            outputs.push(("TUPLE_Parameters".to_string(), "0".to_string()));
                            // declared below via tuple unpack in body scope
                            self.pending_event_params.push((p0.clone(), b));
                        }
                    }
                    let name = self.emit_root(
                        "BindableFunctionOnInvoke",
                        vec![("BindableFunction".into(), self.val_input(&basev, "Object"))],
                        outputs,
                        &mut roots,
                    );
                    self.push_scope();
                    let mut body_chain = Vec::new();
                    if !params.is_empty() {
                        let pack_tbl = self.temp();
                        self.emit(
                            "TablePack",
                            vec![("Values".into(), InputVal::Var("0".into()))],
                            vec![("TableVariable".into(), pack_tbl.clone())],
                            &mut body_chain,
                        );
                        for (i, prm) in params.iter().enumerate() {
                            let pb = self.unique_binding(prm);
                            self.emit(
                                "GetTableValue",
                                vec![
                                    ("Table".into(), InputVal::Var(pack_tbl.clone())),
                                    (
                                        "Key".into(),
                                        InputVal::Literal {
                                            value: hexs((i + 1) as f64),
                                            vtype: "Number".into(),
                                        },
                                    ),
                                ],
                                vec![("Value".into(), pb.clone())],
                                &mut body_chain,
                            );
                            self.declare(prm, pb);
                        }
                        self.note(
                            "VERIFY",
                            "OnInvoke parameters accessed via TablePack + GetTableValue                              (1-based) - confirm in game"
                                .into(),
                        );
                    }
                    self.stmts(body, &mut body_chain);
                    self.set_children(&name, body_chain);
                    self.pop_scope();
                    return;
                }
            }
        }
        if let Expr::Index(base, key) = target {
            let basev = self.expr(base, chain, None);
            let prop = match &**key {
                Expr::Str(s) => s.clone(),
                other => {
                    let kv = self.expr(other, chain, None);
                    match kv {
                        Val::Lit(LitVal::Str(s)) => s,
                        Val::Var(b) => b,
                        _ => String::new(),
                    }
                }
            };
            let valv = self.expr(value, chain, None);
            // static paths on the Any-typed Value input: RS resolves object
            // references BY NAME with a concrete "Object" type token (user
            // hand-fix, T2: Value 1 workspace Object)
            let valv = match &valv {
                Val::Path(pp) => Val::Lit(LitVal::ObjectRef(pp.clone())),
                other => other.clone(),
            };
            // Lua-table LHS dispatch: if the base is a binding tracked in
            // self.lua_tables (created via CreateTable, RaycastParams.new(),
            // table_expr {}, FindPartOnRay RaycastResult synthesis, OR a
            // GetTableValue result from another Lua table - see PATCH 7b),
            // we MUST use SetTableValue - not SetObjectProperty. RS's
            // SetObjectProperty expects an Instance and rejects Lua tables
            // at runtime: "'Object' input must be a 'Instance' got 'table'".
            // For an Instance LHS (game.X.Parent = ... etc), keep
            // SetObjectProperty.
            let is_lua_table = match &basev {
                Val::Var(b) => self.lua_tables.contains(b),
                _ => false,
            };
            // A non-string key (number, variable, expression like j+1) can
            // ONLY target a Lua table: Instance property names are always
            // strings in Luau (identifier or string literal). If the base is
            // a variable (not a confirmed Instance/ObjectRef) and the key is
            // non-string, use SetTableValue regardless of lua_tables tracking.
            // This eliminates the "[GAP] non-string property key on Instance"
            // fallback for array/param writes like `arr[i] = arr[i+1]` where
            // arr is an untracked local or function parameter.
            let key_is_string = matches!(&**key, Expr::Str(_));
            let base_is_var = matches!(&basev, Val::Var(_));
            let use_set_table = is_lua_table || (!key_is_string && base_is_var);
            if use_set_table {
                // RUNTIME BUG FIX: for non-string keys (e.g., `squares[i][j] = sq`
                // where j is a Number loop variable), pass the actual key
                // VALUE as the Key input (mode 2 var ref), not a literal string
                // of the binding NAME. Previously the code extracted the
                // binding name "j" as a literal String for the Key, which
                // made SetTableValue look up the literal string "j" instead
                // of the actual integer key (1, 2, 3, ...).
                let key_input = match &**key {
                    Expr::Str(s) => InputVal::Literal {
                        value: property_alias(s).to_string(),
                        vtype: "String".into(),
                    },
                    other => {
                        let kv = self.expr(other, chain, None);
                        self.val_input(&kv, "Any")
                    }
                };
                self.emit(
                    "SetTableValue",
                    vec![
                        ("Table".into(), self.val_input(&basev, "Table")),
                        ("Key".into(), key_input),
                        ("Value".into(), self.val_input(&valv, "Any")),
                    ],
                    vec![],
                    chain,
                );
            } else {
                // For Instance LHS, Property must be a String (property name).
                // This branch is now only reached when the key IS a string
                // (non-string keys with a variable base go to SetTableValue
                // above) or when the base is a confirmed Instance/ObjectRef
                // with a non-string key (a genuine Luau error — keep the GAP).
                let prop_str = match &**key {
                    Expr::Str(s) => property_alias(s).to_string(),
                    _ => {
                        self.note("GAP", format!("non-string property key on Instance: {:?}", key));
                        String::new()
                    }
                };
                self.emit(
                    "SetObjectProperty",
                    vec![
                        ("Object".into(), self.val_input(&basev, "String")),
                        (
                            "Property".into(),
                            InputVal::Literal { value: prop_str, vtype: "String".into() },
                        ),
                        ("Value".into(), self.val_input(&valv, "Any")),
                    ],
                    vec![],
                    chain,
                );
            }
            // TARGETED CSE INVALIDATION (Opportunity #4): SetObjectProperty
            // on `<obj>.<prop>` modifies only the property `<prop>` of
            // object `<obj>`. Cached reads of OTHER properties on the SAME
            // object remain valid (object reference unchanged). Invalidate
            // only entries whose key is exactly `<basev_binding>.<prop>`
            // (direct access to the modified property) or starts with
            // `<basev_binding>.<prop>.` (deeper accesses through it).
            // Previously a blanket `cse_map.clear()` — too conservative.
            if let Val::Var(bv) = &basev {
                let aliased = property_alias(&prop);
                let prefix_direct = format!("{}.{}", bv, aliased);
                let to_remove: Vec<String> = self
                    .cse_map
                    .keys()
                    .filter(|k| {
                        *k == &prefix_direct
                        || k.starts_with(&format!("{}.", prefix_direct))
                    })
                    .cloned()
                    .collect();
                for k in to_remove {
                    self.cse_map.remove(&k);
                }
            } else {
                self.cse_map.clear();
            }
            return;
        }
        if let Expr::Name(n) = target {
            self.assign_to(n, value, chain);
            return;
        }
        self.note("GAP", format!("unsupported assignment target {:?}", target));
    }

    fn if_stmt(&mut self, arms: &[(Expr, Vec<Stmt>)], els: &Option<Vec<Stmt>>, chain: &mut Vec<String>) {
        let (cond, body) = &arms[0];
        // Truthiness fusion (user typeof recreation, T5): `if var then` with
        // no else lowers to TypeOf + bool-check / nil-check pair, because a
        // bare variable may be a bool, object, string, ... and RS has no
        // truthy comparison against an empty value.
        if arms.len() == 1 && els.is_none() {
            if let Expr::Name(n) = cond {
                let sv = match self.lookup(n) {
                    Some(b) => b,
                    None => sanitize(n),
                };
                // Single combined condition: `var ~= nil AND var ~= false`
                //   - Bool true:  true AND true = true  (truthy)
                //   - Bool false: true AND false = false (falsy)
                //   - Nil:        false AND true = false (falsy)
                //   - Instance / String / ...: true AND true = true (truthy)
                //
                // REPLACES the old 3-If truthy fusion that emitted the body
                // TWICE (once in `var == true` branch, once in `var ~= nil`
                // branch). The duplication cascaded: `if a then if b then
                // BODY end end` produced 4 copies of BODY. With this fix,
                // BODY is emitted exactly once.
                //
                // Cost: 3 condition blocks (2 Comparison + 1 ANDAny) + 1 If
                // + N body = 4 + N. Old cost was 1 TypeOf + 3 If + 2N body
                // = 4 + 2N. Saves N blocks per `if var then` (no else).
                let c1 = self.temp();
                self.emit(
                    "Comparison",
                    vec![
                        ("Value 1".into(), InputVal::Var(sv.clone())),
                        ("ComparisonType".into(),
                         InputVal::Literal { value: "~=".into(), vtype: "String".into() }),
                        ("Value 2".into(),
                         InputVal::Literal { value: String::new(), vtype: "Nil".into() }),
                    ],
                    vec![("Result".into(), c1.clone())],
                    chain,
                );
                let c2 = self.temp();
                self.emit(
                    "Comparison",
                    vec![
                        ("Value 1".into(), InputVal::Var(sv.clone())),
                        ("ComparisonType".into(),
                         InputVal::Literal { value: "~=".into(), vtype: "String".into() }),
                        ("Value 2".into(),
                         InputVal::Literal { value: "0".into(), vtype: "Bool".into() }),
                    ],
                    vec![("Result".into(), c2.clone())],
                    chain,
                );
                let c3 = self.temp();
                self.emit(
                    "ANDAny",
                    vec![
                        ("A".into(), InputVal::Var(c1)),
                        ("B".into(), InputVal::Var(c2)),
                    ],
                    vec![("Result".into(), c3.clone())],
                    chain,
                );
                let if_name = self.emit(
                    "If",
                    vec![
                        ("Value 1".into(), InputVal::Var(c3)),
                        ("ComparisonType".into(),
                         InputVal::Literal { value: "==".into(), vtype: "String".into() }),
                        ("Value 2".into(),
                         InputVal::Literal { value: "1".into(), vtype: "Bool".into() }),
                    ],
                    vec![], chain,
                );
                self.push_scope();
                let mut then_chain = Vec::new();
                self.stmts(body, &mut then_chain);
                self.set_children(&if_name, then_chain);
                self.pop_scope();
                return;
            }
        }
        // Fusion (user ground truth): a plain comparison becomes the If's
        // own inputs directly - no separate Comparison block, no Bool gate
        // (gate blocks rendered "? == ?" and carried no information).
        // Compound conditions (and/or/not) and truthiness keep gates;
        // while-loops keep gates (the body-end recompute needs a variable).
        //
        // COMPOUND CONDITION FOLD: if the condition is a compound BinOp
        // (And/Or) or UnOp (Not) with all-safe operands (no Call/Method/
        // Table/Function - they need separate blocks), fold the WHOLE
        // condition into ONE SolveEquation. Saves N-1 blocks per compound
        // condition (vs emitting N Comparison + N-1 ANDAny/ORAny/NOTAny).
        //
        // The equation text is wrapped in `not not (...)` to force a Bool
        // result. SolveEquation's `and`/`or` short-circuit semantics may
        // return non-Bool values (Instance, etc.) - `not not` always
        // returns Bool true/false, so the If's `result == true` check
        // (built by Self::if_inputs) works correctly for all value types.
        //
        // Verified safe at runtime: the user confirmed SolveEquation
        // evaluates `not processed and input.KeyCode == Enum.KeyCode.Q`
        // correctly in the event test.
        let is_compound = match cond {
            Expr::BinOp(op, _, _) => matches!(op, BinOp::And | BinOp::Or),
            Expr::UnOp(UnOp::Not, _) => true,
            _ => false,
        };
        // [patched: eq-truthy-wrap v2 - supersede the v1 blanket refusal.
        // v1 refused the fold whenever an and/or operand was a bare atom,
        // so `if a and b then` fell back to Comparison+ANDAny and the
        // upstream regression guard
        // transpile_compound_condition_uses_solveequation_fold failed.
        // Instead, rewrite bare Name operands into the game-proven
        // truthiness comparison `(x ~= nil and x ~= false)` (Solve
        // Equation23/67/68/99/101/116 ran that exact text): after wrapping,
        // EVERY and/or operand is non-bare - the constraint RS's equation
        // parser actually enforces ("Couldn't figure this out!" on Solve
        // Equation26 `light and light.Enabled` and Eq36
        // `doInterlace and (((i2 + j2) % 2) ~= interlacePhase)`, while
        // Eq8/23/45/112 comparison-shaped operands compiled). Conditions
        // that cannot be rewritten (Number/Bool/Nil/Vararg directly under
        // and/or) still fall back to the Comparison+ANDAny decomposition.]
        let folded_inputs = if is_compound && is_safe_for_eq_fold(cond) {
            match eq_wrap_bare_names(cond) {
                Some(wrapped) if is_eq_compilable_condition(&wrapped) => {
                    let cond_var = self.temp();
                    let eq_text = if returns_bool(&wrapped) {
                        self.eq_expr(&wrapped)
                    } else {
                        let cond_text = self.eq_expr(&wrapped);
                        format!("((({}) ~= nil) and (({}) ~= false))", cond_text, cond_text)
                    };
                    self.folded += 1;
                    self.emit(
                        "SolveEquation",
                        vec![(
                            "Equation".into(),
                            InputVal::Literal { value: eq_text, vtype: "String".into() },
                        )],
                        vec![("Result".into(), cond_var.clone())],
                        chain,
                    );
                    Some(Self::if_inputs(&cond_var))
                }
                _ => None,
            }
        } else {
            None
        };
        let inputs = match folded_inputs {
            Some(i) => i,
            None => match self.direct_if_inputs(cond, chain) {
                Some(i) => i,
                None => {
                    let cond_var = self.condition(cond, chain);
                    Self::if_inputs(&cond_var)
                }
            },
        };
        let if_name = self.emit("If", inputs, vec![], chain);
        self.push_scope();
        let mut then_chain = Vec::new();
        self.stmts(body, &mut then_chain);
        self.set_children(&if_name, then_chain);
        self.pop_scope();

        let rest = &arms[1..];
        if !rest.is_empty() || els.is_some() {
            let mut else_chain = Vec::new();
            if !rest.is_empty() {
                self.if_stmt(rest, els, &mut else_chain);
            } else {
                self.cse_map.clear();
                self.push_scope();
                let mut body_chain = Vec::new();
                self.stmts(els.as_ref().unwrap(), &mut body_chain);
                self.pop_scope();
                else_chain = body_chain;
            }
            // Link the else chain sequentially. Supersedes the B6
            // windows(2) loop (user-caught B6: elseif condition blocks after
            // the first were left unparented at ROOT, so an `or` operand
            // evaluated outside its branch).
            //
            // [patched: else-chain v7 - the windows loop SKIPPED the link whenever
            // the previous block "routes" (has children) - which is ALWAYS
            // true for while/for/if bodies - so statements after a while
            // loop inside an else branch were left UNPARENTED. The finish
            // pass promoted them to startup roots and they executed ONCE at
            // spawn with every local/parameter unset (user-caught: Get Table
            // Value39 "Table does not exist" + Solve Equation115-118
            // "sub on number and nil" - the raycast half of updateGrid ran
            // at startup). chain_walk already implements the sequence model
            // (BODY continuation = siblings via the container); route the
            // else chain through it, with body_opens_container=true because
            // an else body may start with an if/while and the else slot is
            // a single ElseChildBlock.
            let else_heads = chain_walk_with(else_chain, &self.index, &mut self.blocks, true);
            if let Some(first) = else_heads.first().cloned() {
                self.set_else(&if_name, first);
            }
            if else_heads.len() > 1 {
                self.note(
                    "VERIFY",
                    format!(
                        "else chain split into {} runs (INERT block inside an else                          body?); only the first run is wired as the else branch",
                        else_heads.len()
                    ),
                );
            }
        }
    }

        fn while_stmt(&mut self, cond: &Expr, body: &[Stmt], chain: &mut Vec<String>) {
        // `while true` (user optimization): the condition IS true == true -
        // no Comparison gate block, no body-end recompute.
        let is_true = matches!(cond, Expr::Bool(true));
        let (wl, gate) = if is_true {
            let wl = self.emit(
                "WhileLoop3",
                vec![
                    ("Value 1".into(),
                     InputVal::Literal { value: "1".into(), vtype: "Bool".into() }),
                    ("ComparisonType".into(),
                     InputVal::Literal { value: "==".into(), vtype: "String".into() }),
                    ("Value 2".into(),
                     InputVal::Literal { value: "1".into(), vtype: "Bool".into() }),
                ],
                vec![],
                chain,
            );
            (wl, None)
        } else {
            let gate = self.temp();
            self.condition_to(cond, gate.clone(), chain);
            let wl = self.emit("WhileLoop3", Self::if_inputs(&gate), vec![], chain);
            (wl, Some(gate))
        };
        self.push_scope();
        self.cse_map.clear();
        let mut body_chain = Vec::new();
        self.stmts(body, &mut body_chain);
        self.cse_map.clear();
        // recompute the gate so the next iteration sees body writes
        // (constant `while true` needs none)
        if let Some(gate) = gate {
            self.condition_to(cond, gate, &mut body_chain);
        }
        self.set_children(&wl, body_chain);
        self.pop_scope();
    }


    fn numeric_for(
        &mut self,
        var: &str,
        from: &Expr,
        to: &Expr,
        step: Option<&Expr>,
        body: &[Stmt],
        chain: &mut Vec<String>,
    ) {
        let fv = self.expr(from, chain, None);
        let tv = self.expr(to, chain, None);
        let step_val = match step {
            Some(e) => match self.const_num(e) {
                Some(n) => n,
                None => match self.expr(e, chain, None) {
                    Val::Lit(LitVal::Num(n)) => n,
                    _ => 1.0,
                },
            },
            None => 1.0,
        };
        let binding = self.unique_binding(var);
        let name = self.emit(
            "ForLoop2",
            vec![
                ("Initial".into(), self.val_input(&fv, "Number")),
                (
                    "Increment".into(),
                    InputVal::Literal { value: hexs(step_val), vtype: "Number".into() },
                ),
                ("Maximum".into(), self.val_input(&tv, "Number")),
            ],
            vec![("Place".into(), binding.clone())],
            chain,
        );
        if step_val < 0.0 {
            self.note(
                "VERIFY",
                "descending ForLoop2 (negative Increment) - confirm the game counts down".into(),
            );
        }
        self.push_scope();
        self.cse_map.clear();
        self.declare(var, binding);
        let mut body_chain = Vec::new();
        self.stmts(body, &mut body_chain);
        self.set_children(&name, body_chain);
        self.pop_scope();
    }

    fn generic_for(
        &mut self,
        names: &[String],
        exprs: &[Expr],
        body: &[Stmt],
        chain: &mut Vec<String>,
    ) {
        let inner = match exprs {
            [Expr::Call(f, args)] if matches!(**f, Expr::Name(ref n) if n == "ipairs" || n == "pairs") => {
                match args.len() {
                    1 => &args[0],
                    _ => {
                        self.note("GAP", "ipairs with extra args".into());
                        return;
                    }
                }
            }
            _ => {
                self.note("GAP", "generic for over non-ipairs expression".into());
                return;
            }
        };

        // `for _, x in ipairs(O:GetChildren())` -> LoopThroughChildren
        let (btype, table_input): (&str, InputVal) =
            if let Expr::Method(base, m, _args) = inner {
                if m == "GetChildren" {
                    let basev = self.expr(base, chain, None);
                    ("LoopThroughChildren", self.val_input(&basev, "Object"))
                } else {
                    let v = self.expr(inner, chain, None);
                    ("LoopThroughTable", self.val_input(&v, "Table"))
                }
            } else {
                let v = self.expr(inner, chain, None);
                ("LoopThroughTable", self.val_input(&v, "Table"))
            };

        let mut outputs = Vec::new();
        let mut decls: Vec<(String, String)> = Vec::new();
        let valid = |n: &String| !n.is_empty() && n != "_";
        if btype == "LoopThroughChildren" {
            let value_name = names.get(1).or_else(|| names.first()).cloned().unwrap_or_default();
            if valid(&value_name) {
                let b = self.unique_binding(&value_name);
                outputs.push(("Child".to_string(), b.clone()));
                decls.push((value_name, b));
            }
        } else {
            let value_name = names.get(1).cloned().unwrap_or_default();
            let index_name = names.first().cloned().unwrap_or_default();
            if valid(&value_name) {
                let b = self.unique_binding(&value_name);
                outputs.push(("Value".to_string(), b.clone()));
                decls.push((value_name, b));
            }
            if valid(&index_name) && Some(&index_name) != names.get(1) {
                let b = self.unique_binding(&index_name);
                outputs.push(("Index".to_string(), b.clone()));
                decls.push((index_name, b));
            }
        }
        let input_name = if btype == "LoopThroughChildren" { "Object" } else { "Table" };
        let name = self.emit(btype, vec![(input_name.to_string(), table_input)], outputs, chain);
        self.push_scope();
        self.cse_map.clear();
        for (lua, binding) in decls {
            self.scopes.last_mut().unwrap().insert(lua, binding);
        }
        let mut body_chain = Vec::new();
        self.stmts(body, &mut body_chain);
        self.set_children(&name, body_chain);
        self.pop_scope();
    }

    fn function(
        &mut self,
        name: &str,
        params: &[String],
        body: &[Stmt],
        chain: &mut Vec<String>,
    ) {
        // closure-return specialization: `return function(p) ... end`.
        // The inner function is defined INSIDE the outer's scope so its body
        // resolves the outer's parameters to the same shared variables.
        if let Some(Stmt::Return(rets)) = body.last() {
            if let [Expr::Function(inner_params, inner_body)] = rets.as_slice() {
                let outer_body: Vec<Stmt> = body[..body.len() - 1].to_vec();
                let outer_b = self.define_function_scoped(name, params, &outer_body, true);
                let inner_name = format!("{}Inner", name);
                let inner_b =
                    self.define_function(&inner_name, inner_params, inner_body, chain);
                self.pop_scope(); // close the outer scope opened for the inner
                // re-declare both handles in the enclosing (persistent) scope so
                // call sites `f(a, b)(w)` resolve them
                let scope = self.scopes.last_mut().unwrap();
                scope.insert(name.to_string(), outer_b);
                scope.insert(inner_name, inner_b);
                self.note(
                    "INFO",
                    format!(
                        "{} returns a closure; split into {} + {}Inner with \
                         immediately-invoked call sites rewired",
                        name, name, name
                    ),
                );
                return;
            }
        }
        self.define_function(name, params, body, chain);
    }

    fn define_function_scoped(
        &mut self,
        name: &str,
        params: &[String],
        body: &[Stmt],
        keep_scope: bool,
    ) -> String {
        // variant of define_function that leaves the function scope open so a
        // nested definition (closure specialization) can resolve its locals
        let binding = self.unique_binding(name);
        // params bind up front; RS exposes them as TUPLEPARAM_Parameters_i
        // outputs on the DefineFunction block (user ground truth, T8)
        let param_bindings: Vec<String> =
            params.iter().map(|p| self.unique_binding(p)).collect();
        let fname = {
            self.validate("DefineFunction", &[]);
            let n = self.display_name("DefineFunction");
            let mut outs = vec![
                ("Function".into(), binding.clone()),
                ("TUPLE_Parameters".into(), params.len().to_string()),
            ];
            for (i, pb) in param_bindings.iter().enumerate() {
                outs.push((format!("TUPLEPARAM_Parameters_{}", i + 1), pb.clone()));
            }
            self.blocks.push(EmittedBlock {
                name: n.clone(),
                btype: "DefineFunction".into(),
                inputs: vec![],
                outputs: outs,
                children: Vec::new(),
                else_child: None,
            });
            self.index.insert(n.clone(), self.blocks.len() - 1);
            n
        };
        self.push_scope();
        let mut body_chain = Vec::new();
        // params arrive as TUPLEPARAM_Parameters_i outputs on the
        // DefineFunction block (RS convention, user ground truth T8) -
        // bind them into scope directly. No TablePack/GetTableValue:
        // packing Var("0") inside a function body references a variable
        // that does not exist there (verified broken in RetroStudio).
        for (p, pb) in params.iter().zip(param_bindings.iter()) {
            self.declare(p, pb.clone());
        }
        // [patch13] params used as Lua tables: TRACK their TUPLEPARAM
        // bindings for SetTableValue dispatch (the binding IS the
        // variable, wire semantics) - no materialization block. v12
        // dropped the tracking along with the block, which sent writes
        // like t[2] = "x" to SetObjectProperty (non-string property key
        // on Instance GAP).
        let tab_params = self.params_used_as_tables(params, body);
        for (p, pb) in params.iter().zip(param_bindings.iter()) {
            if tab_params.contains(p) {
                self.lua_tables.insert(pb.clone());
            }
        }
        // [patch10] attribute Return-statement table-ness to this function
        self.fn_stack.push(binding.clone());
        self.stmts(body, &mut body_chain);
        self.fn_stack.pop();
        self.set_children(&fname, body_chain);
        // record return arity for call-site TUPLEPARAM_ReturnedValues_i
        let arity = body
            .last()
            .map(|s| match s {
                Stmt::Return(exprs) => exprs.len(),
                _ => 0,
            })
            .unwrap_or(0);
        self.fn_returns.insert(binding.clone(), arity);
        // [patch10] record 1:1 param passthru returns for call-site
        // table-ness propagation (`local t = wrap(arg)`).
        if let Some(Stmt::Return(exprs)) = body.last() {
            if exprs.len() == 1 {
                if let Expr::Name(pn) = &exprs[0] {
                    if let Some(idx) = params.iter().position(|p| p == pn) {
                        self.fn_ret_passthru.insert(binding.clone(), idx);
                    }
                }
            }
        }
        if !keep_scope {
            self.pop_scope();
            self.declare(name, binding.clone());
        } else {
            // declare into the still-open scope so the inner body sees the
            // function handle too
            self.scopes.last_mut().unwrap().insert(name.to_string(), binding.clone());
        }
        binding
    }

    fn define_function(
        &mut self,
        name: &str,
        params: &[String],
        body: &[Stmt],
        chain: &mut Vec<String>,
    ) -> String {
        // DefineFunction blocks are roots; block_order picks them up.
        let binding = self.unique_binding(name);
        // params bind up front; RS exposes them as TUPLEPARAM_Parameters_i
        // outputs on the DefineFunction block (user ground truth, T8)
        let param_bindings: Vec<String> =
            params.iter().map(|p| self.unique_binding(p)).collect();
        let fname = {
            self.validate("DefineFunction", &[]);
            let n = self.display_name("DefineFunction");
            let mut outs = vec![
                ("Function".into(), binding.clone()),
                ("TUPLE_Parameters".into(), params.len().to_string()),
            ];
            for (i, pb) in param_bindings.iter().enumerate() {
                outs.push((format!("TUPLEPARAM_Parameters_{}", i + 1), pb.clone()));
            }
            self.blocks.push(EmittedBlock {
                name: n.clone(),
                btype: "DefineFunction".into(),
                inputs: vec![],
                outputs: outs,
                children: Vec::new(),
                else_child: None,
            });
            self.index.insert(n.clone(), self.blocks.len() - 1);
            n
        };
        // NEST: a LocalFunction declared inside a body is created at
        // this execution point (user-caused logical bug: hoisting made
        // FunctionReturn reference an outer-scope variable and let
        // multiple invocations share one upvalue). Top-level calls
        // pass the roots list, so top-level functions stay roots.
        chain.push(fname.clone());
        self.push_scope();
        let mut body_chain = Vec::new();
        // params arrive as TUPLEPARAM_Parameters_i outputs on the
        // DefineFunction block (RS convention, user ground truth T8) -
        // bind them into scope directly. No TablePack/GetTableValue:
        // packing Var("0") inside a function body references a variable
        // that does not exist there (verified broken in RetroStudio).
        for (p, pb) in params.iter().zip(param_bindings.iter()) {
            self.declare(p, pb.clone());
        }
        // [patch13] params used as Lua tables: TRACK their TUPLEPARAM
        // bindings for SetTableValue dispatch (the binding IS the
        // variable, wire semantics) - no materialization block. v12
        // dropped the tracking along with the block, which sent writes
        // like t[2] = "x" to SetObjectProperty (non-string property key
        // on Instance GAP).
        let tab_params = self.params_used_as_tables(params, body);
        for (p, pb) in params.iter().zip(param_bindings.iter()) {
            if tab_params.contains(p) {
                self.lua_tables.insert(pb.clone());
            }
        }
        // [patch10] attribute Return-statement table-ness to this function
        self.fn_stack.push(binding.clone());
        self.stmts(body, &mut body_chain);
        self.fn_stack.pop();
        self.set_children(&fname, body_chain);
        // record return arity for call-site TUPLEPARAM_ReturnedValues_i
        let arity = body
            .last()
            .map(|s| match s {
                Stmt::Return(exprs) => exprs.len(),
                _ => 0,
            })
            .unwrap_or(0);
        self.fn_returns.insert(binding.clone(), arity);
        // [patch10] record 1:1 param passthru returns for call-site
        // table-ness propagation (`local t = wrap(arg)`).
        if let Some(Stmt::Return(exprs)) = body.last() {
            if exprs.len() == 1 {
                if let Expr::Name(pn) = &exprs[0] {
                    if let Some(idx) = params.iter().position(|p| p == pn) {
                        self.fn_ret_passthru.insert(binding.clone(), idx);
                    }
                }
            }
        }
        self.pop_scope();
        self.declare(name, binding.clone());
        // RUNTIME BUG FIX: record this function's source parameter list so that
        // a later `Event:Connect(name)` (named function reference, not inline
        // closure) can use the same parameter list to bind the event's outputs
        // and pass them as arguments to the function call.
        self.function_params.insert(name.to_string(), params.to_vec());
        binding
    }

    // ----- conditions -----

    fn condition(&mut self, cond: &Expr, chain: &mut Vec<String>) -> String {
        let t = self.temp();
        self.condition_to(cond, t, chain)
    }

    fn condition_to(&mut self, cond: &Expr, out: String, chain: &mut Vec<String>) -> String {
        match cond {
            Expr::BinOp(BinOp::And, a, b) => {
                let av = self.condition(a, chain);
                let bv = self.condition(b, chain);
                self.emit(
                    "ANDAny",
                    vec![("A".into(), InputVal::Var(av)), ("B".into(), InputVal::Var(bv))],
                    vec![("Result".into(), out.clone())],
                    chain,
                );
                out
            }
            Expr::BinOp(BinOp::Or, a, b) => {
                let av = self.condition(a, chain);
                let bv = self.condition(b, chain);
                self.emit(
                    "ORAny",
                    vec![("A".into(), InputVal::Var(av)), ("B".into(), InputVal::Var(bv))],
                    vec![("Result".into(), out.clone())],
                    chain,
                );
                out
            }
            Expr::UnOp(UnOp::Not, e) => {
                let v = self.condition(e, chain);
                self.emit(
                    "NOTAny",
                    vec![("A".into(), InputVal::Var(v))],
                    vec![("Result".into(), out.clone())],
                    chain,
                );
                out
            }
            Expr::BinOp(op, a, b)
                if matches!(
                    op,
                    BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge
                ) =>
            {
                let av = self.expr(a, chain, None);
                let bv = self.expr(b, chain, None);
                let ct = match op {
                    BinOp::Eq => "==",
                    BinOp::Ne => "~=",
                    BinOp::Lt => "<",
                    BinOp::Le => "<=",
                    BinOp::Gt => ">",
                    BinOp::Ge => ">=",
                    _ => unreachable!(),
                };
                self.emit(
                    "Comparison",
                    vec![
                        ("Value 1".into(), self.val_input(&av, "Any")),
                        (
                            "ComparisonType".into(),
                            InputVal::Literal { value: ct.into(), vtype: "String".into() },
                        ),
                        ("Value 2".into(), self.val_input(&bv, "Any")),
                    ],
                    vec![("Result".into(), out.clone())],
                    chain,
                );
                out
            }
            other => {
                // RUNTIME BUG FIX: previous code emitted a single `v ~= nil` comparison,
                // which is true for ALL non-nil values including Bool `false`. This broke
                // `if child:IsA("SpecialMesh")` - the If fired for ALL children, not just
                // SpecialMesh ones, so DestroyObject attempted to delete Head attachments
                // (BodyBackAttachment etc.) and ultimately cascaded to deleting the local
                // player's character. Proper Luau truthiness: truthy <=> (v ~= nil) AND
                // (v ~= false). For Bool (e.g. IsA result), this returns true only when
                // v is true. The pattern matches truthy_gate_var's user-proven shape.
                let v = self.expr(other, chain, None);
                match &v {
                    Val::Var(b) => {
                        // [patched: truthy equation gate v1 - OPTIMIZATION]
                        // One SolveEquation instead of 2 Comparison + 1
                        // ANDAny (saves 2 blocks per site, incl. per-iteration
                        // while-loop gate recomputes). Game-proven text for
                        // PLAIN bindings: the imported payload ran Solve
                        // Equation67/68/99/101/116 of exactly this form.
                        // Safe here because b is a binding name; compound
                        // conditions with bare-atom and/or operands are kept
                        // out of equations by is_eq_compilable_condition.
                        let eq = format!("({} ~= nil and {} ~= false)", b, b);
                        self.emit(
                            "SolveEquation",
                            vec![(
                                "Equation".into(),
                                InputVal::Literal { value: eq, vtype: "String".into() },
                            )],
                            vec![("Result".into(), out.clone())],
                            chain,
                        );
                        out
                    }
                    Val::Lit(l) => {
                        let truthy = Self::lit_truthy(l);
                        self.emit(
                            "SetVariable1",
                            vec![("Value".into(), InputVal::Literal {
                                value: if truthy { "1".into() } else { "0".into() },
                                vtype: "Bool".into() })],
                            vec![("VariableName".into(), out.clone())],
                            chain,
                        );
                        out
                    }
                    _ => {
                        // For Path/Tuple/other Val types we cannot inspect at transpile
                        // time; fall back to the ~= nil check (these types are never Bool
                        // false, so the false-skip is unnecessary here).
                        self.emit(
                            "Comparison",
                            vec![
                                ("Value 1".into(), self.val_input(&v, "Any")),
                                ("ComparisonType".into(), InputVal::Literal { value: "~=".into(), vtype: "String".into() }),
                                ("Value 2".into(), InputVal::Literal { value: String::new(), vtype: "Nil".into() }),
                            ],
                            vec![("Result".into(), out.clone())],
                            chain,
                        );
                        out
                    }
                }
            }
        }
    }

    // ----- expressions -----

    fn const_num(&self, e: &Expr) -> Option<f64> {
        match e {
            Expr::Number(n) => Some(*n),
            Expr::UnOp(UnOp::Neg, inner) => self.const_num(inner).map(|v| -v),
            _ => None,
        }
    }

    fn const_lit(&self, e: &Expr) -> Option<LitVal> {
        match e {
            Expr::Nil => Some(LitVal::Nil),
            Expr::Number(n) => Some(LitVal::Num(*n)),
            Expr::Str(s) => Some(LitVal::Str(s.clone())),
            Expr::Bool(b) => Some(LitVal::Bool(*b)),
            Expr::UnOp(UnOp::Neg, inner) => self.const_num(inner).map(|v| LitVal::Num(-v)),
            _ => None,
        }
    }

    fn expr_key(e: &Expr) -> Option<String> {
        fn walk(e: &Expr, out: &mut String) -> bool {
            match e {
                Expr::Name(n) => {
                    out.push_str(n);
                    true
                }
                Expr::Number(n) => {
                    out.push_str(&format!("#{:?}", n));
                    true
                }
                Expr::Str(s) => {
                    out.push_str(&format!("{:?}", s));
                    true
                }
                Expr::Index(b, k) => {
                    walk(b, out) && {
                        out.push('.');
                        walk(k, out)
                    }
                }
                Expr::BinOp(op, a, b) => {
                    walk(a, out) && {
                        out.push_str(&format!(" op{:?} ", op));
                        walk(b, out)
                    }
                }
                _ => false,
            }
        }
        let mut s = String::new();
        if walk(e, &mut s) {
            Some(s)
        } else {
            None
        }
    }

    pub fn expr(&mut self, e: &Expr, chain: &mut Vec<String>, bind: Option<&str>) -> Val {
        match e {
            Expr::Nil => Val::Lit(LitVal::Nil),
            Expr::Bool(b) => Val::Lit(LitVal::Bool(*b)),
            Expr::Number(n) => Val::Lit(LitVal::Num(*n)),
            Expr::Str(s) => Val::Lit(LitVal::Str(s.clone())),
            Expr::Vararg => {
                self.note("GAP", "vararg (...) not supported".into());
                return Val::Lit(LitVal::Str(String::new()));
            }
            Expr::Paren(inner) => self.expr(inner, chain, bind),
            Expr::Name(n) => {
                if let Some(b) = self.lookup(n) {
                    return Val::Var(b.clone());
                }
                match n.as_str() {
                    "game" | "script" | "workspace" | "Enum"
                    | "math" | "Vector3" | "Vector2" | "Color3" | "UDim2"
                    | "CFrame" | "RaycastParams" | "BrickColor" | "Random"
                    | "Instance" | "string" | "table" | "task" | "os"
                    | "coroutine" | "SharedTable" => Val::Path(n.clone()),
                    _ => {
                        self.note("GAP", format!("reference to undeclared variable {:?}", n));
                        return Val::Var(sanitize(n));
                    }
                }
            }
            Expr::Index(base, key) => self.index_expr(base, key, chain, bind),
            Expr::Call(f, args) => self.call_expr(f, args, chain, bind),
            Expr::Method(base, m, args) => self.method_expr(base, m, args, chain, bind),
            Expr::Function(params, body) => {
                let name = format!("closure{}", self.temp_counter + 1);
                self.function(&name, params, body, chain);
                self.note(
                    "WORKAROUND",
                    format!("inline closure converted to DefineFunction {}", name),
                );
                match self.lookup(&name) {
                    Some(b) => Val::Var(b.clone()),
                    None => Val::Var(name),
                }
            }
            Expr::Table(fields) => self.table_expr(fields, chain, bind),
            Expr::UnOp(op, inner) => self.unop(*op, inner, chain, bind),
            Expr::BinOp(op, a, b) => self.binop(*op, a, b, chain, bind),
        }
    }

    fn unop(&mut self, op: UnOp, inner: &Expr, chain: &mut Vec<String>, bind: Option<&str>) -> Val {
        match op {
            UnOp::Neg => {
                if let Some(n) = self.const_num(&Expr::UnOp(UnOp::Neg, Box::new(inner.clone()))) {
                    return Val::Lit(LitVal::Num(n));
                }
                let v = self.expr(inner, chain, None);
                let out = bind.map(|s| s.to_string()).unwrap_or_else(|| self.temp());
                self.emit(
                    "Subtraction",
                    vec![
                        ("Number1".into(), InputVal::Literal { value: "0".into(), vtype: "Number".into() }),
                        ("Number2".into(), self.val_input(&v, "Number")),
                    ],
                    vec![("Result".into(), out.clone())],
                    chain,
                );
                return Val::Var(out);
            }
            UnOp::Len => {
                let v = self.expr(inner, chain, None);
                let out = bind.map(|s| s.to_string()).unwrap_or_else(|| self.temp());
                // [patch14] GetLength is STRING-only in RetroStudio: on a
                // table operand it yields 0 (run 4: n = #r on a returned
                // table -> inner fill loop ran zero times -> empty embedded
                // tables in LASTS). SolveEquation evaluates the raw Lua '#'
                // operator and reads both strings and tables (user
                // experiment: Equation '#table' and '#string' both correct),
                // so no static type guess is needed. Literals keep GetLength
                // (avoids quoting inside equation text).
                if let Val::Var(name) = &v {
                    self.emit(
                        "SolveEquation",
                        vec![(
                            "Equation".into(),
                            InputVal::Literal {
                                value: format!("#{}", name),
                                vtype: "String".into(),
                            },
                        )],
                        vec![("Result".into(), out.clone())],
                        chain,
                    );
                } else {
                    self.emit(
                        "GetLength",
                        vec![("String".into(), self.val_input(&v, "String"))],
                        vec![("Length".into(), out.clone())],
                        chain,
                    );
                }
                return Val::Var(out);
            }
            UnOp::Not => {
                let c = self.condition(inner, chain);
                let out = bind.map(|s| s.to_string()).unwrap_or_else(|| self.temp());
                self.emit(
                    "NOTAny",
                    vec![("A".into(), InputVal::Var(c))],
                    vec![("Result".into(), out.clone())],
                    chain,
                );
                return Val::Var(out);
            }
        }
    }

    fn binop(
        &mut self,
        op: BinOp,
        a: &Expr,
        b: &Expr,
        chain: &mut Vec<String>,
        bind: Option<&str>,
    ) -> Val {
        // constant folding
        if let (Some(x), Some(y)) = (self.const_num(a), self.const_num(b)) {
            let v = match op {
                BinOp::Add => Some(x + y),
                BinOp::Sub => Some(x - y),
                BinOp::Mul => Some(x * y),
                BinOp::Div => Some(x / y),
                // [patch19] Lua modulo is FLOORED, Rust's is truncated:
                // (-7) % 3 is 2 under Lua/RS semantics, -1 under Rust.
                // RS equations run Lua semantics (user-proven, run 7
                // E3), so the fold must match or a const-folded literal
                // disagrees with the equation the same source produces
                // once its operands stop being constants.
                BinOp::Mod => {
                    if y == 0.0 {
                        None
                    } else {
                        let r = x % y;
                        Some(if r != 0.0 && (r < 0.0) != (y < 0.0) { r + y } else { r })
                    }
                }
                BinOp::Pow => Some(x.powf(y)),
                _ => None,
            };
            if let Some(v) = v {
                return Val::Lit(LitVal::Num(v));
            }
        }
        match op {
            BinOp::Concat => {
                let mut parts = Vec::new();
                self.flatten_concat(a, &mut parts, chain);
                self.flatten_concat(b, &mut parts, chain);
                if parts.is_empty() {
                    return Val::Lit(LitVal::Str(String::new()));
                } else if parts.len() == 1 {
                    return match &parts[0] {
                        InputVal::Var(v) => Val::Var(v.clone()),
                        InputVal::Literal { value, .. } | InputVal::Implicit(value) => {
                            Val::Lit(LitVal::Str(value.clone()))
                        }
                    };
                } else {
                    // [patch17] RS Concatenate requires BOTH inputs to be
                    // strings at runtime ("SecondString" input of
                    // "Concatenate" must be a "string" got "number" - run 6,
                    // check(id, ...) building "[..id..]" with a numeric id).
                    // Luau's `..` coerces numbers, so route every var operand
                    // through ToStringAny (identity on strings, tostring on
                    // numbers/bools) - the same policy the print path already
                    // applies to vars (Bug C, bak63).
                    for p in parts.iter_mut() {
                        if let InputVal::Var(v) = p {
                            let s = self.temp();
                            self.emit(
                                "ToStringAny",
                                vec![("Input".into(), InputVal::Var(v.clone()))],
                                vec![("String".into(), s.clone())],
                                chain,
                            );
                            *p = InputVal::Var(s);
                        }
                    }
                    let mut cur = parts[0].clone();
                    let final_out = bind.map(|s| s.to_string()).unwrap_or_else(|| self.temp());
                    for (i, next) in parts[1..].iter().enumerate() {
                        let step_out = if i == parts.len() - 2 {
                            final_out.clone()
                        } else {
                            self.temp()
                        };
                        self.emit(
                            "Concatenate",
                            vec![("FirstString".into(), cur), ("SecondString".into(), next.clone())],
                            vec![("Result".into(), step_out.clone())],
                            chain,
                        );
                        cur = InputVal::Var(step_out);
                    }
                    return Val::Var(final_out);
                }
            }
            BinOp::And | BinOp::Or => {
                let out = bind.map(|s| s.to_string()).unwrap_or_else(|| self.temp());
                self.andor_value_expr(
                    &Expr::BinOp(op, Box::new(a.clone()), Box::new(b.clone())),
                    &out,
                    chain,
                );
                return Val::Var(out);
            }
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                let out = bind.map(|s| s.to_string()).unwrap_or_else(|| self.temp());
                self.condition_to(&Expr::BinOp(op, Box::new(a.clone()), Box::new(b.clone())), out.clone(), chain);
                return Val::Var(out);
            }
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod | BinOp::Pow => {
                let out = bind.map(|s| s.to_string()).unwrap_or_else(|| self.temp());
                if self.optimize {
                    if let Some(key) = Self::expr_key(&Expr::BinOp(op, Box::new(a.clone()), Box::new(b.clone()))) {
                        if let Some(binding) = self.cse_map.get(&key) {
                            self.cse_hits += 1;
                            return Val::Var(binding.clone());
                        }
                    }
                }
                // BIG OPTIMIZATION (block count): if both operands are
                // "safe to fold" (no Call/Method/Table/Function side
                // effects that need separate blocks), emit ONE
                // SolveEquation for the WHOLE BinOp tree instead of
                // recursing into operands and emitting per-op blocks.
                //
                // Before: `local x = a.X + (i - 0.5) * invG * b.Y`
                //   emitted 5 blocks:
                //     1 SolveEquation(a.X) + 1 Subtraction(i, 0.5)
                //     + 1 SolveEquation(sub*invG) + 1 SolveEquation(b.Y)
                //     + 1 SolveEquation(add)
                // After: 1 SolveEquation('a.X + (i - 0.5) * invG * b.Y')
                //
                // Verified at runtime: RS's SolveEquation resolves
                // variable names + property access + arithmetic in a
                // single equation (event test: 'Players.LocalPlayer'
                // produced a real Player Instance; raycast test:
                // 'result.Instance/Position/Normal' resolved).
                //
                // We DO NOT fold when either operand contains a
                // Call/Method/Table/Function - those need separate
                // blocks for side effects / table construction / fn
                // definition.
                let a_safe = is_safe_for_eq_fold(a);
                let b_safe = is_safe_for_eq_fold(b);
                let a_simple = matches!(a, Expr::Name(_) | Expr::Number(_));
                let b_simple = matches!(b, Expr::Name(_) | Expr::Number(_));
                if a_safe && b_safe && !(a_simple && b_simple) {
                    let full = Expr::BinOp(op, Box::new(a.clone()), Box::new(b.clone()));
                    let eq = self.eq_expr(&full);
                    self.folded += 1;
                    self.emit(
                        "SolveEquation",
                        vec![(
                            "Equation".into(),
                            InputVal::Literal { value: eq, vtype: "String".into() },
                        )],
                        vec![("Result".into(), out.clone())],
                        chain,
                    );
                    if self.optimize {
                        if let Some(key) = Self::expr_key(&full) {
                            self.cse_map.insert(key, out.clone());
                        }
                    }
                    return Val::Var(out);
                }
                let av = self.expr(a, chain, None);
                let bv = self.expr(b, chain, None);
                if a_simple && b_simple {
                    let btype = match op {
                        BinOp::Add => "Addition",
                        BinOp::Sub => "Subtraction",
                        BinOp::Mul => "Multiplication",
                        BinOp::Div => "Division",
                        BinOp::Mod => "Modulus",
                        BinOp::Pow => "Exponentiation",
                        _ => unreachable!(),
                    };
                    self.emit(
                        btype,
                        vec![
                            ("Number1".into(), self.val_input(&av, "Number")),
                            ("Number2".into(), self.val_input(&bv, "Number")),
                        ],
                        vec![("Result".into(), out.clone())],
                        chain,
                    );
                } else {
                    let eq = format!(
                        "{} {} {}",
                        self.eq_operand(&av),
                        eq_op_str(op),
                        self.eq_operand(&bv)
                    );
                    self.folded += 1;
                    self.emit(
                        "SolveEquation",
                        vec![(
                            "Equation".into(),
                            InputVal::Literal { value: eq, vtype: "String".into() },
                        )],
                        vec![("Result".into(), out.clone())],
                        chain,
                    );
                }
                if self.optimize {
                    if let Some(key) = Self::expr_key(&Expr::BinOp(op, Box::new(a.clone()), Box::new(b.clone()))) {
                        self.cse_map.insert(key, out.clone());
                    }
                }
                return Val::Var(out);
            }
        }
    }

    fn eq_operand(&self, v: &Val) -> String {
        match v {
            Val::Var(b) => b.clone(),
            Val::Lit(LitVal::Num(n)) => format!("{}", n),
            Val::Lit(LitVal::Str(s)) => format!("{:?}", s),
            // [patched: typed eq literals v1 - non-number literals used to
            // render as "0" (observed: `hPos + Vector3.new(0, 2, 0)` became
            // `hPos + 0`, silently dropping the offset). Constructor calls
            // inside equations are game-proven: Solve Equation51
            // Vector3.new(nx, ny, (-1)), Eq69/78 Color3.new(...),
            // Eq3/4 UDim2.new(...).]
            Val::Lit(LitVal::Bool(b)) => {
                if *b {
                    "true".into()
                } else {
                    "false".into()
                }
            }
            Val::Lit(LitVal::Nil) => "nil".into(),
            Val::Lit(LitVal::Vector3(x, y, z)) => {
                format!("Vector3.new({}, {}, {})", x, y, z)
            }
            Val::Lit(LitVal::Vector2(x, y)) => format!("Vector2.new({}, {})", x, y),
            Val::Lit(LitVal::Color3(r, g, bl)) => {
                format!("Color3.new({}, {}, {})", r, g, bl)
            }
            Val::Lit(LitVal::UDim2(a, b, c, d)) => {
                format!("UDim2.new({}, {}, {}, {})", a, b, c, d)
            }
            Val::Lit(LitVal::BrickColor(n)) => format!("{}", n),
            // [patched: eq-safety v6 - user correction: equations DO support
            // object references (reads like `Players.LocalPlayer`,
            // `camera.CFrame` ran at runtime); only SETTING objects via
            // equations is unsupported, and object targets are kept out of
            // folds at the fold sites (block chains instead). Render the
            // stored dot-path - same convention as the Val::Path arm.
            Val::Lit(LitVal::ObjectRef(p)) => p.clone(),
            Val::Lit(_) => "0".into(),
            Val::Path(p) => p.clone(),
            Val::Tuple(Some(b)) => b.clone(),
            Val::Tuple(None) => String::new(),
        }
    }

    fn flatten_concat(
        &mut self,
        e: &Expr,
        parts: &mut Vec<InputVal>,
        chain: &mut Vec<String>,
    ) {
        if let Expr::BinOp(BinOp::Concat, a, b) = e {
            self.flatten_concat(a, parts, chain);
            self.flatten_concat(b, parts, chain);
            return;
        }
        let v = self.expr(e, chain, None);
        if let (Some(InputVal::Literal { value: prev, vtype: _ }), Val::Lit(LitVal::Str(s))) =
            (parts.last().cloned(), &v)
        {
            let merged = format!("{}{}", prev, s);
            if let Some(last) = parts.last_mut() {
                if let InputVal::Literal { value, .. } = last {
                    *value = merged;
                    return;
                }
            }
        }
        parts.push(self.val_input(&v, "String"));
    }

    /// `a and b` / `a or b` in value position: default + nested ifs.


        fn index_expr(
        &mut self,
        base: &Expr,
        key: &Expr,
        chain: &mut Vec<String>,
        bind: Option<&str>,
    ) -> Val {
        let out = bind.map(|s| s.to_string()).unwrap_or_else(|| self.temp());
        if let Some(p) = self.static_path(&Expr::Index(Box::new(base.clone()), Box::new(key.clone()))) {
            // Enum item access (Enum.<Type>.<Item>) -> the ITEM NAME string
            // (user ground truth T4/B3: Font = "SourceSans"). RESTORED: this
            // check was lost in the multi-hop rewrite (user-caught N5:
            // 'local m = Enum.Material.Neon' produced no producer block).
            let segs: Vec<&str> = p.split('.').collect();
            if segs.len() >= 3 && segs[0] == "Enum" {
                return Val::Lit(LitVal::Str(segs[2].to_string()));
            }
            return Val::Path(p);
        }
        // math.huge style constants
        if let (Expr::Name(ns), Expr::Str(prop)) = (base, key) {
            if ns == "math" && prop == "huge" {
                return Val::Lit(LitVal::Num(1e308));
            }
        }
        // Enum resolution: Enum.<Type>.<Item> evaluates to the string item name
        if let Expr::Str(prop) = key {
            if let Some(p) = self.static_path(base) {
                if p == "Enum" {
                    return Val::Path(format!("Enum.{}", prop));
                } else if p.starts_with("Enum.") {
                    return Val::Lit(LitVal::Str(prop.clone()));
                }
            }
        }
        // MULTI-HOP (user ground truth): chained property reads on variable
        // bases CANNOT be stacked GetObjectProperty calls - RS resolves deep
        // paths via SolveEquation with binding names (same convention as
        // constructor equations: UDim2.new(0.5, w, 1, -14)). Count hops: if
        // the total path from the innermost VARIABLE base is >= 2 property
        // accesses, emit ONE SolveEquation.
        fn count_hops(base: &Expr) -> usize {
            match base {
                Expr::Index(_, _) => 1 + count_hops(match base {
                    Expr::Index(b, _) => b,
                    _ => unreachable!(),
                }),
                _ => 0,
            }
        }
        // [patched: eq-safety v6 - true when every key from this Index down to
        // the non-Index base is a Str literal (dot-renderable in an
        // equation). Non-Str keys would render `t[expr]` - banned.
        // [patch17] a Str key must ALSO be identifier-shaped: the fold
        // renders `base.key` and RS equations reject non-identifier
        // members (CH2B.0 / CH2B.+ / CH2B.( - run 6 charcode anchors).
        fn index_chain_all_str_keys(base: &Expr, key: &Expr) -> bool {
            if !matches!(key, Expr::Str(s) if rs_eq_member_ok(s)) {
                return false;
            }
            match base {
                Expr::Index(b, k) => index_chain_all_str_keys(b, k),
                _ => true,
            }
        }
        let hops = count_hops(&Expr::Index(Box::new(base.clone()), Box::new(key.clone())));
        // [patched: eq-safety v6 - multi-hop chains fold into ONE equation only
        // when EVERY key along the chain is a Str (dot-rendered, e.g.
        // `CONFIG.PartSize.X`, `l.color.B` - game-proven). Chains with
        // computed keys rendered `t[a][b]` and nilled out at runtime
        // (Solve Equation93/98). The fall-through below recurses into
        // `base` (emitting inner hops as their own proven single-hop
        // reads) and handles the outer hop via GetTableValue /
        // SolveEquation `<row>.<prop>`.
        if hops >= 2 && index_chain_all_str_keys(base, key) {
            // multi-hop: ONE SolveEquation. User ground truth: equations CAN
            // index Lua tables (CONFIG.PartSize.X is valid). The original N2
            // failure was the OUTPUT NAME - RS's SolveEquation writes through
            // its "Result" output; we emitted "Value", the result never
            // landed, and downstream reads got nil ('table index is nil').
            let eq = self.eq_expr(&Expr::Index(Box::new(base.clone()), Box::new(key.clone())));
            // CSE for multi-hop property access (Opportunity #4): cache
            // key is the equation text (e.g. "config.Toggle.Value").
            // Same invalidation points as single-hop CSE.
            if self.optimize {
                if let Some(cached) = self.cse_map.get(&eq) {
                    self.cse_hits += 1;
                    return Val::Var(cached.clone());
                }
            }
            self.emit(
                "SolveEquation",
                vec![("Equation".into(),
                      InputVal::Literal { value: eq.clone(), vtype: "String".into() })],
                vec![("Result".into(), out.clone())],
                chain,
            );
            if self.optimize {
                self.cse_map.insert(eq, out.clone());
            }
            return Val::Var(out);
        }
        let basev = self.expr(base, chain, None);
        // [patched: eq-safety v6 - when this call is the FINAL hop of a decomposed
        // bracket chain, its result must NOT be marked as a Lua table: the
        // cell can be an Instance (`squares[i2][j2]` is a Frame whose
        // BackgroundColor3 is set via SetObjectProperty - payload-proven).
        // Intermediate hops keep the marking (rows ARE tables, so
        // `squares[i][j] = sq` still routes to SetTableValue).
        let decomposed_leaf = hops >= 2 && !index_chain_all_str_keys(base, key);
        match key {
            // [patch17] RS equation member syntax only accepts a bare
            // identifier after the dot. Bracket reads with other string
            // keys (CH2B["0"] / CH2B["+"] / CH2B["("]) rendered
            // CH2B.0 / CH2B.+ / CH2B.( and failed RS's equation compiler
            // ("Couldn't figure this out CH2B." / "Unequal amount of
            // closing and opening parenthesis" - run 6 charcode anchors).
            // Identifier-shaped keys keep the game-proven dot-equation
            // route; everything else falls through to the GetTableValue
            // arm below - the same proven shape variable-key reads use.
            Expr::Str(prop) if rs_eq_member_ok(prop) => {
                // Variable bases: route through SolveEquation ('binding.Prop'
                // - RS resolves both instance paths AND Lua-table members in
                // equations; SimonUI proved instance paths, N2/N5+user error
                // proved GetObjectProperty is instances-only so table values
                // MUST ride equations). Implicit bases (static
                // game/workspace paths) keep GetObjectProperty (T1-proven).
                if let Val::Var(bv) = &basev {
                    // CSE for property access (Opportunity #4): cache key
                    // is the equation text `<binding>.<prop>`. Hit -> reuse
                    // the cached binding, skip the redundant emission.
                    // Invalidated by: declare (lua-name shadow), assign_to
                    // (binding reassignment), SetObjectProperty on the same
                    // property path, while/for body entry.
                    let cse_key = format!("{}.{}", bv, prop);
                    if self.optimize {
                        if let Some(cached) = self.cse_map.get(&cse_key) {
                            self.cse_hits += 1;
                            return Val::Var(cached.clone());
                        }
                    }
                    let eq = cse_key.clone();
                    self.emit(
                        "SolveEquation",
                        vec![("Equation".into(),
                              InputVal::Literal { value: eq, vtype: "String".into() })],
                        vec![("Result".into(), out.clone())],
                        chain,
                    );
                    if self.optimize {
                        self.cse_map.insert(cse_key, out.clone());
                    }
                    return Val::Var(out);
                }
                self.emit(
                    "GetObjectProperty",
                    vec![
                        ("Object".into(), self.val_input(&basev, "Object")),
                        (
                            "Property".into(),
                            InputVal::Literal { value: prop.clone(), vtype: "String".into() },
                        ),
                    ],
                    vec![("Value".into(), out.clone())],
                    chain,
                );
                return Val::Var(out);
            }
            key_expr => {
                let kv = self.expr(key_expr, chain, None);
                // RUNTIME BUG FIX: if the source table is a Lua table
                // (tracked in self.lua_tables), the result of GetTableValue
                // is ALSO a Lua table. Without this, subsequent assignments
                // like `squares[i][j] = sq` misroute to SetObjectProperty
                // (because t144 isn't in lua_tables) and RS rejects the
                // Lua table with "'Object' input must be a Instance".
                if let Val::Var(b) = &basev {
                    if self.lua_tables.contains(b) && !decomposed_leaf {
                        self.lua_tables.insert(out.clone());
                    }
                }
                // [patch12] the read binds the caller's name directly
                // (assignment rebinding: `local e = LASTS[li]` costs ONE
                // block - no SetVariable1, no temp). Table-typed consumers
                // reference the name as a mode-0 raw name, which the
                // importer wires to this output binding (raycaster-proven
                // shape: GTV -> t144 -> SetTableValue t144[...] worked
                // in-game). The patch8/10 registrations are gone - their
                // self-named "X to X" SetVariable1 shapes resolved to nil
                // in-game (charcode runs 2 and 3).
                self.emit(
                    "GetTableValue",
                    vec![
                        ("Table".into(), self.val_input(&basev, "Table")),
                        ("Key".into(), self.val_input(&kv, "Any")),
                    ],
                    vec![("Value".into(), out.clone())],
                    chain,
                );
                return Val::Var(out);
            }
        }
    }


    fn static_path(&self, e: &Expr) -> Option<String> {
        fn walk(e: &Expr, out: &mut String) -> bool {
            match e {
                Expr::Name(n) if matches!(n.as_str(), "game" | "script" | "workspace" | "Enum") => {
                    out.push_str(n);
                    true
                }
                Expr::Index(b, k) => {
                    if walk(b, out) {
                        if let Expr::Str(s) = &**k {
                            out.push('.');
                            out.push_str(s);
                            return true;
                        }
                    }
                    false
                }
                _ => false,
            }
        }
        let mut s = String::new();
        if walk(e, &mut s) {
            Some(s)
        } else {
            None
        }
    }

    fn method_expr(
        &mut self,
        base: &Expr,
        m: &str,
        args: &[Expr],
        chain: &mut Vec<String>,
        bind: Option<&str>,
    ) -> Val {
        let out = bind.map(|s| s.to_string()).unwrap_or_else(|| self.temp());
        match m {
            "Connect" => {
                let gate = bind.map(|b| format!("{}Gate", b));
                self.connect(base, args, chain, gate.clone());
                // initialize the gate to true when the connection is bound
                if gate.is_some() {
                    if let Some(name) = bind {
                        self.emit(
                            "SetVariable1",
                            vec![(
                                "Value".into(),
                                InputVal::Literal { value: "1".into(), vtype: "Bool".into() },
                            )],
                            vec![("VariableName".into(), format!("{}Gate", name))],
                            chain,
                        );
                    }
                }
                return Val::Lit(LitVal::Bool(true));
            }
            "Disconnect" => {
                // gate the handler off: SetVariable1 <binding>Gate = false
                if let Expr::Name(n) = base {
                    if let Some(b) = self.lookup(n) {
                        let gate = format!("{}Gate", b);
                        self.emit(
                            "SetVariable1",
                            vec![(
                                "Value".into(),
                                InputVal::Literal { value: "0".into(), vtype: "Bool".into() },
                            )],
                            vec![("VariableName".into(), gate)],
                            chain,
                        );
                        self.note(
                            "WORKAROUND",
                            format!(
                                "{}:Disconnect() lowered to clearing its gate variable (event \
                                 blocks expose no connection objects)",
                                n
                            ),
                        );
                        return Val::Lit(LitVal::Bool(true));
                    }
                }
                self.note(
                    "VERIFY",
                    "Disconnect on a connection handle that flowed through a return value - \
                     no gate available; handler keeps firing (safe for destroyed hitboxes)"
                        .into(),
                );
                return Val::Lit(LitVal::Bool(true));
            }
            _ => {}
        }

        let basev = self.expr(base, chain, None);
        match (m, args) {
            ("FindFirstChild", [name]) => {
                let nv = self.expr(name, chain, None);
                self.emit(
                    "FindFirstChildNew",
                    vec![
                        (
                            "CanReturnNil".into(),
                            InputVal::Literal { value: "1".into(), vtype: "Bool".into() },
                        ),
                        ("ChildName".into(), self.val_input(&nv, "String")),
                        (
                            "Recursive".into(),
                            InputVal::Literal { value: "0".into(), vtype: "Bool".into() },
                        ),
                        ("Parent".into(), self.val_input(&basev, "Object")),
                    ],
                    vec![("Result".into(), out.clone())],
                    chain,
                );
                return Val::Var(out);
            }
            ("WaitForChild", [name]) => {
                let nv = self.expr(name, chain, None);
                self.emit(
                    "WaitForChild",
                    vec![
                        ("Parent".into(), self.val_input(&basev, "Object")),
                        ("ChildName".into(), self.val_input(&nv, "String")),
                        (
                            "TimeOut".into(),
                            InputVal::Literal { value: "5".into(), vtype: "Number".into() },
                        ),
                    ],
                    vec![("Result".into(), out.clone())],
                    chain,
                );
                return Val::Var(out);
            }
            ("WaitForChild", [name, timeout]) => {
                let nv = self.expr(name, chain, None);
                let tv = self.expr(timeout, chain, None);
                self.emit(
                    "WaitForChild",
                    vec![
                        ("Parent".into(), self.val_input(&basev, "Object")),
                        ("ChildName".into(), self.val_input(&nv, "String")),
                        ("TimeOut".into(), self.val_input(&tv, "Number")),
                    ],
                    vec![("Result".into(), out.clone())],
                    chain,
                );
                return Val::Var(out);
            }
            ("PointToObjectSpace", [v]) | ("VectorToWorldSpace", [v])
            | ("PointToWorldSpace", [v]) | ("ToObjectSpace", [v])
            | ("ToWorldSpace", [v]) => {
                // CFrame methods — route through SolveEquation (user confirmed
                // SolveEquation can replace all CFrame operation blocks).
                let vv = self.expr(v, chain, None);
                let base_str = match &basev {
                    Val::Var(b) => b.clone(),
                    Val::Path(p) => p.clone(),
                    _ => "nil".into(),
                };
                let arg_str = match &vv {
                    Val::Var(b) => b.clone(),
                    Val::Path(p) => p.clone(),
                    _ => "nil".into(),
                };
                let eq = format!("{}:{}({})", base_str, m, arg_str);
                self.folded += 1;
                self.emit(
                    "SolveEquation",
                    vec![("Equation".into(), InputVal::Literal { value: eq, vtype: "String".into() })],
                    vec![("Result".into(), out.clone())],
                    chain,
                );
                return Val::Var(out);
            }
            ("GetChildren", []) => {
                // [patch12] direct binding - the ChildrenTable output IS
                // the variable (wire semantics). No registration block.
                self.emit(
                    "GetChildren",
                    vec![("Parent".into(), self.val_input(&basev, "Object"))],
                    vec![("ChildrenTable".into(), out.clone())],
                    chain,
                );
                self.lua_tables.insert(out.clone());
                return Val::Var(out);
            }
            ("GetPlayers", []) => {
                // [patch12] direct binding - the ChildrenTable output IS
                // the variable (wire semantics). No registration block.
                self.emit(
                    "GetChildren",
                    vec![("Parent".into(), self.val_input(&basev, "Object"))],
                    vec![("ChildrenTable".into(), out.clone())],
                    chain,
                );
                self.lua_tables.insert(out.clone());
                self.note(
                    "WORKAROUND",
                    ":GetPlayers() lowered to GetChildren(Parent)".into(),
                );
                return Val::Var(out);
            }
            ("Play", []) => {
                self.emit(
                    "PlaySound",
                    vec![("Sound".into(), self.val_input(&basev, "Object"))],
                    vec![],
                    chain,
                );
                return Val::Lit(LitVal::Bool(true));
            }
            ("Stop", []) => {
                self.emit(
                    "StopSound",
                    vec![("Sound".into(), self.val_input(&basev, "Object"))],
                    vec![],
                    chain,
                );
                return Val::Lit(LitVal::Bool(true));
            }
            ("IsA", [class]) => {
                let cv = self.expr(class, chain, None);
                self.emit(
                    "IsA",
                    vec![
                        ("Object".into(), self.val_input(&basev, "Object")),
                        ("Class".into(), self.val_input(&cv, "String")),
                    ],
                    vec![("Result".into(), out.clone())],
                    chain,
                );
                return Val::Var(out);
            }
            ("GetDescendants", []) => {
                // [patch12] direct binding - the DescendantTable output
                // IS the variable (wire semantics). No registration block.
                self.emit(
                    "GetDescendants",
                    vec![("Parent".into(), self.val_input(&basev, "Object"))],
                    vec![("DescendantTable".into(), out.clone())],
                    chain,
                );
                self.lua_tables.insert(out.clone());
                return Val::Var(out);
            }
            ("FindFirstChildOfClass", [class]) => {
                let cv = self.expr(class, chain, None);
                self.emit(
                    "FindFirstChildOfClassNew",
                    vec![
                        ("Parent".into(), self.val_input(&basev, "Object")),
                        ("ChildClass".into(), self.val_input(&cv, "String")),
                        ("CanReturnNil".into(), InputVal::Literal { value: "1".into(), vtype: "Bool".into() }),
                    ],
                    vec![("Result".into(), out.clone())],
                    chain,
                );
                return Val::Var(out);
            }
            ("Invoke", args) => {
                let param_inputs = self.tuple_param_inputs(args, chain);
                self.emit(
                    "BindableFunctionInvoke",
                    {
                        let mut v = vec![(
                            "BindableFunction".into(),
                            self.val_input(&basev, "Object"),
                        )];
                        v.extend(param_inputs);
                        v
                    },
                    vec![("TUPLE_Output".into(), out.clone())],
                    chain,
                );
                self.note(
                    "VERIFY",
                    "BindableFunctionInvoke single result bound directly to the variable \
                     (1-tuple treated as the value)"
                        .into(),
                );
                return Val::Var(out);
            }
            ("Fire", args) => {
                let mut inputs = vec![(
                    "BindableEvent".into(),
                    self.val_input(&basev, "Object"),
                )];
                inputs.extend(self.tuple_param_inputs(args, chain));
                self.emit("BindableFire2", inputs, vec![], chain);
                return Val::Lit(LitVal::Bool(true));
            }
            ("FireAllClients", args) => {
                let mut inputs = vec![(
                    "RemoteEvent".into(),
                    self.val_input(&basev, "Object"),
                )];
                inputs.extend(self.tuple_param_inputs(args, chain));
                self.emit("RemoteFireAllClients2", inputs, vec![], chain);
                return Val::Lit(LitVal::Bool(true));
            }
            ("FireClient", [player, rest @ ..]) => {
                let pv = self.expr(player, chain, None);
                let mut inputs = vec![
                    ("RemoteEvent".into(), self.val_input(&basev, "Object")),
                    ("Player".into(), self.val_input(&pv, "Object")),
                ];
                inputs.extend(self.tuple_param_inputs(rest, chain));
                self.emit("RemoteFireClient2", inputs, vec![], chain);
                return Val::Lit(LitVal::Bool(true));
            }
            ("FireServer", args) => {
                // RS tuple-INPUT convention (user hand-fix, B9)
                let mut inputs = vec![(
                    "RemoteEvent".into(),
                    self.val_input(&basev, "Object"),
                )];
                inputs.extend(self.tuple_param_inputs(args, chain));
                self.emit("RemoteFireServer2", inputs, vec![], chain);
                return Val::Lit(LitVal::Bool(true));
            }
            ("TakeDamage", [dmg]) => {
                let dv = self.expr(dmg, chain, None);
                self.emit(
                    "TakeDamage",
                    vec![
                        ("Player".into(), self.val_input(&basev, "Object")),
                        ("Damage".into(), self.val_input(&dv, "Number")),
                    ],
                    vec![],
                    chain,
                );
                return Val::Lit(LitVal::Bool(true));
            }
            ("Clone", []) => {
                self.emit(
                    "CloneObject",
                    vec![("Object".into(), self.val_input(&basev, "Object"))],
                    vec![("NewObject".into(), out.clone())],
                    chain,
                );
                return Val::Var(out);
            }
            ("Destroy", []) => {
                self.emit(
                    "DestroyObject",
                    vec![("Object".into(), self.val_input(&basev, "Object"))],
                    vec![],
                    chain,
                );
                return Val::Lit(LitVal::Bool(true));
            }
            ("Raycast", [origin_expr, direction_expr]) => {
                // RS doesn't have workspace:Raycast() — synthesize using
                // FindPartOnRay. No RaycastParams (2-arg form): use defaults.
                let origin_val = self.expr(origin_expr, chain, None);
                let direction_val = self.expr(direction_expr, chain, None);
                let normal_b = self.temp();
                let instance_b = self.temp();
                let position_b = self.temp();
                self.emit(
                    "FindPartOnRay",
                    vec![
                        ("Origin".into(), self.val_input(&origin_val, "Vector3")),
                        ("Direction".into(), self.val_input(&direction_val, "Vector3")),
                        ("FilterType".into(), InputVal::Implicit("Blacklist".into())),
                        ("FilterDescendantsTable".into(), InputVal::Implicit(String::new())),
                    ],
                    vec![
                        ("Normal".into(), normal_b.clone()),
                        ("Instance".into(), instance_b.clone()),
                        ("Position".into(), position_b.clone()),
                    ],
                    chain,
                );
                // Create a RaycastResult table {Instance, Position, Normal}
                // so raycastResult.Instance etc. work via SolveEquation.
                let tbl = self.unique_binding("raycastResult");
                self.lua_tables.insert(tbl.clone());
                self.emit("CreateTable", vec![], vec![("TableVariable".into(), tbl.clone())], chain);
                let instance_b_for_check = instance_b.clone();
                for (field, val_b) in [("Instance", instance_b), ("Position", position_b), ("Normal", normal_b)] {
                    self.emit(
                        "SetTableValue",
                        vec![
                            ("Table".into(), InputVal::Var(tbl.clone())),
                            ("Key".into(), InputVal::Literal { value: field.into(), vtype: "String".into() }),
                            ("Value".into(), InputVal::Var(val_b)),
                        ],
                        vec![],
                        chain,
                    );
                }
                // If Instance is nil (ray missed), set raycastResult to nil
                // so `if not raycastResult then break end` works correctly.
                let is_nil_b = self.temp();
                self.emit(
                    "Comparison",
                    vec![
                        ("Value 1".into(), InputVal::Var(instance_b_for_check.clone())),
                        ("Value 2".into(), InputVal::Literal { value: String::new(), vtype: "Nil".into() }),
                        ("ComparisonType".into(), InputVal::Literal { value: "==".into(), vtype: "String".into() }),
                    ],
                    vec![("Result".into(), is_nil_b.clone())],
                    chain,
                );
                let if_nil = self.emit("If", Self::if_inputs(&is_nil_b), vec![], chain);
                self.push_scope();
                let mut nil_chain = Vec::new();
                self.emit(
                    "SetVariable1",
                    vec![("Value".into(), InputVal::Literal { value: String::new(), vtype: "Nil".into() })],
                    vec![("VariableName".into(), tbl.clone())],
                    &mut nil_chain,
                );
                self.set_children(&if_nil, nil_chain);
                self.pop_scope();
                return Val::Var(tbl);
            }
            ("Raycast", [origin_expr, direction_expr, params_expr]) => {
                // 3-arg form with RaycastParams table.
                // Extract FilterType and FilterDescendantsInstances from the
                // params table via SolveEquation / GetTableValue.
                let origin_val = self.expr(origin_expr, chain, None);
                let direction_val = self.expr(direction_expr, chain, None);
                let params_val = self.expr(params_expr, chain, None);
                // RUNTIME BUG FIX: previous code embedded a Lua TABLE CONSTRUCTOR
                // inside a SolveEquation equation text:
                //   (({Exclude="Blacklist",Include="Whitelist"})[params.FilterType] or "Blacklist")
                // RS's SolveEquation parser cannot compile table constructors - it raised
                // "Equation not Compiled" at runtime. Replaced with the equivalent
                // GetTableValue + Comparison + If sequence:
                //   ft = params.FilterType   (GetTableValue)
                //   is_exclude = (ft == "Exclude")  (Comparison)
                //   if is_exclude: filter_type = "Blacklist"
                //   else:           filter_type = "Whitelist"
                let filter_type_b = self.temp();
                let ft_val_b = self.temp();
                self.emit(
                    "GetTableValue",
                    vec![
                        ("Table".into(), self.val_input(&params_val, "Table")),
                        ("Key".into(), InputVal::Literal { value: "FilterType".into(), vtype: "String".into() }),
                    ],
                    vec![("Value".into(), ft_val_b.clone())],
                    chain,
                );
                let is_exclude_b = self.temp();
                self.emit(
                    "Comparison",
                    vec![
                        ("Value 1".into(), InputVal::Var(ft_val_b.clone())),
                        ("Value 2".into(), InputVal::Literal { value: "Exclude".into(), vtype: "String".into() }),
                        ("ComparisonType".into(), InputVal::Literal { value: "==".into(), vtype: "String".into() }),
                    ],
                    vec![("Result".into(), is_exclude_b.clone())],
                    chain,
                );
                let if_name = self.emit(
                    "If",
                    Self::if_inputs(&is_exclude_b),
                    vec![],
                    chain,
                );
                self.push_scope();
                let mut then_chain = Vec::new();
                self.emit(
                    "SetVariable1",
                    vec![("Value".into(), InputVal::Literal { value: "Blacklist".into(), vtype: "String".into() })],
                    vec![("VariableName".into(), filter_type_b.clone())],
                    &mut then_chain,
                );
                self.set_children(&if_name, then_chain);
                self.pop_scope();
                let mut else_chain = Vec::new();
                self.emit(
                    "SetVariable1",
                    vec![("Value".into(), InputVal::Literal { value: "Whitelist".into(), vtype: "String".into() })],
                    vec![("VariableName".into(), filter_type_b.clone())],
                    &mut else_chain,
                );
                if let Some(i) = self.index.get(&if_name).copied() {
                    self.blocks[i].else_child = else_chain.first().cloned();
                }
                // RUNTIME BUG FIX (PATCH 8): do NOT push the else-branch's
                // first block to the outer chain. The existing if_stmt
                // convention (line ~1411) only sets If.else_child, doesn't
                // push to chain. Pushing made Set Variable43 a SIBLING of
                // If20 in Comparison43's children - it ran UNCONDITIONALLY,
                // clobbering t156 with Whitelist even when the then-branch
                // (Blacklist) had run. Worse, it broke the chain: when the
                // condition was true, Set Variable42 (then-branch) ran but
                // had no children, so GetTableValue8 + FindPartOnRay1
                // (which were children of Set Variable43) never ran.
                // Now the outer chain continues naturally with the next
                // statement (GetTableValue8 + FindPartOnRay1) as a sibling
                // of If20, running AFTER the If regardless of which
                // branch executed.
                // FilterDescendantsTable: GetTableValue(params, "FilterDescendantsInstances")
                let filter_table_b = self.temp();
                self.emit(
                    "GetTableValue",
                    vec![
                        ("Table".into(), self.val_input(&params_val, "Table")),
                        ("Key".into(), InputVal::Literal { value: "FilterDescendantsInstances".into(), vtype: "String".into() }),
                    ],
                    vec![("Value".into(), filter_table_b.clone())],
                    chain,
                );
                // Emit FindPartOnRay
                let normal_b = self.temp();
                let instance_b = self.temp();
                let position_b = self.temp();
                self.emit(
                    "FindPartOnRay",
                    vec![
                        ("Origin".into(), self.val_input(&origin_val, "Vector3")),
                        ("Direction".into(), self.val_input(&direction_val, "Vector3")),
                        ("FilterType".into(), InputVal::Var(filter_type_b)),
                        ("FilterDescendantsTable".into(), InputVal::Var(filter_table_b)),
                    ],
                    vec![
                        ("Normal".into(), normal_b.clone()),
                        ("Instance".into(), instance_b.clone()),
                        ("Position".into(), position_b.clone()),
                    ],
                    chain,
                );
                // Create RaycastResult table
                let tbl = self.unique_binding("raycastResult");
                self.lua_tables.insert(tbl.clone());
                self.emit("CreateTable", vec![], vec![("TableVariable".into(), tbl.clone())], chain);
                let instance_b_for_check = instance_b.clone();
                for (field, val_b) in [("Instance", instance_b), ("Position", position_b), ("Normal", normal_b)] {
                    self.emit(
                        "SetTableValue",
                        vec![
                            ("Table".into(), InputVal::Var(tbl.clone())),
                            ("Key".into(), InputVal::Literal { value: field.into(), vtype: "String".into() }),
                            ("Value".into(), InputVal::Var(val_b)),
                        ],
                        vec![],
                        chain,
                    );
                }
                // If Instance is nil (ray missed), set raycastResult to nil
                // so `if not raycastResult then break end` works correctly.
                let is_nil_b = self.temp();
                self.emit(
                    "Comparison",
                    vec![
                        ("Value 1".into(), InputVal::Var(instance_b_for_check.clone())),
                        ("Value 2".into(), InputVal::Literal { value: String::new(), vtype: "Nil".into() }),
                        ("ComparisonType".into(), InputVal::Literal { value: "==".into(), vtype: "String".into() }),
                    ],
                    vec![("Result".into(), is_nil_b.clone())],
                    chain,
                );
                let if_nil = self.emit("If", Self::if_inputs(&is_nil_b), vec![], chain);
                self.push_scope();
                let mut nil_chain = Vec::new();
                self.emit(
                    "SetVariable1",
                    vec![("Value".into(), InputVal::Literal { value: String::new(), vtype: "Nil".into() })],
                    vec![("VariableName".into(), tbl.clone())],
                    &mut nil_chain,
                );
                self.set_children(&if_nil, nil_chain);
                self.pop_scope();
                return Val::Var(tbl);
            }
            ("GetService", [svc]) => {
                let sv = self.expr(svc, chain, None);
                // RetroStudio service handling (runtime-verified fix).
                //
                // ALL game:GetService("X") calls route through
                // GetObjectProperty(game, "X") so the binding
                // actually holds the live service Instance at
                // runtime. Verified working in the event test:
                //   - game:GetService("RunService")     -> GetObjectProperty
                //   - game:GetService("UserInputService") -> GetObjectProperty
                //   - game:GetService("Players")         -> GetObjectProperty
                // Each yields a real Instance reference, downstream
                // SolveEquation('Players.LocalPlayer') resolves, and
                // events like CharacterAdded.Player=<localPlayer var name>
                // work because localPlayer is a real Player Instance.
                //
                // The previous code returned Val::Path(svc_name) for
                // services on the RS_SUPPORTED_SERVICES list, which
                // produced SetVariable1 with the literal string svc_name
                // (type=Object). That binding did NOT resolve to the
                // live service at runtime - Players ended up nil,
                // Players.LocalPlayer was nil, CharacterAdded.Player
                // got nil, runtime error:
                //   "'Player' input of 'CharacterAdded' must be a 'Player'
                //    got 'nil'"
                //
                // Note: real Roblox globals (`workspace`, `game`,
                // `script`) STILL route through Val::Path ->
                // SetVariable1-with-literal - they ARE globals in
                // Roblox, the literal name resolves. Service names are
                // NOT globals, they are properties of `game` and must
                // be fetched.
                const RS_UNSUPPORTED_SERVICES: &[&str] = &[
                    "ReplicatedStorage",
                    "ServerStorage",
                    "ServerScriptService",
                ];
                if let Val::Lit(LitVal::Str(svc_name)) = &sv {
                    if RS_UNSUPPORTED_SERVICES.contains(&svc_name.as_str()) {
                        self.flag_unsupported(
                            &format!("game:GetService({:?})", svc_name),
                            chain,
                        );
                        return Val::Lit(LitVal::Nil);
                    }
                    self.note(
                        "VERIFY",
                        format!(
                            "game:GetService({:?}) - verify the service is available in RetroStudio",
                            svc_name
                        ),
                    );
                }
                self.emit(
                    "GetObjectProperty",
                    vec![
                        ("Object".into(), InputVal::Implicit("game".into())),
                        ("Property".into(), self.val_input(&sv, "String")),
                    ],
                    vec![("Value".into(), out.clone())],
                    chain,
                );
                return Val::Var(out);
            }
            ("GetPlayerFromCharacter", [char]) => {
                let cv = self.expr(char, chain, None);
                self.emit(
                    "GetPlayerFromCharacter",
                    vec![("Character".into(), self.val_input(&cv, "Object"))],
                    vec![("Player".into(), out.clone())],
                    chain,
                );
                return Val::Var(out);
            }
            other => {
                // RNG synthesis - impl in rs_support; non-RNG unknown methods
                // flag unsupported (nil, never truthy garbage)
                let (mname, margs) = other;
                let state = match &basev {
                    Val::Var(s) => Some(s.clone()),
                    _ => None,
                };
                if let Some(state) = state {
                    if self.rs_try_synth_rng(mname, &state, margs, &out, chain) {
                        return Val::Var(out);
                    }
                }
                self.flag_unsupported(&format!(":{}", mname), chain);
                return Val::Lit(LitVal::Nil);
            }
        }
    }

pub(crate) fn eq_expr(&self, e: &Expr) -> String {
        match e {
            Expr::Nil => "nil".into(),
            Expr::Bool(b) => if *b { "true".into() } else { "false".into() },
            Expr::Number(n) => format!("{}", n),
            Expr::Str(s) => format!("{:?}", s),
            Expr::Vararg => "...".into(),
            // RUNTIME BUG FIX: previously `Paren(inner)` emitted `(<inner>)`, but
            // BinOp already wraps each operand in `()`, so this produced redundant
            // double parens like `((currentPhase + 1))` (3 leading parens for source
            // that had only 2). Dropping the outer wrap is safe because:
            // - For BinOp/UnOp inner: their own `()` provides correct grouping.
            // - For atoms (Name/Number/Str): no parens needed in equation text.
            // - For Call/Index: they have their own delimiters; precedence inside a
            //   larger BinOp is preserved by the outer BinOp's parens.
            Expr::Paren(inner) => self.eq_expr(inner),
            Expr::Name(n) => match self.lookup(n) {
                Some(b) => b,
                None => sanitize(n),
            },
            Expr::Index(b, k) => {
                let bs = self.eq_expr(b);
                match &**k {
                    Expr::Str(s) => {
                        // Enum.X.Y -> "Y": RS SolveEquation can't resolve
                        // enum constants, but InputBegan sets KeyCode
                        // to the string name (user-confirmed fix).
                        if bs.starts_with("Enum.") {
                            return format!("{:?}", s);
                        }
                        format!("{}.{}", bs, s)
                    }
                    other => format!("{}[{}]", bs, self.eq_expr(other)),
                }
            }
            Expr::Call(f, args) => {
                let fs = self.eq_expr(f);
                // tonumber(x) -> x: RS doesn't support tonumber in
                // SolveEquation ("invalid function"). Remove wrapper.
                if fs == "tonumber" && args.len() == 1 {
                    return self.eq_expr(&args[0]);
                }
                let as_: Vec<String> = args.iter().map(|a| self.eq_expr(a)).collect();
                format!("{}({})", fs, as_.join(", "))
            }
            Expr::Method(base, m, args) => {
                let bs = self.eq_expr(base);
                let as_: Vec<String> = args.iter().map(|a| self.eq_expr(a)).collect();
                format!("{}:{}({})", bs, m, as_.join(", "))
            }
            Expr::UnOp(op, inner) => {
                let s = self.eq_expr(inner);
                match op {
                    UnOp::Neg => format!("(-{})", s),
                    UnOp::Not => format!("not ({})", s),
                    UnOp::Len => format!("#({})", s),
                }
            }
            Expr::BinOp(op, a, b) => {
                let x = self.eq_expr(a);
                let y = self.eq_expr(b);
                let ops = match op {
                    BinOp::Add => "+",
                    BinOp::Sub => "-",
                    BinOp::Mul => "*",
                    BinOp::Div => "/",
                    BinOp::Mod => "%",
                    BinOp::Pow => "^",
                    BinOp::Concat => "..",
                    BinOp::Eq => "==",
                    BinOp::Ne => "~=",
                    BinOp::Lt => "<",
                    BinOp::Le => "<=",
                    BinOp::Gt => ">",
                    BinOp::Ge => ">=",
                    BinOp::And => "and",
                    BinOp::Or => "or",
                };
                format!("({} {} {})", x, ops, y)
            }
            Expr::Table(_) => "{}".into(),
            Expr::Function(..) => "<function>".into(),
        }
    }
    /// Emit an event block. When we're INSIDE a function body
    /// (scopes.len() > 1), use `emit` (adds to the current chain) so the
    /// event block registers when the function runs and reaches that
    /// point - AFTER any materialize blocks have set the player/object
    /// value. When at top-level, use `emit_root` (current behavior) so
    /// the event block is a top-level root registering at script-load.
    fn emit_event_block(
        &mut self,
        btype: &str,
        inputs: Vec<(String, InputVal)>,
        outputs: Vec<(String, String)>,
        chain: &mut Vec<String>,
    ) -> String {
        if self.scopes.len() > 1 {
            // Inside a function body: emit to chain so the event block
            // is a child of the function body chain. With BODY
            // classification in chain_walk, it becomes a child of the
            // previous tail (the materialize block), and the next chain
            // element continues through the container - so the event
            // block's handler body (its own children) stays separate.
            self.emit(btype, inputs, outputs, chain)
        } else {
            // Top-level: emit as a root. Promoted to top-level roots
            // via the orphan-promotion step in transpile().
            let mut roots: Vec<String> = Vec::new();
            self.emit_root(btype, inputs, outputs, &mut roots)
        }
    }

    /// [patch10] Lua-table-ness oracle: a binding PROVEN to hold a Lua table
    /// (CreateTable / TablePack / tracked GetTableValue result / registered
    /// table-returning call result / materialized table parameter). Single
    /// source of truth for patch8 read registration and SetTableValue write
    /// dispatch.
    fn val_is_table(&self, v: &Val) -> bool {
        match v {
            Val::Var(b) => self.lua_tables.contains(b),
            _ => false,
        }
    }

    /// [patch10] Pre-scan a function body for parameters USED AS LUA TABLES:
    /// bracket/dot indexing (read or write position), `#p`, method calls on
    /// the param, table.insert/table.remove with the param as first argument,
    /// and pairs/ipairs/next/unpack over it. Only those params get
    /// materialized into a registered variable + lua_tables, keeping block
    /// count tight - Instance params (player, part, ...) keep the raw wire.
    fn params_used_as_tables(
        &self,
        params: &[String],
        body: &[Stmt],
    ) -> std::collections::HashSet<String> {
        let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
        fn is_param(n: &str, params: &[String]) -> bool {
            params.iter().any(|p| p == n)
        }
        fn mark_name(n: &str, params: &[String], used: &mut std::collections::HashSet<String>) {
            if is_param(n, params) {
                used.insert(n.to_string());
            }
        }
        fn walk(e: &Expr, params: &[String], used: &mut std::collections::HashSet<String>) {
            match e {
                Expr::Index(b, k) => {
                    if let Expr::Name(n) = &**b {
                        mark_name(n, params, used);
                    }
                    walk(b, params, used);
                    walk(k, params, used);
                }
                Expr::Method(b, _m, args) => {
                    if let Expr::Name(n) = &**b {
                        mark_name(n, params, used);
                    }
                    walk(b, params, used);
                    for a in args {
                        walk(a, params, used);
                    }
                }
                Expr::Call(f, args) => {
                    if let Expr::Name(n) = &**f {
                        if matches!(
                            n.as_str(),
                            "table.insert" | "table.remove" | "pairs" | "ipairs" | "next" | "unpack"
                        ) {
                            if let Some(Expr::Name(a0)) = args.first() {
                                mark_name(a0, params, used);
                            }
                        }
                    }
                    if let Expr::Index(b, _k) = &**f {
                        if let Expr::Name(n) = &**b {
                            mark_name(n, params, used);
                        }
                    }
                    walk(f, params, used);
                    for a in args {
                        walk(a, params, used);
                    }
                }
                Expr::UnOp(op, inner) => {
                    if matches!(op, UnOp::Len) {
                        if let Expr::Name(n) = &**inner {
                            mark_name(n, params, used);
                        }
                    }
                    walk(inner, params, used);
                }
                Expr::BinOp(_op, l, r) => {
                    walk(l, params, used);
                    walk(r, params, used);
                }
                Expr::Paren(i) => walk(i, params, used),
                Expr::Function(ps, b) => {
                    // a name shadowed by an inner function's own param list is
                    // the inner function's parameter, not ours
                    let inner: Vec<String> = params
                        .iter()
                        .filter(|p| !ps.contains(p))
                        .cloned()
                        .collect();
                    walk_stmts(b, &inner, used);
                }
                Expr::Table(fields) => {
                    for f in fields {
                        match f {
                            TableField::Pos(v) => walk(v, params, used),
                            TableField::Named(_k, v) => walk(v, params, used),
                            TableField::Bracketed(_k, v) => walk(v, params, used),
                        }
                    }
                }
                _ => {}
            }
        }
        fn walk_stmts(
            stmts: &[Stmt],
            params: &[String],
            used: &mut std::collections::HashSet<String>,
        ) {
            for s in stmts {
                match s {
                    Stmt::Local(_names, exprs) => {
                        for e in exprs {
                            walk(e, params, used);
                        }
                    }
                    Stmt::Return(exprs) => {
                        for e in exprs {
                            walk(e, params, used);
                        }
                    }
                    Stmt::LocalFunction(_n, ps, b) => {
                        let inner: Vec<String> = params
                            .iter()
                            .filter(|p| !ps.contains(p))
                            .cloned()
                            .collect();
                        walk_stmts(b, &inner, used);
                    }
                    Stmt::Assign(t, v) => {
                        walk(t, params, used);
                        walk(v, params, used);
                    }
                    Stmt::MultiAssign(ts, vs) => {
                        for t in ts {
                            walk(t, params, used);
                        }
                        for v in vs {
                            walk(v, params, used);
                        }
                    }
                    Stmt::If(arms, els) => {
                        for (c, b) in arms {
                            walk(c, params, used);
                            walk_stmts(b, params, used);
                        }
                        if let Some(b) = els {
                            walk_stmts(b, params, used);
                        }
                    }
                    Stmt::While(c, b) => {
                        walk(c, params, used);
                        walk_stmts(b, params, used);
                    }
                    Stmt::NumericFor(_v, from, to, step, b) => {
                        walk(from, params, used);
                        walk(to, params, used);
                        if let Some(st) = step {
                            walk(st, params, used);
                        }
                        walk_stmts(b, params, used);
                    }
                    Stmt::GenericFor(_names, exprs, b) => {
                        for e in exprs {
                            walk(e, params, used);
                        }
                        walk_stmts(b, params, used);
                    }
                    Stmt::ExprStmt(e) => walk(e, params, used),
                    Stmt::Break | Stmt::Continue => {}
                }
            }
        }
        walk_stmts(body, params, &mut used);
        used
    }

    /// When `:Connect(...)` is called inside a function body, the base
    /// object expression (e.g. `player` in `player.CharacterAdded:Connect`)
    /// is often a function parameter. Function parameters are bound via
    /// DefineFunction's TUPLEPARAM_Parameters_i outputs, which are
    /// function-local - they're cleared after the function returns.
    ///
    /// Event blocks (CharacterAdded, Chatted, PartTouched, ...) fire LATER,
    /// when the actual event happens. By that time the function has returned
    /// and the parameter binding is gone. RS's `Player 0 <name>` lookup at
    /// event fire time returns nil -> "'Player' input of 'CharacterAdded'
    /// must be a 'Player', got 'nil'".
    ///
    /// Fix: capture the obj value into a FRESH persistent variable via
    /// SetVariable1 at registration time. The fresh binding persists for
    /// the script's lifetime, so the event block can resolve it later.
    ///
    /// For Val::Lit (literals), no materialization needed - they're
    /// constants. For other Val types, use materialize_val.
    fn materialize_for_event(&mut self, ov: &Val, chain: &mut Vec<String>) -> Val {
        match ov {
            Val::Var(b) => {
                // Var binding (e.g., function parameter 'player'): materialize
                // to a fresh persistent binding. The event fires LATER, after
                // the current function has returned, so we need a persistent
                // value that survives. The fresh SetVariable1 captures the
                // value at registration time.
                let fresh = self.temp();
                self.emit(
                    "SetVariable1",
                    vec![(
                        "Value".into(),
                        InputVal::Var(b.clone()),
                    )],
                    vec![("VariableName".into(), fresh.clone())],
                    chain,
                );
                Val::Var(fresh)
            }
            // RUNTIME BUG FIX: Val::Lit and Val::Path are constants - pass
            // them through directly (no materialization). Previously the
            // `_ =>` branch called materialize_val which doesn't handle
            // Val::Path (does nothing), so the fresh binding was created
            // but never set. This caused DescendantAdded1 to reference
            // t276 (an unset binding) as its Object input -> "got nil".
            // Now Val::Path("workspace") passes through, and the event
            // block uses Object=workspace directly (RS resolves the
            // workspace Instance via its named object registry).
            Val::Lit(_) | Val::Path(_) => ov.clone(),
            _ => {
                let fresh = self.temp();
                self.materialize_val(ov, &fresh, chain);
                Val::Var(fresh)
            }
        }
    }

    /// Renders a value into `out` as a REAL SetVariable1 block - the old
    /// and/or lowering returned Val::Lit and never wrote the variable
    /// (user-caught B5: toY came out empty).
    fn materialize_val(&mut self, v: &Val, out: &str, chain: &mut Vec<String>) {
        match v {
            Val::Lit(l) => {
                let input = self.lit_input(l);
                self.emit(
                    "SetVariable1",
                    vec![("Value".into(), input)],
                    vec![("VariableName".into(), out.to_string())],
                    chain,
                );
            }
            Val::Var(b) if b == out => {
                // SELF-ASSIGNMENT NO-OP ELIMINATION (Opportunity #3):
                // Skip emission when the value's binding equals the
                // output binding. The producer block already bound `out`
                // to the correct value. SetVariable1(Var(b), b) is a
                // pure no-op (rebinds a variable to its own current
                // value) — no observable side effect.
            }
            Val::Var(b) => {
                self.emit(
                    "SetVariable1",
                    vec![("Value".into(), InputVal::Var(b.clone()))],
                    vec![("VariableName".into(), out.to_string())],
                    chain,
                );
            }
            _ => {}
        }
    }

    /// Luau truthiness on a literal: only nil and false are falsy.
    fn lit_truthy(l: &LitVal) -> bool {
        match l {
            LitVal::Nil => false,
            LitVal::Bool(b) => *b,
            _ => true,
        }
    }

    /// Const-folds an and/or chain of literals with Luau semantics.
    fn const_andor(&self, e: &Expr) -> Option<LitVal> {
        match e {
            Expr::BinOp(BinOp::And, a, b) => {
                let av = self.const_andor(a)?;
                let bv = self.const_andor(b)?;
                Some(if Self::lit_truthy(&av) { bv } else { av })
            }
            Expr::BinOp(BinOp::Or, a, b) => {
                let av = self.const_andor(a)?;
                let bv = self.const_andor(b)?;
                Some(if Self::lit_truthy(&av) { av } else { bv })
            }
            other => self.const_lit(other),
        }
    }

        /// Bool gate for truthiness of a BINDING. Implements exact Luau
/// truthiness semantics:
///   truthy(x) <=> x ~= nil AND x ~= false
/// (`false` is excluded — this is the reason a bare `~= nil` check is
/// wrong; `0` and `""` are correctly truthy). Replaced the previous
/// 3-block lowering (2 Comparison + 1 ANDAny, plus 1 NOTAny for the
/// want=false case) with a single SolveEquation. This is the same
/// primitive the compound-condition fold already relies on (line
/// ~1148) and is semantics-preserving under the SAME evidence:
///
///   - SolveEquation evaluates Luau infix over bound variables
///     (TRANSPILE.md §"VERIFY list" #6).
///   - The compound fold path was "Verified safe at runtime: the
///     user confirmed SolveEquation evaluates `not processed and
///     input.KeyCode == Enum.KeyCode.Q` correctly in the event test".
///
/// The equation text uses the EXPLICIT `(<b> ~= nil and <b> ~= false)`
/// form rather than `not not <b>` because:
///   - `~=` returns Bool for any operand pair (Luau spec).
///   - `and` of two Bools returns Bool (left if falsy, else right;
///     both-Bool inputs produce a Bool result).
///   - `not (...)` of a Bool returns Bool.
/// So the entire equation is Bool-valued over Bool inputs, avoiding
/// any unverified behaviour of `not not` on Instance/Table/String
/// inputs (the user's warning: "Do not assume `not not x` is valid
/// for every target block input type without evidence.").
///
/// Cost: 1 SolveEquation block. Previous cost: 3 blocks for want=true
/// (2 Comparison + 1 ANDAny) or 4 blocks for want=false (extra NOTAny).
        fn truthy_gate_var(&mut self, b: &str, want: bool, chain: &mut Vec<String>) -> String {
            let g = self.temp();
            let eq = if want {
                format!("({} ~= nil and {} ~= false)", b, b)
            } else {
                format!("not ({} ~= nil and {} ~= false)", b, b)
            };
            self.emit(
                "SolveEquation",
                vec![(
                    "Equation".into(),
                    InputVal::Literal { value: eq, vtype: "String".into() },
                )],
                vec![("Result".into(), g.clone())],
                chain,
            );
            g
        }

    /// Bool gate for truthiness of an EXPR. Comparisons use the proven
    /// fused lowering; and/or sub-chains recurse; bare vars use the
    /// typeof pair. Returns (gate_for_if, value_binding_of_operand).
    fn truthy_gate(
        &mut self,
        e: &Expr,
        want: bool,
        chain: &mut Vec<String>,
    ) -> (String, Option<String>) {
        match e {
            Expr::BinOp(
                BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge,
                _,
                _,
            )
            | Expr::UnOp(UnOp::Not, _) => {
                let t = self.temp();
                let g = self.condition_to(e, t, chain);
                if want {
                    (g.clone(), Some(g))
                } else {
                    let ng = self.temp();
                    self.emit(
                        "NOTAny",
                        vec![("A".into(), InputVal::Var(g.clone()))],
                        vec![("Result".into(), ng.clone())],
                        chain,
                    );
                    self.note(
                        "VERIFY",
                        "NOTAny inverted gate in and/or value position - block unproven in game".into(),
                    );
                    (ng, Some(g))
                }
            }
            Expr::BinOp(BinOp::And | BinOp::Or, ..) => {
                let t = self.temp();
                self.andor_value_expr(e, &t, chain);
                let g = self.truthy_gate_var(&t, want, chain);
                (g, Some(t))
            }
            _ => {
                let v = self.expr(e, chain, None);
                match &v {
                    Val::Var(b) => {
                        let g = self.truthy_gate_var(b, want, chain);
                        (g, Some(b.clone()))
                    }
                    Val::Lit(l) => {
                        let hit = Self::lit_truthy(l);
                        let g = self.temp();
                        self.emit(
                            "SetVariable1",
                            vec![("Value".into(), InputVal::Literal {
                                value: if hit == want { "1".into() } else { "0".into() },
                                vtype: "Bool".into() })],
                            vec![("VariableName".into(), g.clone())],
                            chain,
                        );
                        (g, None)
                    }
                    _ => {
                        let g = self.temp();
                        self.emit(
                            "SetVariable1",
                            vec![("Value".into(), InputVal::Literal {
                                value: if want { "1".into() } else { "0".into() },
                                vtype: "Bool".into() })],
                            vec![("VariableName".into(), g.clone())],
                            chain,
                        );
                        (g, None)
                    }
                }
            }
        }
    }

    /// Value-position and/or (user ground truth B5):
    /// - all-literal chains const-fold
    /// - ternary idiom `A and B or C`: default C, If(truthy A) -> B
    /// - general chains: left-assoc recursion, real writes only
    fn andor_value_expr(&mut self, e: &Expr, out: &str, chain: &mut Vec<String>) {
        if let Some(l) = self.const_andor(e) {
            self.materialize_val(&Val::Lit(l), out, chain);
            return;
        }
        if let Expr::BinOp(BinOp::Or, ab, c) = e {
            if let Expr::BinOp(BinOp::And, a, b) = &**ab {
                let (gate, _) = self.truthy_gate(a, true, chain);
                self.andor_value_expr(c, out, chain);
                let ifn = self.emit("If", Self::if_inputs(&gate), vec![], chain);
                let mut body = Vec::new();
                self.andor_value_expr(b, out, &mut body);
                self.set_children(&ifn, body);
                return;
            }
        }
        match e {
            Expr::BinOp(op @ (BinOp::And | BinOp::Or), a, b) => {
                let want = matches!(op, BinOp::And);
                let (gate, aval) = self.truthy_gate(a, want, chain);
                if let Some(ab) = aval {
                    self.materialize_val(&Val::Var(ab), out, chain);
                }
                let ifn = self.emit("If", Self::if_inputs(&gate), vec![], chain);
                let mut body = Vec::new();
                self.andor_value_expr(b, out, &mut body);
                self.set_children(&ifn, body);
            }
            other => {
                let v = self.expr(other, chain, Some(out));
                self.materialize_val(&v, out, chain);
            }
        }
    }

    /// RS tuple-INPUT convention (user hand-fix, B9): the Parameters input
    /// is mode 0 with a comma-joined TUPLEPARAM_Parameters_i name list
    /// (reference before declaration), followed by one TUPLEPARAM_Parameters_i
    /// input per value in its own wire form (mode 2 var / mode 1 literal
    /// with concrete token). Zero args: mode 0 empty (ExecuteFunction
    /// zero-arg precedent). NOTE: Number-valued elements emit hexs()+Number
    /// token - unproven on the wire (B9 covered String/var only).
    fn tuple_param_inputs(
        &mut self,
        exprs: &[Expr],
        chain: &mut Vec<String>,
    ) -> Vec<(String, InputVal)> {
        if exprs.is_empty() {
            return vec![("Parameters".into(), InputVal::Implicit(String::new()))];
        }
        let names: Vec<String> = (1..=exprs.len())
            .map(|i| format!("TUPLEPARAM_Parameters_{}", i))
            .collect();
        let mut inputs = vec![(
            "Parameters".into(),
            InputVal::Implicit(names.join(",")),
        )];
        for (i, e) in exprs.iter().enumerate() {
            let v = self.expr(e, chain, None);
            let iv = self.val_input(&v, "Any");
            inputs.push((names[i].clone(), iv));
        }
        inputs
    }

    fn output_binding(&self, block: &str, out: &str) -> Option<String> {
        let i = *self.index.get(block)?;
        self.blocks[i]
            .outputs
            .iter()
            .find(|(n, _)| n == out)
            .map(|(_, b)| b.clone())
    }

    fn connect(
        &mut self,
        base: &Expr,
        args: &[Expr],
        chain: &mut Vec<String>,
        gate: Option<String>,
    ) {
        let (params, body) = match args {
            [Expr::Function(p, b)] => (p.clone(), b.clone()),
            // RUNTIME BUG FIX: when the argument is a Name reference to a
            // previously-defined DefineFunction (e.g. setupPlayer), look up
            // the function's parameter list and use it as the event's params.
            // Construct the body as `setupPlayer(p1, p2, ...)` passing each
            // param name as an argument - inside the event handler scope,
            // those names will be re-declared to point at the event's bound
            // outputs (Player, Message, etc.), so the call receives the
            // actual event-supplied values.
            [target_fn @ Expr::Name(n)] => {
                let fn_params = self
                    .function_params
                    .get(n)
                    .cloned()
                    .unwrap_or_default();
                let arg_exprs: Vec<Expr> = fn_params
                    .iter()
                    .map(|p| Expr::Name(p.clone()))
                    .collect();
                (
                    fn_params,
                    vec![Stmt::ExprStmt(Expr::Call(
                        Box::new(target_fn.clone()),
                        arg_exprs,
                    ))],
                )
            }
            [target_fn] => (
                vec![],
                vec![Stmt::ExprStmt(Expr::Call(Box::new(target_fn.clone()), vec![]))],
            ),
            _ => {
                self.note("GAP", "Connect with invalid arguments".into());
                return;
            }
        };

        enum Ev {
            PartTouched(Box<Expr>),
            PartTouchEnded(Box<Expr>),
            RemoteServer(Box<Expr>),
            RemoteClient(Box<Expr>),
            Bindable(Box<Expr>),
            PlayerAdded,
            PlayerRemoving,
            Property(Box<Expr>, String),
            GuiClick(Box<Expr>),
            TextBoxFocusLost(Box<Expr>),
            AnyProperty(Box<Expr>),
            RenderStepped,
            InputBegan,
            CharacterAdded(Box<Expr>),
            Chatted(Box<Expr>),
            DescendantAdded(Box<Expr>),
            Other,
        }
        let ev = match base {
            Expr::Index(obj, key) => match &**key {
                Expr::Str(k) => match k.as_str() {
                    "Touched" => Ev::PartTouched(obj.clone()),
                    "TouchEnded" => Ev::PartTouchEnded(obj.clone()),
                    "OnServerEvent" => Ev::RemoteServer(obj.clone()),
                    "OnClientEvent" => Ev::RemoteClient(obj.clone()),
                    "Event" => Ev::Bindable(obj.clone()),
                    "PlayerAdded" if matches!(&**obj, Expr::Name(n) if n == "Players") => {
                        Ev::PlayerAdded
                    }
                    "PlayerRemoving" if matches!(&**obj, Expr::Name(n) if n == "Players") => {
                        Ev::PlayerRemoving
                    }
                    "MouseButton1Click" => Ev::GuiClick(obj.clone()),
                    "FocusLost" => Ev::TextBoxFocusLost(obj.clone()),
                    "Changed" => Ev::AnyProperty(obj.clone()),
                    "RenderStepped" => Ev::RenderStepped,
                    "InputBegan" => Ev::InputBegan,
                    "CharacterAdded" => Ev::CharacterAdded(obj.clone()),
                    "Chatted" => Ev::Chatted(obj.clone()),
                    "DescendantAdded" => Ev::DescendantAdded(obj.clone()),
                    _ => Ev::Other,
                },
                _ => Ev::Other,
            },
            Expr::Method(obj, m, margs) if m == "GetPropertyChangedSignal" => match margs.as_slice()
            {
                [Expr::Str(p)] => Ev::Property(obj.clone(), p.clone()),
                _ => Ev::Other,
            },
            _ => Ev::Other,
        };

        // emit the event root; its pre-inputs (object refs) go into `chain`
        // but the root itself is standalone.
        let name = match &ev {
            Ev::PartTouched(obj) | Ev::PartTouchEnded(obj) => {
                let ov_raw = self.expr(obj, chain, None);
                let ov = self.materialize_for_event(&ov_raw, chain);
                let t = if matches!(ev, Ev::PartTouched(_)) { "PartTouched" } else { "PartTouchEnded" };
                let mut outputs = Vec::new();
                if let Some(p0) = params.first() {
                    if p0 != "_" {
                        outputs.push(("otherPart".to_string(), self.unique_binding(p0)));
                    }
                }
                let mut roots: Vec<String> = Vec::new();
                let n = self.emit_event_block(t, vec![("Part".into(), self.val_input(&ov, "Object"))], outputs, chain);
                n
            }
            Ev::RemoteServer(obj) | Ev::RemoteClient(obj) => {
                let ov_raw = self.expr(obj, chain, None);
                let ov = self.materialize_for_event(&ov_raw, chain);
                let server = matches!(ev, Ev::RemoteServer(_));
                let t = if server { "RemoteRecieveServer2" } else { "RemoteRecieveClient2" };
                // RS exposes handler parameters as TUPLEPARAM_Parameters_i
                // outputs on the event block (user ground truth, T5); no
                // TablePack/GetTableValue needed.
                let skipn = if server { 1 } else { 0 };
                let tparams: Vec<&String> = params.iter().skip(skipn).collect();
                let mut outputs = vec![(
                    "TUPLE_Parameters".to_string(),
                    tparams.len().to_string(),
                )];
                if server {
                    if let Some(p0) = params.first() {
                        if p0 != "_" {
                            outputs.push(("Player".to_string(), self.unique_binding(p0)));
                        }
                    }
                    if tparams.len() > 0 {
                        self.note("VERIFY", "server-remote TUPLEPARAM indexing past the Player slot - confirm in game".into());
                    }
                }
                for (i, prm) in tparams.iter().enumerate() {
                    if prm.as_str() != "_" {
                        outputs.push((
                            format!("TUPLEPARAM_Parameters_{}", i + 1),
                            self.unique_binding(prm),
                        ));
                    }
                }
                let mut roots: Vec<String> = Vec::new();
                let n = self.emit_root(
                    t,
                    vec![("RemoteEvent".into(), self.val_input(&ov, "Object"))],
                    outputs,
                    chain,
                );
                n
            }
            Ev::Bindable(obj) => {
                let ov = self.expr(obj, chain, None);
                let mut roots: Vec<String> = Vec::new();
                let tparams: Vec<&String> = params.iter().collect();
                let mut outputs = vec![(
                    "TUPLE_Parameters".to_string(),
                    tparams.len().to_string(),
                )];
                for (i, prm) in tparams.iter().enumerate() {
                    if prm.as_str() != "_" {
                        outputs.push((
                            format!("TUPLEPARAM_Parameters_{}", i + 1),
                            self.unique_binding(prm),
                        ));
                    }
                }
                let n = self.emit_root(
                    "BindableRecieve2",
                    vec![("BindableEvent".into(), self.val_input(&ov, "Object"))],
                    outputs,
                    &mut roots,
                );
                n
            }
            Ev::PlayerAdded | Ev::PlayerRemoving => {
                let t = if matches!(ev, Ev::PlayerAdded) { "PlayerAdded" } else { "PlayerRemoving" };
                let mut outputs = Vec::new();
                if let Some(p0) = params.first() {
                    if p0 != "_" {
                        outputs.push(("Player".to_string(), self.unique_binding(p0)));
                    }
                }
                let mut roots: Vec<String> = Vec::new();
                let n = self.emit_root(t, vec![], outputs, &mut roots);
                n
            }
            Ev::Property(obj, prop) => {
                let ov_raw = self.expr(obj, chain, None);
                let ov = self.materialize_for_event(&ov_raw, chain);
                let mut outputs = Vec::new();
                if let Some(p0) = params.first() {
                    if p0 != "_" {
                        outputs.push(("NewValue".to_string(), self.unique_binding(p0)));
                    }
                }
                let mut roots: Vec<String> = Vec::new();
                let n = self.emit_event_block(
                    "PropertyChanged",
                    vec![
                        ("Object".into(), self.val_input(&ov, "Object")),
                        (
                            "Property".into(),
                            InputVal::Literal { value: property_alias(&prop).to_string(), vtype: "String".into() },
                        ),
                    ],
                    outputs,
                    chain,
                );
                n
            }
            Ev::GuiClick(obj) => {
                let ov_raw = self.expr(obj, chain, None);
                let ov = self.materialize_for_event(&ov_raw, chain);
                let mut roots: Vec<String> = Vec::new();
                let n = self.emit_event_block(
                    "GUILeftMouseButtonClick",
                    vec![("GUIButton".into(), self.val_input(&ov, "Object"))],
                    vec![],
                    chain,
                );
                n
            }
            Ev::TextBoxFocusLost(obj) => {
                let ov_raw = self.expr(obj, chain, None);
                let ov = self.materialize_for_event(&ov_raw, chain);
                let mut outputs = Vec::new();
                if let Some(p0) = params.first() {
                    if p0 != "_" {
                        outputs.push(("EnterPressed".to_string(), self.unique_binding(p0)));
                    }
                }
                let mut roots: Vec<String> = Vec::new();
                let n = self.emit_event_block(
                    "TextBoxFocusLost",
                    vec![("TextBox".into(), self.val_input(&ov, "Object"))],
                    outputs,
                    chain,
                );
                n
            }
            Ev::AnyProperty(obj) => {
                let ov_raw = self.expr(obj, chain, None);
                let ov = self.materialize_for_event(&ov_raw, chain);
                let mut outputs = Vec::new();
                if let Some(p0) = params.first() {
                    if p0 != "_" {
                        let out_name = if p0.to_lowercase().contains("prop") {
                            "Property"
                        } else {
                            "NewValue"
                        };
                        outputs.push((out_name.to_string(), self.unique_binding(p0)));
                    }
                }
                if let Some(p1) = params.get(1) {
                    if p1 != "_" {
                        let out_name = if p1.to_lowercase().contains("prop") {
                            "Property"
                        } else {
                            "NewValue"
                        };
                        outputs.push((out_name.to_string(), self.unique_binding(p1)));
                    }
                }
                let mut roots: Vec<String> = Vec::new();
                let n = self.emit_event_block(
                    "AnyPropertyChanged",
                    vec![("Object".into(), self.val_input(&ov, "Object"))],
                    outputs,
                    chain,
                );
                n
            }
            Ev::RenderStepped => {
                let mut outputs = Vec::new();
                if let Some(p0) = params.first() {
                    if p0 != "_" {
                        outputs.push(("deltaTime".to_string(), self.unique_binding(p0)));
                    }
                }
                let mut roots: Vec<String> = Vec::new();
                let n = self.emit_event_block("RenderStepped", vec![], outputs, chain);
                n
            }
            Ev::InputBegan => {
                // InputBegan outputs: Type, KeyCode, Position, Processed, EventConnection
                // Create an InputObject table for the first param so that
                // input.KeyCode etc. work via SolveEquation table field access.
                let type_b = self.unique_binding("inputType");
                let keycode_b = self.unique_binding("inputKeyCode");
                let pos_b = self.unique_binding("inputPos");
                let proc_b = self.unique_binding("inputProcessed");
                let outputs = vec![
                    ("Type".to_string(), type_b.clone()),
                    ("KeyCode".to_string(), keycode_b.clone()),
                    ("Position".to_string(), pos_b.clone()),
                    ("Processed".to_string(), proc_b.clone()),
                ];
                let mut roots: Vec<String> = Vec::new();
                let n = self.emit_event_block("InputBegan", vec![], outputs, chain);
                // Declare 2nd param as Processed output
                if let Some(p1) = params.get(1) {
                    if p1 != "_" {
                        self.pending_event_params.push((p1.clone(), proc_b));
                    }
                }
                n
            }
            Ev::CharacterAdded(obj) => {
                let ov_raw = self.expr(obj, chain, None);
                let ov = self.materialize_for_event(&ov_raw, chain);
                let mut outputs = Vec::new();
                if let Some(p0) = params.first() {
                    if p0 != "_" {
                        outputs.push(("Character".to_string(), self.unique_binding(p0)));
                    }
                }
                let mut roots: Vec<String> = Vec::new();
                let n = self.emit_event_block(
                    "CharacterAdded",
                    vec![("Player".into(), self.val_input(&ov, "Object"))],
                    outputs,
                    chain,
                );
                n
            }
            Ev::Chatted(obj) => {
                let ov_raw = self.expr(obj, chain, None);
                let ov = self.materialize_for_event(&ov_raw, chain);
                let mut outputs = Vec::new();
                if let Some(p0) = params.first() {
                    if p0 != "_" {
                        outputs.push(("Message".to_string(), self.unique_binding(p0)));
                    }
                }
                let mut roots: Vec<String> = Vec::new();
                let n = self.emit_root(
                    "Chatted",
                    vec![("Player".into(), self.val_input(&ov, "Object"))],
                    outputs,
                    chain,
                );
                n
            }
            Ev::DescendantAdded(obj) => {
                let ov_raw = self.expr(obj, chain, None);
                let ov = self.materialize_for_event(&ov_raw, chain);
                let mut outputs = Vec::new();
                if let Some(p0) = params.first() {
                    if p0 != "_" {
                        outputs.push(("Descendant".to_string(), self.unique_binding(p0)));
                    }
                }
                let mut roots: Vec<String> = Vec::new();
                let n = self.emit_root(
                    "DescendantAdded",
                    vec![("Object".into(), self.val_input(&ov, "Object"))],
                    outputs,
                    chain,
                );
                n
            }
            Ev::Other => {
                self.note("GAP", format!("unsupported event source {:?}", base));
                return;
            }
        };

        self.push_scope();
        // re-declare param0 binding inside the body scope
        if let Some(p0) = params.first() {
            if p0 != "_" {
                let b = match &ev {
                    Ev::PartTouched(_) | Ev::PartTouchEnded(_)
                    | Ev::PlayerAdded | Ev::PlayerRemoving | Ev::Property(..)
                    | Ev::GuiClick(_) | Ev::TextBoxFocusLost(_) | Ev::AnyProperty(_)
                    | Ev::RenderStepped | Ev::CharacterAdded(_) | Ev::Chatted(_) | Ev::DescendantAdded(_) => {
                        if let Some(i) = self.index.get(&name).copied() {
                            self.blocks[i]
                                .outputs
                                .iter()
                                .find(|(_, b)| !b.is_empty())
                                .map(|(_, b)| b.clone())
                        } else {
                            None
                        }
                    }
                    Ev::InputBegan => {
                        // InputBegan: first param is an InputObject table.
                        // Create a table binding; the table will be populated
                        // from the event root outputs in body_chain below.
                        Some(self.unique_binding("inputObj"))
                    }
                    Ev::RemoteServer(_) => {
                        if let Some(i) = self.index.get(&name).copied() {
                            self.blocks[i]
                                .outputs
                                .iter()
                                .find(|(n, b)| n == "Player" && !b.is_empty())
                                .map(|(_, b)| b.clone())
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                if let Some(b) = b {
                    self.scopes.last_mut().unwrap().insert(p0.clone(), b);
                }
            }
        }
        if let Some(p1) = params.get(1) {
            if p1 != "_" {
                if let Ev::AnyProperty(_) = &ev {
                    if let Some(i) = self.index.get(&name).copied() {
                        let b1 = self.blocks[i]
                            .outputs
                            .iter()
                            .filter(|(_, b)| !b.is_empty())
                            .nth(1)
                            .map(|(_, b)| b.clone());
                        if let Some(b) = b1 {
                            self.scopes.last_mut().unwrap().insert(p1.clone(), b);
                        }
                    }
                }
                // InputBegan: 2nd param = Processed output
                if let Ev::InputBegan = &ev {
                    if let Some(i) = self.index.get(&name).copied() {
                        let proc_b = self.blocks[i]
                            .outputs
                            .iter()
                            .find(|(n, b)| n == "Processed" && !b.is_empty())
                            .map(|(_, b)| b.clone());
                        if let Some(b) = proc_b {
                            self.scopes.last_mut().unwrap().insert(p1.clone(), b);
                        }
                    }
                }
            }
        }

        let mut body_chain = Vec::new();
        // InputBegan: populate InputObject table from event root outputs
        if let Ev::InputBegan = &ev {
            if let Some(p0) = params.first() {
                if p0 != "_" {
                    if let Some(tbl) = self.lookup(p0) {
                        self.lua_tables.insert(tbl.clone());
                        self.emit("CreateTable", vec![], vec![("TableVariable".into(), tbl.clone())], &mut body_chain);
                        if let Some(i) = self.index.get(&name).copied() {
                            let fields: Vec<(String, String)> = self.blocks[i].outputs.iter()
                                .filter(|(n, b)| n != "EventConnection" && !b.is_empty())
                                .map(|(n, b)| (n.clone(), b.clone()))
                                .collect();
                            for (out_name, out_binding) in fields {
                                self.emit(
                                    "SetTableValue",
                                    vec![
                                        ("Table".into(), InputVal::Var(tbl.clone())),
                                        ("Key".into(), InputVal::Literal { value: out_name, vtype: "String".into() }),
                                        ("Value".into(), InputVal::Var(out_binding)),
                                    ],
                                    vec![],
                                    &mut body_chain,
                                );
                            }
                        }
                    }
                }
            }
        }
        // tuple element params: for server remotes the Player occupies tuple
        // element 1, so named params after it start at element 2; client
        // remotes and bindables use the whole tuple starting at element 1.
        // Tuple-parameter events (remotes/bindables): handler params are
        // TUPLEPARAM_Parameters_i OUTPUTS on the event block (RS convention,
        // T5) - just bring them into scope. Server slot 0 = Player.
        if matches!(
            &ev,
            Ev::RemoteServer(_) | Ev::RemoteClient(_) | Ev::Bindable(_)
        ) {
            let skipn = if matches!(&ev, Ev::RemoteServer(_)) { 1 } else { 0 };
            for (i2, prm) in params.iter().skip(skipn).enumerate() {
                if prm.as_str() == "_" {
                    continue;
                }
                let out_name = format!("TUPLEPARAM_Parameters_{}", i2 + 1);
                if let Some(b) = self.output_binding(&name, &out_name) {
                    self.scopes.last_mut().unwrap().insert(prm.clone(), b);
                }
            }
        }
        self.stmts(&body, &mut body_chain);
        // wrap the handler in its gate check when the connection was bound to
        // a variable (Disconnect clears the gate)
        if let Some(g) = gate {
            let if_name = {
                let n = self.display_name("If");
                self.validate("If", &Self::if_inputs(&g));
                self.blocks.push(EmittedBlock {
                    name: n.clone(),
                    btype: "If".into(),
                    inputs: Self::if_inputs(&g),
                    outputs: vec![],
                    children: body_chain,
                    else_child: None,
                });
                self.index.insert(n.clone(), self.blocks.len() - 1);
                n
            };
            self.set_children(&name, vec![if_name]);
        } else {
            self.set_children(&name, body_chain);
        }
        self.pop_scope();
    }

    fn call_expr(
        &mut self,
        f: &Expr,
        args: &[Expr],
        chain: &mut Vec<String>,
        bind: Option<&str>,
    ) -> Val {
        let out = bind.map(|s| s.to_string()).unwrap_or_else(|| self.temp());
        // RS unsupported deny-list (user-confirmed items only)
        let callee: Option<String> = match f {
            Expr::Index(b, k) => match (&**b, &**k) {
                (Expr::Name(ns), Expr::Str(m)) => Some(format!("{}.{}", ns, m)),
                _ => None,
            },
            Expr::Name(n) => Some(n.clone()),
            _ => None,
        };
        if let Some(cn) = &callee {
            if super::rs_support::rs_unsupported(cn) {
                self.flag_unsupported(cn, chain);
                return Val::Lit(LitVal::Nil);
            }
        }

        // value constructors: BrickColor.new / Vector3.new / Vector2.new / UDim2.new / CFrame.new / Color3.new / Color3.fromRGB / Instance.new
        if let Expr::Index(base, key) = f {
            if let (Expr::Name(ns), Expr::Str(meth)) = (&**base, &**key) {
                if meth == "new" || meth == "fromRGB" {
                    match (ns.as_str(), meth.as_str()) {
                        ("Random", "new") => {
                            // RNG synthesis - impl in rs_support (Park-Miller,
                            // binding-is-state)
                            self.rs_synth_random_new(args, &out, chain);
                            return Val::Var(out);
                        }
                        ("RaycastParams", "new") => {
                            // RS doesn't have RaycastParams construction -
                            // create a plain Lua table instead. The user's
                            // script sets .FilterType, .FilterDescendantsInstances
                            // etc. via SetTableValue on this table.
                            self.lua_tables.insert(out.clone());
                            self.emit("CreateTable", vec![], vec![("TableVariable".into(), out.clone())], chain);
                            return Val::Var(out);
                        }
                        ("BrickColor", "new") => {
                            // named colors -> palette numbers (wire form IS the
                            // number; user type table). Strings were silently
                            // becoming BrickColor 0 before (user-caught).
                            if let Some(Expr::Str(s)) = args.first() {
                                match super::rs_support::brickcolor_number(s) {
                                    Some(num) => {
                                        return Val::Lit(LitVal::BrickColor(num))
                                    }
                                    None => {
                                        self.note(
                                            "GAP",
                                            format!(
                                                "unknown BrickColor name {:?} - add it to brickcolor_number()",
                                                s
                                            ),
                                        );
                                        return Val::Lit(LitVal::BrickColor(0.0));
                                    }
                                }
                            }
                            let n = self.const_num(&args[0]).unwrap_or(0.0);
                            return Val::Lit(LitVal::BrickColor(n));
                        }
                        ("UDim2", "new") => {
                            let nums: Option<Vec<f64>> =
                                args.iter().map(|a| self.const_num(a)).collect();
                            if let Some(v) = nums {
                                if v.len() == 4 {
                                    return Val::Lit(LitVal::UDim2(v[0], v[1], v[2], v[3]));
                                }
                            }
                        }
                        ("Vector3", "new") => {
                            let nums: Option<Vec<f64>> =
                                args.iter().map(|a| self.const_num(a)).collect();
                            if let Some(v) = nums {
                                if v.len() == 3 {
                                    return Val::Lit(LitVal::Vector3(v[0], v[1], v[2]));
                                }
                            }
                        }
                        ("Vector2", "new") => {
                            let nums: Option<Vec<f64>> =
                                args.iter().map(|a| self.const_num(a)).collect();
                            if let Some(v) = nums {
                                if v.len() == 2 {
                                    return Val::Lit(LitVal::Vector2(v[0], v[1]));
                                }
                            }
                        }
                        ("Color3", "new") => {
                            let nums: Option<Vec<f64>> =
                                args.iter().map(|a| self.const_num(a)).collect();
                            if let Some(v) = nums {
                                if v.len() == 3 {
                                    return Val::Lit(LitVal::Color3(v[0], v[1], v[2]));
                                }
                            }
                        }
                        ("Color3", "fromRGB") => {
                            let nums: Option<Vec<f64>> =
                                args.iter().map(|a| self.const_num(a)).collect();
                            if let Some(v) = nums {
                                if v.len() == 3 {
                                    return Val::Lit(LitVal::Color3(v[0] / 255.0, v[1] / 255.0, v[2] / 255.0));
                                }
                            }
                        }
                        _ => {}
                    }
                    // non-constant args: emit constructor blocks
                    match (ns.as_str(), meth.as_str()) {
                        ("Instance", "new") if !args.is_empty() => {
                            let class_v = self.expr(&args[0], chain, None);
                            let parent_v = args.get(1).map(|p| self.expr(p, chain, None));
                            let mut inputs = vec![("ClassName".into(), self.val_input(&class_v, "String"))];
                            if let Some(pv) = parent_v {
                                inputs.push(("Parent".into(), self.val_input(&pv, "Object")));
                            } else {
                            // RS's CreateObject requires a Parent; real Luau leaves
                            // the instance parentless (nil). Wire-exact semantics
                            // (user-verified in RetroStudio): set a variable to
                            // nil (type token "Nil") and reference it by name.
                            let cls = match &class_v {
                                Val::Lit(LitVal::Str(s)) => s.clone(),
                                _ => String::new(),
                            };
                            // emit the Nil producer ONCE (user-caught B3:
                            // duplicated per CreateObject; chaining already
                            // guarantees the producer precedes every use)
                            if self.nil_ref_binding.is_none() {
                                let b = self.unique_binding("nil_ref");
                                self.emit(
                                    "SetVariable1",
                                    vec![("Value".into(),
                                          InputVal::Literal { value: String::new(), vtype: "Nil".into() })],
                                    vec![("VariableName".into(), b.clone())],
                                    chain,
                                );
                                self.nil_ref_binding = Some(b);
                            }
                            let binding = self.nil_ref_binding.clone().unwrap();
                            self.note("INFO", format!(
                                "Instance.new({:?}) with no parent: Parent bound to nil-reference {:?} (Luau nil-parent semantics; assign .Parent explicitly to place it)",
                                cls, binding
                            ));
                            inputs.push(("Parent".into(), InputVal::Var(binding)));
                        }
                            self.emit(
                                "CreateObject",
                                inputs,
                                vec![("Object".into(), out.clone())],
                                chain,
                            );
                            return Val::Var(out);
                        }
                        ("Vector2", "new") if args.len() == 2 => {
                            // SolveEquation adoption (user tip): one
                            // expression block, variable names resolved by RS
                            let eq = format!("Vector2.new({}, {})",
                                self.eq_expr(&args[0]),
                                self.eq_expr(&args[1])
                            );
                            self.emit(
                                "SolveEquation",
                                vec![("Equation".into(),
                                      InputVal::Literal { value: eq, vtype: "String".into() })],
                                vec![("Result".into(), out.clone())],
                                chain,
                            );
                            return Val::Var(out);
                        }
                        ("UDim2", "new") if args.len() == 4 => {
                            // SolveEquation adoption (user tip): one
                            // expression block, variable names resolved by RS
                            let eq = format!("UDim2.new({}, {}, {}, {})",
                                self.eq_expr(&args[0]),
                                self.eq_expr(&args[1]),
                                self.eq_expr(&args[2]),
                                self.eq_expr(&args[3])
                            );
                            self.emit(
                                "SolveEquation",
                                vec![("Equation".into(),
                                      InputVal::Literal { value: eq, vtype: "String".into() })],
                                vec![("Result".into(), out.clone())],
                                chain,
                            );
                            return Val::Var(out);
                        }
                        ("Color3", "new") if args.len() == 3 => {
                            // SolveEquation adoption (user tip): one
                            // expression block, variable names resolved by RS
                            let eq = format!("Color3.new({}, {}, {})",
                                self.eq_expr(&args[0]),
                                self.eq_expr(&args[1]),
                                self.eq_expr(&args[2])
                            );
                            self.emit(
                                "SolveEquation",
                                vec![("Equation".into(),
                                      InputVal::Literal { value: eq, vtype: "String".into() })],
                                vec![("Result".into(), out.clone())],
                                chain,
                            );
                            return Val::Var(out);
                        }
                        ("Color3", "fromRGB") if args.len() == 3 => {
                            // SolveEquation adoption (user tip): one
                            // expression block, variable names resolved by RS
                            let eq = format!("Color3.fromRGB({}, {}, {})",
                                self.eq_expr(&args[0]),
                                self.eq_expr(&args[1]),
                                self.eq_expr(&args[2])
                            );
                            self.emit(
                                "SolveEquation",
                                vec![("Equation".into(),
                                      InputVal::Literal { value: eq, vtype: "String".into() })],
                                vec![("Result".into(), out.clone())],
                                chain,
                            );
                            return Val::Var(out);
                        }
                        ("CFrame", "new") => match args.len() {
                            1 => {
                                let pv = self.expr(&args[0], chain, None);
                                self.emit(
                                    "ConstructCFrame",
                                    vec![
                                        ("Position".into(), self.val_input(&pv, "Vector3")),
                                        (
                                            "Rotation".into(),
                                            InputVal::Literal {
                                                value: "0,0,0".into(),
                                                vtype: "Vector3".into(),
                                            },
                                        ),
                                    ],
                                    vec![("CFrame".into(), out.clone())],
                                    chain,
                                );
                                return Val::Var(out);
                            }
                            6 => {
                                let pv = self.expr(&args[0], chain, None);
                                let rv = self.expr(&args[3], chain, None);
                                self.emit(
                                    "ConstructCFrame",
                                    vec![
                                        ("Position".into(), self.val_input(&pv, "Vector3")),
                                        ("Rotation".into(), self.val_input(&rv, "Vector3")),
                                    ],
                                    vec![("CFrame".into(), out.clone())],
                                    chain,
                                );
                                return Val::Var(out);
                            }
                            _ => {}
                        },
                        ("Vector3", "new") if args.len() == 3 => {
                            // SolveEquation adoption (user tip): one
                            // expression block, variable names resolved by RS
                            let eq = format!("Vector3.new({}, {}, {})",
                                self.eq_expr(&args[0]),
                                self.eq_expr(&args[1]),
                                self.eq_expr(&args[2])
                            );
                            self.emit(
                                "SolveEquation",
                                vec![("Equation".into(),
                                      InputVal::Literal { value: eq, vtype: "String".into() })],
                                vec![("Result".into(), out.clone())],
                                chain,
                            );
                            return Val::Var(out);
                        }
                        _ => {}
                    }
                }
            }
        }

        match f {
            Expr::Name(n) => match n.as_str() {
                "print" | "warn" => {
                    let t = if n == "print" { "Print" } else { "Warn" };
                    // multi-arg print (Luau tostring semantics - user-caught:
                    // extra args were silently dropped, and numeric literals
                    // would render as WIRE HEX). Strings pass through, numbers
                    // render DECIMAL, vars via ToStringAny (T5-proven block).
                    // Multi-arg print via Combine blocks (user ground truth:
                    // CombineStringWithNumber / CombineNumberWithString import
                    // with mode-0 inputs and auto-coerce numbers). No
                    // ToStringAny, no separator blocks.
                    #[derive(Clone)]
                    enum PrintPart {
                        Str(InputVal),
                        Num(InputVal),
                    }
                    let mut parts: Vec<PrintPart> = Vec::new();
                    for a in args {
                        let v = self.expr(a, chain, None);
                        match &v {
                            Val::Lit(LitVal::Str(s)) => {
                                parts.push(PrintPart::Str(InputVal::Implicit(s.clone())))
                            }
                            Val::Lit(LitVal::Num(n2)) => parts.push(PrintPart::Num(
                                InputVal::Implicit(format!("{}", n2)),
                            )),
                            Val::Lit(LitVal::Bool(b)) => parts.push(PrintPart::Str(
                                InputVal::Implicit(
                                    if *b { "true".into() } else { "false".into() },
                                ),
                            )),
                            Val::Lit(LitVal::Nil) => {
                                parts.push(PrintPart::Str(InputVal::Implicit("nil".into())))
                            }
                            // vars: route through ToStringAny so the value is
                            // guaranteed to be a string before going into
                            // Concatenate/Combine. The bak62 design routed
                            // vars as PrintPart::Num, but Combine blocks
                            // strictly require the Number input to BE a
                            // number at runtime; vars holding strings/bools/
                            // nil trigger type-mismatch errors:
                            //   CombineStringWithNumber1: 'Number' input
                            //   must be a 'number' got 'nil'/'string'
                            // (the proximate symptom of Bug A manifesting via
                            // this code path).
                            //
                            // Cost: one extra ToStringAny block per
                            // var-involving print. Benefit: type-correct for
                            // all runtime types. The Combine optimization is
                            // preserved for literal-literal pairs (where the
                            // type is statically known: a Number literal stays
                            // Num, a String literal stays Str).
                            Val::Var(bnd) => {
                                let s = self.temp();
                                self.emit(
                                    "ToStringAny",
                                    vec![("Input".into(), InputVal::Var(bnd.clone()))],
                                    vec![("String".into(), s.clone())],
                                    chain,
                                );
                                parts.push(PrintPart::Str(InputVal::Var(s)))
                            }
                            _ => parts.push(PrintPart::Str(InputVal::Implicit(String::new()))),
                        }
                    }
                    if parts.is_empty() {
                        self.emit(
                            t,
                            vec![("Text".into(), InputVal::Implicit(String::new()))],
                            vec![],
                            chain,
                        );
                        return Val::Lit(LitVal::Bool(true));
                    }
                    if parts.len() == 1 {
                        let only = match &parts[0] {
                            PrintPart::Str(iv) | PrintPart::Num(iv) => iv.clone(),
                        };
                        self.emit(t, vec![("Text".into(), only)], vec![], chain);
                        return Val::Lit(LitVal::Bool(true));
                    }
                    let mut cur: InputVal = match &parts[0] {
                        PrintPart::Str(iv) | PrintPart::Num(iv) => iv.clone(),
                    };
                    let final_out = self.temp();
                    let np = parts.len();
                    for (i, next) in parts[1..].iter().enumerate() {
                        let step_out = if i == np - 2 {
                            final_out.clone()
                        } else {
                            self.temp()
                        };
                        let next_iv = match next {
                            PrintPart::Str(iv) | PrintPart::Num(iv) => iv.clone(),
                        };
                        let cur_is_num = matches!(cur, InputVal::Var(_))
                            && matches!(parts.get(i), Some(PrintPart::Num(_)));
                        let next_is_num = matches!(next, PrintPart::Num(_))
                            && matches!(parts.get(i), Some(PrintPart::Str(_)));
                        if !cur_is_num && !next_is_num {
                            self.emit(
                                "Concatenate",
                                vec![
                                    ("FirstString".into(), cur),
                                    ("SecondString".into(), next_iv),
                                ],
                                vec![("Result".into(), step_out.clone())],
                                chain,
                            );
                        } else if next_is_num {
                            self.emit(
                                "CombineStringWithNumber",
                                vec![
                                    ("String".into(), cur),
                                    ("Number".into(), next_iv),
                                    ("AddSpace".into(), InputVal::Implicit("0".into())),
                                ],
                                vec![("Combined".into(), step_out.clone())],
                                chain,
                            );
                        } else {
                            self.emit(
                                "CombineNumberWithString",
                                vec![
                                    ("Number".into(), cur),
                                    ("String".into(), next_iv),
                                    ("AddSpace".into(), InputVal::Implicit("0".into())),
                                ],
                                vec![("Combined".into(), step_out.clone())],
                                chain,
                            );
                        }
                        cur = InputVal::Var(step_out);
                    }
                    self.emit(
                        t,
                        vec![("Text".into(), InputVal::Var(final_out))],
                        vec![],
                        chain,
                    );
                    return Val::Lit(LitVal::Bool(true));
                }
                "tostring" => {
                    let av = self.expr(&args[0], chain, None);
                    self.emit(
                        "ToStringAny",
                        vec![("Input".into(), self.val_input(&av, "Any"))],
                        vec![("String".into(), out.clone())],
                        chain,
                    );
                    return Val::Var(out);
                }
                "tonumber" => {
                    let av = self.expr(&args[0], chain, None);
                    self.emit(
                        "ToNumberAny",
                        vec![("Input".into(), self.val_input(&av, "Any"))],
                        vec![("Number".into(), out.clone())],
                        chain,
                    );
                    return Val::Var(out);
                }
                "typeof" | "type" => {
                    let av = self.expr(&args[0], chain, None);
                    self.emit(
                        "TypeOf",
                        vec![("Variable".into(), self.val_input(&av, "Any"))],
                        vec![("Type".into(), out.clone())],
                        chain,
                    );
                    return Val::Var(out);
                }
                "wait" => {
                    let tv = if let Some(t) = args.first() {
                        self.expr(t, chain, None)
                    } else {
                        Val::Lit(LitVal::Num(0.0))
                    };
                    self.emit(
                        "Wait",
                        vec![("Time".into(), self.val_input(&tv, "Number"))],
                        vec![],
                        chain,
                    );
                    return Val::Lit(LitVal::Bool(true));
                }
                "tick" => {
                    // tick() is an alias for os.clock() (user directive):
                    // emit the identical OsClock block, no note.
                    self.emit(
                        "OsClock",
                        vec![],
                        vec![("Answer".into(), out.clone())],
                        chain,
                    );
                    return Val::Var(out);
                }
                _ => {
                    if let Some(fb) = self.lookup(n) {
                        // Math-method alias inlining: if this Name was
                        // bound via `local math_X = math.<method>`, inline
                        // the call back to `math.<method>(args)` and route
                        // through SolveEquation (1 block) instead of the
                        // general ExecuteFunction path (2+ blocks: arg
                        // eval + ExecuteFunction). Verified safe at runtime:
                        // SolveEquation resolves math.* (G1/G4-proven).
                        if let Some(method) = self.math_aliases.get(&fb).cloned() {
                            let rewritten_f = Expr::Index(
                                Box::new(Expr::Name("math".into())),
                                Box::new(Expr::Str(method)),
                            );
                            let rewritten_call = Expr::Call(Box::new(rewritten_f), args.to_vec());
                            let eq = self.eq_expr(&rewritten_call);
                            self.folded += 1;
                            self.emit(
                                "SolveEquation",
                                vec![("Equation".into(),
                                      InputVal::Literal { value: eq, vtype: "String".into() })],
                                vec![("Result".into(), out.clone())],
                                chain,
                            );
                            return Val::Var(out);
                        }
                        // RS tuple convention (user ground truth, T8): single
                        // argument rides a TUPLEPARAM_Parameters_1 input that
                        // Parameters references; results come back as
                        // TUPLEPARAM_ReturnedValues_i outputs when the call
                        // feeds a binding.
                        if args.len() == 1 {
                            let av = self.expr(&args[0], chain, None);
                            let iv = self.val_input(&av, "Any");
                            let inputs = vec![
                                ("Function".into(), InputVal::Var(fb.clone())),
                                ("Parameters".into(),
                                 InputVal::Implicit("TUPLEPARAM_Parameters_1".into())),
                                ("TUPLEPARAM_Parameters_1".into(), iv),
                            ];
                            // RS results convention (user hand-fix, T8):
                            // used -> count 1 + named TUPLEPARAM output;
                            // discarded -> count 0, NO tuple outputs (a
                            // binding name in the count slot breaks RS).
                            let discard = std::mem::take(&mut self.exprstmt_discard);
                            let outs = if discard {
                                // [patch9] true discard (T8): count 0
                                vec![("TUPLE_ReturnedValues".into(), "0".into())]
                            } else {
                                if bind.is_some() {
                                    let arity = self.fn_returns.get(&fb).copied();
                                    if let Some(a) = arity {
                                        if a != 1 {
                                            self.note(
                                                "VERIFY",
                                                format!(
                                                    "call to {:?} returns {} values; binding only the first (multi-select unverified)",
                                                    n, a
                                                ),
                                            );
                                        }
                                    }
                                }
                                // [patch9] bound OR consumed-as-operand:
                                // count 1 + named output (T8 used shape)
                                vec![
                                    ("TUPLE_ReturnedValues".into(), "1".into()),
                                    ("TUPLEPARAM_ReturnedValues_1".into(), out.clone()),
                                ]
                            };
                            self.emit("ExecuteFunction", inputs, outs, chain);
                            // [patch10] register table-returning call results:
                            // the TUPLEPARAM output is a wire, not a variable,
                            // so downstream Table-typed raw-name reads and
                            // SetTableValue dispatch need a registered,
                            // lua_tables-tracked binding (patch8 ground truth).
                            // [patch12] table-returning call results: TRACK
                            // the binding for SetTableValue dispatch, but
                            // emit no SetVariable1 - the TUPLEPARAM output
                            // binding IS the variable (wire semantics; the
                            // patch10 self-named registration resolved to
                            // nil in-game, charcode runs 2/3).
                            if !discard
                                && bind.is_some()
                                && (self.fn_ret_table.contains(&fb)
                                    || self.fn_ret_passthru.get(&fb).map_or(
                                        false,
                                        |idx| *idx == 0 && self.val_is_table(&av),
                                    ))
                            {
                                self.lua_tables.insert(out.clone());
                            }
                            return if discard { Val::Lit(LitVal::Nil) } else { Val::Var(out) };
                        }
                        let param_inputs = self.tuple_param_inputs(args, chain);
                        let discard = std::mem::take(&mut self.exprstmt_discard);
                        let outs = if discard {
                            // [patch9] true discard (T8): count 0
                            vec![("TUPLE_ReturnedValues".into(), "0".into())]
                        } else {
                            // [patch9] bound OR consumed-as-operand: count 1
                            vec![
                                ("TUPLE_ReturnedValues".into(), "1".into()),
                                ("TUPLEPARAM_ReturnedValues_1".into(), out.clone()),
                            ]
                        };
                        self.emit(
                            "ExecuteFunction",
                            {
                                let mut v = vec![(
                                    "Function".into(),
                                    InputVal::Var(fb.clone()),
                                )];
                                v.extend(param_inputs);
                                v
                            },
                            outs,
                            chain,
                        );
                        // [patch12] table-returning call results: TRACK the
                        // binding for SetTableValue dispatch, no block (the
                        // TUPLEPARAM output binding IS the variable). These
                        // two sites produced the got2/db/b2 "X to X" blocks
                        // that survived v11 and niled in-game.
                        if !discard && bind.is_some() && self.fn_ret_table.contains(&fb) {
                            self.lua_tables.insert(out.clone());
                        }
                        return if discard { Val::Lit(LitVal::Nil) } else { Val::Var(out) };
                    } else {
                        self.note("GAP", format!("call of undeclared function {:?}", n));
                        return Val::Lit(LitVal::Bool(true));
                    }
                }
            },
            Expr::Index(base, key) => {
                if let (Expr::Name(ns), Expr::Str(meth)) = (&**base, &**key) {
                    match (ns.as_str(), meth.as_str()) {
                        ("math", "floor" | "ceil" | "round") => {
                            let v = self.expr(&args[0], chain, None);
                            let t = match meth.as_str() {
                                "floor" => "Floor",
                                "ceil" => "Ceiling",
                                _ => "Round",
                            };
                            self.emit(
                                "Round",
                                vec![
                                    ("Number".into(), self.val_input(&v, "Number")),
                                    (
                                        "Type".into(),
                                        InputVal::Literal { value: t.to_string(), vtype: "String".into() },
                                    ),
                                ],
                                vec![("Result".into(), out.clone())],
                                chain,
                            );
                            return Val::Var(out);
                        }
                        ("math", "random") => {
                            let (lo, hi) = match args.len() {
                                2 => (
                                    self.expr(&args[0], chain, None),
                                    self.expr(&args[1], chain, None),
                                ),
                                1 => (
                                    Val::Lit(LitVal::Num(1.0)),
                                    self.expr(&args[0], chain, None),
                                ),
                                _ => (Val::Lit(LitVal::Num(0.0)), Val::Lit(LitVal::Num(1.0))),
                            };
                            self.emit(
                                "RandomNumber",
                                vec![
                                    ("Minimum".into(), self.val_input(&lo, "Number")),
                                    ("Maximum".into(), self.val_input(&hi, "Number")),
                                ],
                                vec![("Result".into(), out.clone())],
                                chain,
                            );
                            return Val::Var(out);
                        }
                        ("math", "huge") => return Val::Lit(LitVal::Num(1e308)),
                        ("math", "min" | "max") if args.len() == 2 => {
                            // SolveEquation adoption (user tip): one
                            // expression block, variable names resolved by RS
                            let eq = format!(
                                "math.{}({}, {})",
                                meth,
                                self.eq_expr(&args[0]),
                                self.eq_expr(&args[1])
                            );
                            self.emit(
                                "SolveEquation",
                                vec![("Equation".into(),
                                      InputVal::Literal { value: eq, vtype: "String".into() })],
                                vec![("Result".into(), out.clone())],
                                chain,
                            );
                            return Val::Var(out);
                        }
                        ("string", "lower" | "upper") => {
                            let v = self.expr(&args[0], chain, None);
                            let t = if meth == "lower" { "Lower" } else { "Upper" };
                            self.emit(
                                t,
                                vec![("String".into(), self.val_input(&v, "String"))],
                                vec![("Result".into(), out.clone())],
                                chain,
                            );
                            return Val::Var(out);
                        }
                        ("string", "sub") => {
                            let v = self.expr(&args[0], chain, None);
                            let s = self.expr(&args[1], chain, None);
                            let e = self
                                .expr(args.get(2).unwrap_or(&Expr::Number(-1.0)), chain, None);
                            self.emit(
                                "Substring",
                                vec![
                                    ("String".into(), self.val_input(&v, "String")),
                                    ("StartPoint".into(), self.val_input(&s, "Number")),
                                    ("EndPoint".into(), self.val_input(&e, "Number")),
                                ],
                                vec![("Result".into(), out.clone())],
                                chain,
                            );
                            return Val::Var(out);
                        }
                        ("string", "find") => {
                            let v = self.expr(&args[0], chain, None);
                            let p = self.expr(&args[1], chain, None);
                            self.emit(
                                "FindString",
                                vec![
                                    ("String".into(), self.val_input(&v, "String")),
                                    ("Pattern".into(), self.val_input(&p, "String")),
                                ],
                                vec![("Result".into(), out.clone())],
                                chain,
                            );
                            return Val::Var(out);
                        }
                        ("string", "len") => {
                            let v = self.expr(&args[0], chain, None);
                            // [patch14] same rationale as UnOp::Len: one
                            // length mechanism, no type guessing.
                            if let Val::Var(name) = &v {
                                self.emit(
                                    "SolveEquation",
                                    vec![(
                                        "Equation".into(),
                                        InputVal::Literal {
                                            value: format!("#{}", name),
                                            vtype: "String".into(),
                                        },
                                    )],
                                    vec![("Result".into(), out.clone())],
                                    chain,
                                );
                            } else {
                                self.emit(
                                    "GetLength",
                                    vec![("String".into(), self.val_input(&v, "String"))],
                                    vec![("Length".into(), out.clone())],
                                    chain,
                                );
                            }
                            return Val::Var(out);
                        }
                        ("string", "gsub") if args.len() >= 3 => {
                            let s = self.expr(&args[0], chain, None);
                            let p = self.expr(&args[1], chain, None);
                            let r = self.expr(&args[2], chain, None);
                            self.emit(
                                "GSubString",
                                vec![
                                    ("String".into(), self.val_input(&s, "String")),
                                    ("Replacement".into(), self.val_input(&r, "String")),
                                    ("Pattern".into(), self.val_input(&p, "String")),
                                ],
                                vec![("Result".into(), out.clone())],
                                chain,
                            );
                            return Val::Var(out);
                        }
                        ("table", "clear") => {
                            // lowered to a descending TableRemove loop
                            let v = self.expr(&args[0], chain, None);
                            let len_out = self.temp();
                            self.emit(
                                "GetTableLength",
                                vec![("Table".into(), self.val_input(&v, "Table"))],
                                vec![("Length".into(), len_out.clone())],
                                chain,
                            );
                            let place = self.temp();
                            let fl = self.emit(
                                "ForLoop2",
                                vec![
                                    ("Initial".into(), InputVal::Var(len_out)),
                                    (
                                        "Increment".into(),
                                        InputVal::Literal { value: "-1".into(), vtype: "Number".into() },
                                    ),
                                    (
                                        "Maximum".into(),
                                        InputVal::Literal { value: "1".into(), vtype: "Number".into() },
                                    ),
                                ],
                                vec![("Place".into(), place.clone())],
                                chain,
                            );
                            let mut body_chain = Vec::new();
                            self.emit(
                                "TableRemove",
                                vec![
                                    ("Table".into(), self.val_input(&v, "Table")),
                                    ("Position".into(), InputVal::Var(place)),
                                ],
                                vec![],
                                &mut body_chain,
                            );
                            self.set_children(&fl, body_chain);
                            self.note(
                                "WORKAROUND",
                                "table.clear(t) lowered to a descending TableRemove loop (no \
                                 TableClear block exists)"
                                    .into(),
                            );
                            return Val::Lit(LitVal::Bool(true));
                        }
                        ("table", "insert") => {
                            let tv = self.expr(&args[0], chain, None);
                            let vv = self.expr(&args[1], chain, None);
                            self.emit(
                                "TableAppend",
                                vec![
                                    ("Table".into(), self.val_input(&tv, "Table")),
                                    ("Value".into(), self.val_input(&vv, "Any")),
                                ],
                                vec![],
                                chain,
                            );
                            return Val::Lit(LitVal::Bool(true));
                        }
                        ("table", "remove") => {
                            let tv = self.expr(&args[0], chain, None);
                            let pv = self.expr(&args[1], chain, None);
                            self.emit(
                                "TableRemove",
                                vec![
                                    ("Table".into(), self.val_input(&tv, "Table")),
                                    ("Position".into(), self.val_input(&pv, "Number")),
                                ],
                                vec![],
                                chain,
                            );
                            return Val::Lit(LitVal::Bool(true));
                        }
                        ("table", "find") => {
                            let tv = self.expr(&args[0], chain, None);
                            let vv = self.expr(&args[1], chain, None);
                            self.emit(
                                "TableFind",
                                vec![
                                    ("Value".into(), self.val_input(&vv, "String")),
                                    ("Table".into(), self.val_input(&tv, "Table")),
                                    (
                                        "Init".into(),
                                        InputVal::Literal { value: "1".into(), vtype: "Number".into() },
                                    ),
                                ],
                                vec![("Key".into(), out.clone())],
                                chain,
                            );
                            return Val::Var(out);
                        }
                        ("os", "time") => {
                            self.emit("OsTime", vec![], vec![("Answer".into(), out.clone())], chain);
                            return Val::Var(out);
                        }
                        ("os", "clock") => {
                            self.emit("OsClock", vec![], vec![("Answer".into(), out.clone())], chain);
                            return Val::Var(out);
                        }
                        ("task", "wait") => {
                            match args.first() {
                                None | Some(Expr::Nil) => {
                                    self.note(
                                        "GAP",
                                        "task.wait() outside loop-tail position needs an \
                                         explicit duration"
                                            .into(),
                                    );
                                }
                                Some(t) => {
                                    let tv = self.expr(t, chain, None);
                                    self.emit(
                                        "Wait",
                                        vec![("Time".into(), self.val_input(&tv, "Number"))],
                                        vec![],
                                        chain,
                                    );
                                }
                            }
                            return Val::Lit(LitVal::Bool(true));
                        }
                        ("task", "spawn" | "defer") => {
                            if meth == "defer" {
                                self.note("WORKAROUND", "task.defer lowered to Spawn".into());
                            }
                            let (fname, rest): (String, &[Expr]) = match args {
                                [Expr::Function(p, body), rest @ ..] => {
                                    let name = format!("spawned{}", self.temp_counter + 1);
                                    self.function(&name, p, body, chain);
                                    self.note(
                                        "WORKAROUND",
                                        format!("inline closure converted to DefineFunction {}", name),
                                    );
                                    (name, rest)
                                }
                                [Expr::Name(fnname), rest @ ..] => (fnname.clone(), rest),
                                _ => {
                                    self.note("GAP", "task.spawn with non-function first arg".into());
                                    return Val::Lit(LitVal::Bool(true));
                                }
                            };
                            if let Some(fb) = self.lookup(&fname) {
                                let param_inputs = self.tuple_param_inputs(rest, chain);
                                self.emit(
                                    "Spawn",
                                    {
                                        let mut v = vec![(
                                            "Function".into(),
                                            InputVal::Var(fb.clone()),
                                        )];
                                        v.extend(param_inputs);
                                        v
                                    },
                                    vec![],
                                    chain,
                                );
                            }
                            return Val::Lit(LitVal::Bool(true));
                        }
                        ("task", "delay") => {
                            // task.delay(time, callback) → Spawn a wrapper
                            // DefineFunction whose body is: Wait(time) +
                            // ExecuteFunction(callback). RS has no native
                            // task.delay — this synthesizes the behavior.
                            let (time_expr, cb_expr) = match args {
                                [t, c] => (t, c),
                                _ => {
                                    self.note("GAP", "task.delay expects (time, callback)".into());
                                    return Val::Lit(LitVal::Nil);
                                }
                            };
                            // 1. Evaluate time → store in a variable so the
                            //    wrapper function can access it at runtime.
                            let time_val = self.expr(time_expr, chain, None);
                            let time_binding = match &time_val {
                                Val::Var(b) => b.clone(),
                                _ => {
                                    let t = self.temp();
                                    self.emit(
                                        "SetVariable1",
                                        vec![("Value".into(), self.val_input(&time_val, "Number"))],
                                        vec![("VariableName".into(), t.clone())],
                                        chain,
                                    );
                                    t
                                }
                            };
                            // 2. Convert callback to a DefineFunction.
                            let cb_binding = match cb_expr {
                                Expr::Function(p, body) => {
                                    let name = format!("delayCb{}", self.temp_counter + 1);
                                    self.function(&name, p, body, chain);
                                    self.note("WORKAROUND",
                                        format!("task.delay callback → DefineFunction {}", name));
                                    self.lookup(&name).unwrap_or(name)
                                }
                                Expr::Name(fnname) => {
                                    self.lookup(fnname).unwrap_or_else(|| sanitize(fnname))
                                }
                                _ => {
                                    self.note("GAP", "task.delay with non-function callback".into());
                                    return Val::Lit(LitVal::Nil);
                                }
                            };
                            // 3. Create wrapper DefineFunction:
                            //    body = Wait(time) + ExecuteFunction(callback)
                            let wrapper_name = format!("delayWrapper{}", self.temp_counter + 1);
                            let mut roots: Vec<String> = Vec::new();
                            let fn_root = self.emit_root(
                                "DefineFunction",
                                vec![],
                                vec![
                                    ("Function".into(), wrapper_name.clone()),
                                    ("TUPLE_Parameters".into(), "0".into()),
                                ],
                                &mut roots,
                            );
                            let mut body_chain = Vec::new();
                            self.emit(
                                "Wait",
                                vec![("Time".into(), InputVal::Var(time_binding))],
                                vec![],
                                &mut body_chain,
                            );
                            self.emit(
                                "ExecuteFunction",
                                vec![
                                    ("Function".into(), InputVal::Var(cb_binding)),
                                    ("Parameters".into(), InputVal::Implicit(String::new())),
                                ],
                                vec![("TUPLE_ReturnedValues".into(), "0".into())],
                                &mut body_chain,
                            );
                            self.set_children(&fn_root, body_chain);
                            for r in roots {
                                chain.push(r);
                            }
                            // 4. Spawn the wrapper (non-blocking).
                            // Spawn's schema REQUIRES both `Function` and
                            // `Parameters` inputs (vs_blocks.json: Spawn has
                            // inputs Function:Function + Parameters:Tuple).
                            // task.spawn synthesis uses tuple_param_inputs(..)
                            // which always yields [("Parameters", Implicit(""))]
                            // even for zero args; without it RS rejects the
                            // block at load time (compile failure). Mirror that.
                            self.emit(
                                "Spawn",
                                vec![
                                    ("Function".into(), InputVal::Var(wrapper_name)),
                                    ("Parameters".into(), InputVal::Implicit(String::new())),
                                ],
                                vec![],
                                chain,
                            );
                            return Val::Lit(LitVal::Nil);
                        }
                        _ => {}
                    }
                }
                // unknown call: math.* gets a SolveEquation chance (user tip:
                // RS's equation evaluator is the universal escape hatch -
                // probes decide if it knows noise/abs/clamp). Everything else:
                // unsupported -> comment + note + NIL (never Bool(true): truthy
                // garbage made 'if rng:NextNumber() > x' always-true).
                // routing (policy in rs_support.rs): unknown math.* ->
                // SolveEquation (G1/G4-proven escape hatch); everything else
                // -> unsupported
                if super::rs_support::is_math_call(f) {
                    let call = Expr::Call(Box::new(f.clone()), args.to_vec());
                    let eq = self.eq_expr(&call);
                    self.emit(
                        "SolveEquation",
                        vec![("Equation".into(),
                              InputVal::Literal { value: eq, vtype: "String".into() })],
                        vec![("Result".into(), out.clone())],
                        chain,
                    );
                    return Val::Var(out);
                }
                let cname = match f {
                    Expr::Index(b, k) => match (&**b, &**k) {
                        (Expr::Name(ns), Expr::Str(m)) => format!("{}.{}", ns, m),
                        _ => format!("{:?}", f),
                    },
                    other2 => format!("{:?}", other2),
                };
                self.flag_unsupported(&cname, chain);
                return Val::Lit(LitVal::Nil);
            }
            // immediately-invoked closure result: runWordRound(a, b)(word)
            Expr::Call(inner_f, inner_args) => {
                if let Expr::Name(n) = &**inner_f {
                    // outer function runs for its side effects
                    if let Some(fb) = self.lookup(n) {
                        let param_inputs = self.tuple_param_inputs(inner_args, chain);
                        self.emit(
                            "ExecuteFunction",
                            {
                                let mut v = vec![(
                                    "Function".into(),
                                    InputVal::Var(fb.clone()),
                                )];
                                v.extend(param_inputs);
                                v
                            },
                            // result discarded: count 0 (user hand-fix, T8)
                            vec![("TUPLE_ReturnedValues".into(), "0".into())],
                            chain,
                        );
                    }
                    // inner function receives the outer call's args
                    let inner_name = format!("{}Inner", n);
                    if let Some(fb) = self.lookup(&inner_name) {
                        let param_inputs = self.tuple_param_inputs(args, chain);
                        let discard = std::mem::take(&mut self.exprstmt_discard);
                        let outs = if discard {
                            // [patch9] true discard (T8): count 0
                            vec![("TUPLE_ReturnedValues".into(), "0".into())]
                        } else {
                            // [patch9] bound OR consumed-as-operand: count 1
                            vec![
                                ("TUPLE_ReturnedValues".into(), "1".into()),
                                ("TUPLEPARAM_ReturnedValues_1".into(), out.clone()),
                            ]
                        };
                        self.emit(
                            "ExecuteFunction",
                            {
                                let mut v = vec![(
                                    "Function".into(),
                                    InputVal::Var(fb.clone()),
                                )];
                                v.extend(param_inputs);
                                v
                            },
                            outs,
                            chain,
                        );
                        // [patch12] table-returning call results: TRACK the
                        // binding for SetTableValue dispatch, no block (the
                        // TUPLEPARAM output binding IS the variable). These
                        // two sites produced the got2/db/b2 "X to X" blocks
                        // that survived v11 and niled in-game.
                        if !discard && bind.is_some() && self.fn_ret_table.contains(&fb) {
                            self.lua_tables.insert(out.clone());
                        }
                        return if discard { Val::Lit(LitVal::Nil) } else { Val::Var(out) };
                    }
                    self.note("GAP", format!("immediately-invoked call on unknown function {:?}", n));
                    return Val::Lit(LitVal::Bool(true));
                }
                self.note("GAP", "immediately-invoked call on complex expression".into());
                return Val::Lit(LitVal::Bool(true));
            }
            _ => {
                self.note("GAP", format!("unsupported call target {:?}", f));
                return Val::Lit(LitVal::Bool(true));
            }
        }
    }

    fn table_expr(
        &mut self,
        fields: &[TableField],
        chain: &mut Vec<String>,
        bind: Option<&str>,
    ) -> Val {
        let out = bind.map(|s| s.to_string()).unwrap_or_else(|| self.temp());
        if fields.is_empty() {
            self.lua_tables.insert(out.clone());
        self.emit("CreateTable", vec![], vec![("TableVariable".into(), out.clone())], chain);
            return Val::Var(out);
        }
        // [patch19][patch22] user-proven (run 7): a SolveEquation can
        // BUILD a table - equation text `{1, 2, 3}` compiles and its
        // Result wires to the table variable.
        //
        // [patch22] run-9 root cause, found by the user (2026-11-08):
        // RetroStudio parses the equation as ONE raw chunk - it is NOT
        // Roblox-style per-value tokenizing. Strings carry NO escape
        // sequences: a raw backslash, space, bracket or single quote
        // inside a literal is just a character, and the ONLY terminator
        // is `"`. v20 rendered the quote entry Lua-style (`\"`), which
        // the raw parser reads as string-close + string-OPEN - the
        // `"""` break - and the whole equation died ("attempt to index
        // nil with number" on Solve Equation1/2). Explicit keys were
        // never the problem: the user's working remap
        // `{[32] = ")", [33] = "(", [34] = "`"}` compiles in game.
        // So explicit-key braces are re-armed, string literals ride
        // equations RAW (no escaping - `\\` would silently become TWO
        // backslashes), and entries whose string still holds a `"` (or
        // a control char - the text is single-line printable) defer to
        // one SetTableValue each below. patch21's positional-only ban
        // and v20's blanket gate are both superseded.
        let mut safe: Vec<String> = Vec::with_capacity(fields.len());
        let mut deferred: Vec<(usize, &TableField)> = Vec::new();
        for (i, f) in fields.iter().enumerate() {
            let piece = match f {
                // positional numbers keep the run-7 tested shape
                TableField::Pos(e) => match e {
                    Expr::Number(n) => Some(format!("[{}] = {}", i + 1, n)),
                    _ => None,
                },
                TableField::Named(k, e) => match (lua_eq_str(k), lua_eq_atom(e)) {
                    (Some(ks), Some(vs)) => Some(format!("[{}] = {}", ks, vs)),
                    _ => None,
                },
                TableField::Bracketed(k, e) => match (lua_eq_atom(k), lua_eq_atom(e)) {
                    (Some(ks), Some(vs)) => Some(format!("[{}] = {}", ks, vs)),
                    _ => None,
                },
            };
            match piece {
                Some(p) => safe.push(p),
                None => deferred.push((i, f)),
            }
        }
        if deferred.is_empty() {
            // all-positional constructors keep the run-7 positional
            // render; dictionaries ride explicit keys (user-proven
            // remap shape, run 9)
            let all_pos = fields
                .iter()
                .all(|f| matches!(f, TableField::Pos(e) if matches!(e, Expr::Number(_))));
            let items: Vec<String> = if all_pos {
                fields
                    .iter()
                    .map(|f| match f {
                        TableField::Pos(e) => match e {
                            Expr::Number(n) => format!("{}", n),
                            _ => unreachable!("v22 gate guarantees Number"),
                        },
                        _ => unreachable!("v22 gate guarantees Pos"),
                    })
                    .collect()
            } else {
                safe
            };
            self.lua_tables.insert(out.clone());
            self.emit(
                "SolveEquation",
                vec![(
                    "Equation".into(),
                    InputVal::Literal {
                        value: format!("{{{}}}", items.join(", ")),
                        vtype: "String".into(),
                    },
                )],
                vec![("Result".into(), out.clone())],
                chain,
            );
            return Val::Var(out);
        }
        if !safe.is_empty() {
            // mixed: the equation builds every quote-free entry with
            // EXPLICIT keys (stable indices even with deferred holes),
            // then the deferred entries fill in via SetTableValue below.
            self.lua_tables.insert(out.clone());
            self.emit(
                "SolveEquation",
                vec![(
                    "Equation".into(),
                    InputVal::Literal {
                        value: format!("{{{}}}", safe.join(", ")),
                        vtype: "String".into(),
                    },
                )],
                vec![("Result".into(), out.clone())],
                chain,
            );
        }
        // [patch22] full fallback (nothing was equation-safe) or the
        // deferred fill-ins of the mixed path above (whose equation
        // already built the table): CreateTable only when no equation
        // built it. TablePack stays dead - patch15.
        if safe.is_empty() {
            self.lua_tables.insert(out.clone());
            self.emit("CreateTable", vec![], vec![("TableVariable".into(), out.clone())], chain);
        }
        for (i, f) in deferred {
            match f {
                TableField::Pos(e) => {
                    let v = self.expr(e, chain, None);
                    self.emit(
                        "SetTableValue",
                        vec![
                            // [patch15] Table rides as a mode-0 raw name -
                            // the exact fill shape of every working table
                            // in the run-5 payload (census: 226 mode-0
                            // fills, 0 mode-2).
                            ("Table".into(), self.val_input(&Val::Var(out.clone()), "Table")),
                            (
                                "Key".into(),
                                InputVal::Literal { value: hexs((i + 1) as f64), vtype: "Number".into() },
                            ),
                            ("Value".into(), self.val_input(&v, "Any")),
                        ],
                        vec![],
                        chain,
                    );
                }
                TableField::Named(k, e) => {
                    let v = self.expr(e, chain, None);
                    self.emit(
                        "SetTableValue",
                        vec![
                            ("Table".into(), InputVal::Var(out.clone())),
                            (
                                "Key".into(),
                                InputVal::Literal { value: k.clone(), vtype: "String".into() },
                            ),
                            ("Value".into(), self.val_input(&v, "Any")),
                        ],
                        vec![],
                        chain,
                    );
                }
                TableField::Bracketed(k, e) => {
                    let kv = self.expr(k, chain, None);
                    let v = self.expr(e, chain, None);
                    self.emit(
                        "SetTableValue",
                        vec![
                            ("Table".into(), InputVal::Var(out.clone())),
                            ("Key".into(), self.val_input(&kv, "Any")),
                            ("Value".into(), self.val_input(&v, "Any")),
                        ],
                        vec![],
                        chain,
                    );
                }
            }
        }
        return Val::Var(out);
    }

    /// Input value for a tuple of expressions.
    pub fn tuple_input(&mut self, exprs: &[Expr], chain: &mut Vec<String>) -> InputVal {
        if exprs.is_empty() {
            return InputVal::Implicit(String::new());
        }
        let lits: Option<Vec<String>> = exprs
            .iter()
            .map(|e| match self.const_lit(e) {
                Some(LitVal::Str(s)) if !s.contains(',') => Some(s),
                Some(LitVal::Num(n)) => Some(hexs(n)),
                Some(LitVal::Bool(b)) => Some(if b { "1".into() } else { "0".into() }),
                _ => None,
            })
            .collect();
        if let Some(parts) = lits {
            return InputVal::Literal { value: parts.join(","), vtype: "Tuple".into() };
        }
        if exprs.len() == 1 {
            let v = self.expr(&exprs[0], chain, None);
            if let Val::Lit(l) = &v {
                return self.lit_input(l);
            }
            self.note(
                "VERIFY",
                "single-variable tuple passed as a direct variable reference (1-tuple treated \
                 as the value) - confirm in game"
                    .into(),
            );
            return self.val_input(&v, "Any");
        }
        // multiple non-literal values: table -> TableUnpack -> tuple var
        let tbl = self.temp();
        self.emit("CreateTable", vec![], vec![("TableVariable".into(), tbl.clone())], chain);
        for (i, e) in exprs.iter().enumerate() {
            let v = self.expr(e, chain, None);
            self.emit(
                "SetTableValue",
                vec![
                    ("Table".into(), InputVal::Var(tbl.clone())),
                    (
                        "Key".into(),
                        InputVal::Literal { value: hexs((i + 1) as f64), vtype: "Number".into() },
                    ),
                    ("Value".into(), self.val_input(&v, "Any")),
                ],
                vec![],
                chain,
            );
        }
        let tuple_var = self.temp();
        self.emit(
            "TableUnpack",
            vec![("Table".into(), InputVal::Var(tbl))],
            vec![("TUPLE_Values".into(), tuple_var.clone())],
            chain,
        );
        InputVal::Var(tuple_var)
    }

    pub fn finish(self) -> (Vec<EmittedBlock>, Vec<Note>) {
        (self.blocks, self.notes)
    }
}

// [patch22] A string may ride a RetroStudio equation only when it is
// printable single-line ASCII with no `"` in it: RS parses the equation
// as one raw chunk with NO escape sequences, so a `"` inside a literal
// terminates it early (the `"""` break) and shreds the rest of the text.
// Control chars / newlines cannot live in single-line equation text.
fn eq_str_ok(s: &str) -> bool {
    !s.contains('"') && s.chars().all(|c| matches!(c, ' '..='~'))
}

// [patch22] Render an expression as an atom of a brace constructor:
// quote-free strings ride RAW (no escaping - `\\` would silently become
// TWO backslashes in the no-escape parser) and numbers ride bare.
// Everything else (quote strings, vars, bools, nil, nested tables)
// defers to SetTableValue blocks.
fn lua_eq_atom(e: &Expr) -> Option<String> {
    match e {
        Expr::Str(s) if eq_str_ok(s) => Some(format!("\"{}\"", s)),
        Expr::Number(n) => Some(format!("{}", n)),
        _ => None,
    }
}

// [patch22] Same rule for a plain (Named) string key.
fn lua_eq_str(s: &str) -> Option<String> {
    eq_str_ok(s).then(|| format!("\"{}\"", s))
}

/// Returns true if `e` is safe to fold into a single SolveEquation:
/// it contains only side-effect-free constructs (Name, Number, Str,
/// Bool, Nil, Paren, Index, BinOp, UnOp). Returns false if `e`
/// contains a Call/Method (which may have side effects), a Table
/// literal (which needs separate CreateTable blocks), a Function
/// literal, or a Vararg.
///
/// Used by the BinOp path to decide whether to fold the whole BinOp
/// tree into ONE SolveEquation (saving N-1 blocks for N nested ops)
/// or to recurse via self.expr() and emit per-op blocks.
fn is_safe_for_eq_fold(e: &Expr) -> bool {
    match e {
        Expr::Name(_) | Expr::Number(_) | Expr::Str(_)
        | Expr::Bool(_) | Expr::Nil | Expr::Vararg => true,
        Expr::Paren(inner) => is_safe_for_eq_fold(inner),
        // [patched: eq-safety v6 - bracket ban] RS equations evaluate dot chains
        // (literal keys: Eq90/91 `(amB + (l.color.B * f))` ran every frame)
        // but nil out bracket-index chains at runtime: Solve Equation93
        // `squareColors[i2][j2]` and Eq98 `squares[i2][j2]` produced nil ->
        // the Eq94-96 "attempt to perform arithmetic (sub) on number and
        // nil" cascade, Eq97 "attempt to compare number < nil", and
        // "Set Object Property78: Object must be Instance got nil". The
        // SAME reads as blocks work (initializeGrid's GetTableValue chain
        // built the grid). Non-Str keys render as `t[expr]`, so they
        // disqualify the expression from equation folding; decomposition
        // paths emit GetTableValue chains instead.
        Expr::Index(b, k) => {
            // [patch18] Str keys must be identifier-shaped: the fold
            // renders `base.key` and RS equations reject non-identifier
            // members (T.0 / T.+ / T.( - run 6 charcode anchors). The
            // if-condition fold bypasses the v17 index-read guard, so
            // this gate is the only thing standing between
            // `if T["0"] == 48 and T["+"] == 43 then` and one broken
            // SolveEquation. Non-identifier keys now decompose to
            // GetTableValue + Comparison via the per-operand path.
            matches!(k.as_ref(), Expr::Str(s) if rs_eq_member_ok(s))
                && is_safe_for_eq_fold(b)
                && is_safe_for_eq_fold(k)
        }
        Expr::UnOp(_, inner) => is_safe_for_eq_fold(inner),
        // [patch19] `..` is BANNED inside equations: user test (run 7 E1)
        // proved RS SolveEquation rejects `..` entirely ("A" .. "B" does
        // not compile). Concat must always decompose to Concatenate
        // blocks (+ ToStringAny coercion per v17). Without this arm the
        // recursive BinOp arm below let a condition like
        // `if (s .. "x") == y then` fold into one broken equation.
        Expr::BinOp(BinOp::Concat, _, _) => false,
        Expr::BinOp(_, a, b) => is_safe_for_eq_fold(a) && is_safe_for_eq_fold(b),
        // Call / Method / Table / Function: NOT safe - they emit
        // their own blocks (synthesized calls, CreateTable, etc.).
        Expr::Call(_, _) | Expr::Method(_, _, _)
        | Expr::Table(_) | Expr::Function(_, _) => false,
    }
}

// [patch18] true when s can legally follow '.' in an RS equation (bare
// identifier: [A-Za-z_][A-Za-z0-9_]*). Module-level so the index-read
// path (v17 guard + index_chain_all_str_keys) and the fold-safety gates
// (v18) share one definition. Dot-property reads (a.b) always qualify -
// Lua syntax forbids non-identifier dot keys, so this only reroutes
// bracket reads like a["0"] / a["+"].
fn rs_eq_member_ok(s: &str) -> bool {
    let mut cs = s.chars();
    match cs.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    cs.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// RS equation compile-safety for compound conditions (v4 patch).
///
/// In-game evidence (imported raycaster payload, RetroStudio console):
/// - COMPILED + RAN: `((t2 >= 6) and (t2 < 8))`,
///   `(t77 ~= nil and t77 ~= false)`,
///   `(((dr > t) or (dg > t)) or (db > t))`,
///   `(not (inputProcessed) and (inputObj.KeyCode == "Q"))`,
///   `(screenScaleTextBox.Text or SCREEN_SCALE)` (property-read operand)
/// - "Error compiling equation: Couldn't figure this out!":
///   `light and light.Enabled`  (Solve Equation26)
///   `doInterlace and (((i2 + j2) % 2) ~= interlacePhase)`  (Eq36)
/// Rule: an `and`/`or` operand must NOT be a bare atom (Name / Number /
/// Bool / Nil). Comparisons, property/index reads and `not (...)` parse.
fn is_bare_eq_atom(e: &Expr) -> bool {
    matches!(e, Expr::Name(_) | Expr::Number(_) | Expr::Bool(_) | Expr::Nil)
}

fn is_eq_compilable_condition(e: &Expr) -> bool {
    match e {
        Expr::BinOp(op, a, b) if matches!(op, BinOp::And | BinOp::Or) => {
            !is_bare_eq_atom(a)
                && !is_bare_eq_atom(b)
                && is_eq_compilable_condition(a)
                && is_eq_compilable_condition(b)
        }
        Expr::BinOp(_, a, b) => {
            is_eq_compilable_condition(a) && is_eq_compilable_condition(b)
        }
        Expr::UnOp(_, inner) | Expr::Paren(inner) => is_eq_compilable_condition(inner),
        _ => true,
    }
}

/// [patched: eq-truthy-wrap v1] Rewrite a condition for the compound
/// equation fold: every bare Name directly under an and/or (possibly
/// behind parens or inside nested and-or chains) becomes the game-proven
/// truthiness comparison `(x ~= nil and x ~= false)` - RS's equation
/// parser rejects bare and/or operands ("Couldn't figure this out!":
/// Solve Equation26 `light and light.Enabled`, Eq36
/// `doInterlace and (((i2 + j2) % 2) ~= interlacePhase)`) but accepts
/// comparisons and parenthesized chains (Eq8 `((t2 >= 6) and (t2 < 8))`,
/// Eq23 `(t77 ~= nil and t77 ~= false)`, Eq45
/// `(((dr > t) or (dg > t)) or (db > t))`, Eq112
/// `(not (inputProcessed) and (inputObj.KeyCode == "Q"))`).
///
/// Names INSIDE comparisons are left alone: wrapping them would change
/// semantics (`(x ~= nil) > 0` is a type error). Returns None when the
/// condition cannot be made equation-safe (a Number/Bool/Nil/Vararg
/// directly under and/or - constant truthiness, kept out of equations).
fn eq_wrap_bare_names(e: &Expr) -> Option<Expr> {
    match e {
        Expr::BinOp(op @ (BinOp::And | BinOp::Or), a, b) => {
            let aw = eq_wrap_and_operand(a)?;
            let bw = eq_wrap_and_operand(b)?;
            Some(Expr::BinOp(*op, Box::new(aw), Box::new(bw)))
        }
        // Comparison / arithmetic subtree: recurse so NESTED and/or chains
        // still get wrapped, but leaf atoms stay untouched.
        Expr::BinOp(op, a, b) => Some(Expr::BinOp(
            *op,
            Box::new(eq_wrap_bare_names(a)?),
            Box::new(eq_wrap_bare_names(b)?),
        )),
        Expr::UnOp(op, inner) => {
            Some(Expr::UnOp(*op, Box::new(eq_wrap_bare_names(inner)?)))
        }
        Expr::Paren(inner) => eq_wrap_bare_names(inner),
        _ => Some(e.clone()),
    }
}

/// One and/or operand of eq_wrap_bare_names.
fn eq_wrap_and_operand(e: &Expr) -> Option<Expr> {
    let mut cur = e;
    while let Expr::Paren(inner) = cur {
        cur = inner;
    }
    match cur {
        // [patched: eq-safety v6 - index wrap] Dot-read operands need the same
        // treatment as bare Names: Solve Equation38
        // `(((light ~= nil) and (light ~= false)) and light.Enabled)`
        // still failed to compile ("Couldn't figure this out!") while the
        // all-comparison shape compiled and ran (v5-Eq36, Eq8/23/45, Eq112).
        // RS accepts comparison-shaped operands in both and/or positions
        // but rejects an unparenthesized dot-read on the RIGHT (Eq114's
        // prop-read is on the LEFT). Wrapping makes returns_bool true, so
        // the outer truthiness doubling disappears. Runtime-safe: RS
        // dot-reads are nil-defensive (cur2.R on a missing cell yielded
        // nil instead of erroring). Only Str-keyed indexes reach here
        // (is_safe_for_eq_fold gate bans computed keys).
        Expr::Name(_) | Expr::Index(_, _) => Some(eq_truthy_name(cur)),
        Expr::Number(_) | Expr::Bool(_) | Expr::Nil | Expr::Vararg => None,
        _ => eq_wrap_bare_names(cur),
    }
}

/// `(x ~= nil and x ~= false)` as an Expr - the exact Bool-gate text
/// Solve Equation23/67/68/99/101/116 ran in-game.
fn eq_truthy_name(e: &Expr) -> Expr {
    Expr::BinOp(
        BinOp::And,
        Box::new(Expr::BinOp(BinOp::Ne, Box::new(e.clone()), Box::new(Expr::Nil))),
        Box::new(Expr::BinOp(
            BinOp::Ne,
            Box::new(e.clone()),
            Box::new(Expr::Bool(false)),
        )),
    )
}

/// Returns true if `e` is guaranteed to evaluate to a Bool:
/// comparisons, Not, And/Or of Bool-returning operands.
fn returns_bool(e: &Expr) -> bool {
    match e {
        Expr::BinOp(op, a, b) => match op {
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le
            | BinOp::Gt | BinOp::Ge => true,
            BinOp::And | BinOp::Or => returns_bool(a) && returns_bool(b),
            _ => false,
        },
        Expr::UnOp(UnOp::Not, _) => true,
        _ => false,
    }
}

fn chain_walk(
    names: Vec<String>,
    index: &HashMap<String, usize>,
    blocks: &mut Vec<EmittedBlock>,
) -> Vec<String> {
    chain_walk_with(names, index, blocks, false)
}

/// [patched: else-chain v7 - chain_walk with a flag: when `body_opens_container` is
/// true, a BODY block that OPENS a run (no tail, no container - e.g. at the
/// start of an else branch) becomes that run's container instead of leaving
/// the continuation unparented. The else-branch linker needs this because
/// the else slot is a single ElseChildBlock: the run may not split into
/// multiple heads.
fn chain_walk_with(
    names: Vec<String>,
    index: &HashMap<String, usize>,
    blocks: &mut Vec<EmittedBlock>,
    body_opens_container: bool,
) -> Vec<String> {
    // v3 - SEQUENCE model (user ground truth: B5 hand-fix, checked against
    // Roblox Studio in-game).
    //
    // RS executes a block's ChildBlocks as a SEQUENCE (first to last), and a
    // failed WaitForChild aborts its whole subtree (Studio: infinite yield
    // stops the script). Therefore:
    // - LEAF blocks chain parent->child (their children are a pure
    //   continuation sequence).
    // - BODY blocks (If/While/For/LoopThrough) are SEQUENCE MEMBERS at their
    //   source position: the continuation after them continues as SIBLINGS in
    //   the enclosing sequence (never children - that would run on one branch
    //   only), and they stay inside the enclosing subtree so a failed
    //   dependency up the chain kills them too (Studio parity).
    // - INERT blocks (DefineFunction, event roots) are standalone: their
    //   children are bodies/handlers; nothing chains into them and they do
    //   not extend runs.
    const BODY: &[&str] = &[
        "If", "WhileLoop3", "ForLoop2", "LoopThroughChildren", "LoopThroughTable",
        // RUNTIME BUG FIX: event blocks inside function bodies must be
        // sequence members (BODY), not INERT. When `:Connect(...)` is
        // called inside a function, the event block needs to register
        // WHEN the function runs and reaches that point - not at script-
        // load. With BODY classification, the event block becomes a child
        // of the previous tail (the materialize block that set the player
        // value), so it registers AFTER the value is set. The next chain
        // element goes to the container (the materialize block), not the
        // event block, so the event block's handler body (its own children)
        // stays separate from the next statement.
        "PartTouched", "PartTouchEnded",
        "RemoteRecieveServer2", "RemoteRecieveClient2",
        "BindableRecieve2", "BindableFunctionOnInvoke",
        "PlayerAdded", "PlayerRemoving",
        "PropertyChanged", "AnyPropertyChanged", "GUILeftMouseButtonClick",
        "TextBoxFocusLost", "InputBegan", "InputChanged", "InputEnded",
        "Heartbeat", "RenderStepped", "CharacterAdded", "Chatted", "DescendantAdded",
    ];
    const INERT: &[&str] = &[
        // RUNTIME BUG FIX: event block types moved to BODY list above so
        // they chain as sequence members inside function bodies. Only
        // DefineFunction remains INERT (it's a true root - its body runs
        // only when called via ExecuteFunction).
        "DefineFunction",
    ];
    // kinds precomputed - no immutable borrow may span the mutation loop
    let kinds: std::collections::HashMap<String, u8> = names
        .iter()
        .map(|n| {
            let k = match index.get(n.as_str()) {
                Some(i) if INERT.contains(&blocks[*i].btype.as_str()) => 0u8,
                Some(i) if BODY.contains(&blocks[*i].btype.as_str()) => 2u8,
                Some(_) => 1u8,
                None => 0u8,
            };
            (n.clone(), k)
        })
        .collect();
    let mut heads: Vec<String> = Vec::new();
    let mut tail: Option<String> = None; // current leaf-chain tail
    let mut container: Option<String> = None; // sequence owner after a body-block
    for n in names {
        match kinds.get(&n).copied().unwrap_or(0) {
            0 => {
                // INERT: standalone member; ends the run entirely
                heads.push(n);
                tail = None;
                container = None;
            }
            2 => {
                // BODY: sequence member; the continuation moves to the
                // enclosing sequence (the tail leaf's children), never
                // into the body, never floating to the enclosing ROOT
                if let Some(t) = &tail {
                    if let Some(i) = index.get(t.as_str()) {
                        blocks[*i].children.push(n.clone());
                    }
                    container = Some(t.clone());
                } else if let Some(c) = &container {
                    if let Some(i) = index.get(c.as_str()) {
                        blocks[*i].children.push(n.clone());
                    }
                } else if body_opens_container {
                    // [patched: else-chain v7 - a BODY block opening the run becomes
                    // the container: the continuation after it joins the run
                    // as a sibling of the body block (sequence model) instead
                    // of splitting into a second head.
                    heads.push(n.clone());
                    container = Some(n.clone());
                } else {
                    heads.push(n);
                }
                tail = None;
            }
            _ => {
                // LEAF: extends the current run
                if let Some(t) = &tail {
                    if let Some(i) = index.get(t.as_str()) {
                        blocks[*i].children.push(n.clone());
                    }
                    tail = Some(n);
                } else if let Some(c) = &container {
                    if let Some(i) = index.get(c.as_str()) {
                        blocks[*i].children.push(n.clone());
                    }
                    tail = Some(n);
                } else {
                    heads.push(n.clone());
                    tail = Some(n);
                }
            }
        }
    }
    heads
}

    /// Renders an Expr as Luau source text for SolveEquation. Variable names
    /// resolve to their BINDING names - RS evaluates equations with variable
    /// resolution (ground truth: UDim2.new(0.5, w, 1, -14), w = SetVariable1).
    /// Numbers are DECIMAL here (equation text), unlike wire Number values.
    

fn is_bare_task_wait(e: &Expr) -> bool {
    if let Expr::Call(f, args) = e {
        if args.is_empty() {
            if let Expr::Index(base, key) = &**f {
                if let (Expr::Name(ns), Expr::Str(m)) = (&**base, &**key) {
                    return ns == "task" && m == "wait";
                }
            }
        }
    }
    false
}

fn eq_op_str(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Mod => "%",
        BinOp::Pow => "^",
        _ => "?",
    }
}

/// RetroStudio property-name aliases: modern Luau names -> the legacy names
/// RetroStudio's block database expects (ground truth: TextSize -> FontSize).
fn property_alias(p: &str) -> &str {
    match p {
        "TextSize" => "FontSize",
        other => other,
    }
}

fn sanitize(lua: &str) -> String {
    let mut s: String = lua
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' })
        .collect();
    if s.is_empty() || s.chars().next().unwrap().is_ascii_digit() {
        s = format!("v{}", s);
    }
    s
}

/// Schema types where variable references (mode 2) are legal.
/// Fixed single types take mode 0 with the binding name
/// (ground truth: SetObjectProperty.Object = "frame").
fn var_allowed_on(t: &str) -> bool {
    // Mode 2 (variable reference) is ONLY legal on schema "Any" inputs.
    // Everything else - Function, Object?, Table, ... - is referenced BY
    // NAME via mode 0. RS ground truth: "Function 0 onClick",
    // "Parent 0 nil", "GUIButton 0 btn", "Object 0 frame". Mode 2 on any
    // non-Any input crashes RetroStudio's block-properties UI
    // (PropertiesUiModule: attempt to compare number < nil).
    // "Tuple" stays mode 2: tuple inputs bind a variable (the TUPLE_ output);
    // mode 0 there packs a literal "0" (verified broken). No direct RS
    // evidence yet - A/B when remote events get tested in RetroStudio.
    matches!(t, "Any" | "Tuple")
}