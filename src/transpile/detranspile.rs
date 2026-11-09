//! VS -> Luau detranspiler: walks a decoded block forest and reconstructs
//! Luau source.
//!
//! Inverts [`crate::transpile::emit`]. The goal is *semantic reconstruction*,
//! not byte-identical source: `detranspile(decode(transpile(x)))` must
//! retranspile to a block tree equivalent to the original. Block-for-block
//! identity is impossible by design (the emitter folds constants, lowers
//! `and`/`or` to Comparison chains, restructures early-return guards), so the
//! round-trip contract is checked on block-type multisets, not bytes.
//!
//! Model recap (mirrors emit.rs `chain_walk` docs):
//! - a block's `children` is an ORDERED SEQUENCE that runs after it;
//! - pure expression blocks chain into their consuming statement (the chain
//!   ends at the statement that reads their output binding);
//! - control-flow blocks (If/While/For/LoopThrough/events) own their body in
//!   `children`, and the continuation after them is a SIBLING in the
//!   enclosing sequence;
//! - `else_child` holds the else-branch head;
//! - every input is a literal or a binding reference; bindings produced by
//!   expression blocks are reconstructed inline, bindings produced by
//!   SetVariable1/CreateTable/DefineFunction are variable names.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use crate::decoder::{fmt_num, DecodedBlock, DecodedInput, DecodedReport};
use crate::transpile::emit::effectful;

pub struct DetranspileOutput {
    /// Reconstructed Luau source.
    pub luau: String,
    /// (kind, message) notes, kind "GAP" for unsupported blocks.
    pub notes: Vec<(String, String)>,
}

/// Block types whose `children` is the handler/function BODY (from
/// emit.rs `chain_walk` BODY/INERT lists).
const BODY_BLOCKS: &[&str] = &[
    "If",
    "WhileLoop3",
    "ForLoop2",
    "LoopThroughChildren",
    "LoopThroughTable",
    "PartTouched",
    "PartTouchEnded",
    "RemoteRecieveServer2",
    "RemoteRecieveClient2",
    "BindableRecieve2",
    "BindableFunctionOnInvoke",
    "PlayerAdded",
    "PlayerRemoving",
    "PropertyChanged",
    "AnyPropertyChanged",
    "GUILeftMouseButtonClick",
    "TextBoxFocusLost",
    "InputBegan",
    "InputChanged",
    "InputEnded",
    "Heartbeat",
    "RenderStepped",
    "CharacterAdded",
    "Chatted",
    "DescendantAdded",
];

/// Statement block types that ARE handled; anything outside PURE + this set
/// gets a GAP comment (and its children still walk, so the rest reconstructs).
const HANDLED_STATEMENTS: &[&str] = &[
    "Comment",
    "Comment2",
    "SetVariable1",
    "SetTableValue",
    "TableAppend",
    "TableRemove",
    "SetObjectProperty",
    "CreateObject",
    "CloneObject",
    "DestroyObject",
    "ExecuteFunction",
    "Spawn",
    "Print",
    "Warn",
    "Wait",
    "TakeDamage",
    "PlaySound",
    "StopSound",
    "FunctionReturn",
    "BreakLoop",
    "ContinueLoop",
    "RemoteFireServer2",
    "RemoteFireAllClients2",
    "RemoteFireClient2",
    "BindableFire2",
];

struct Detranspiler<'a> {
    report: &'a DecodedReport,
    /// binding -> owning block name
    owner: HashMap<String, String>,
    /// scopes of declared variable names (script level first)
    scopes: Vec<HashSet<String>>,
    /// notes are pushed from &self contexts (expression reconstruction)
    notes: std::cell::RefCell<Vec<(String, String)>>,
    out: String,
    indent: usize,
    depth: usize,
}

/// Entry point: reconstruct Luau from a decoded report.
pub fn detranspile(report: &DecodedReport) -> DetranspileOutput {
    let mut owner = HashMap::new();
    for b in report.blocks.values() {
        for (_, binding) in &b.outputs {
            owner.insert(binding.clone(), b.name.clone());
        }
    }
    let mut d = Detranspiler {
        report,
        owner,
        scopes: vec![HashSet::new()],
        notes: std::cell::RefCell::new(Vec::new()),
        out: String::new(),
        indent: 0,
        depth: 0,
    };
    // roots: blocks nothing points at as child/else_child
    let mut is_child: HashSet<&String> = HashSet::new();
    for b in report.blocks.values() {
        for c in &b.children {
            is_child.insert(c);
        }
        if let Some(ec) = &b.else_child {
            is_child.insert(ec);
        }
    }
    let roots: Vec<String> = report
        .block_order
        .iter()
        .filter(|n| !is_child.contains(n))
        .cloned()
        .collect();
    d.walk_sequence(&roots);
    DetranspileOutput {
        luau: d.out,
        notes: d.notes.into_inner(),
    }
}

impl<'a> Detranspiler<'a> {
    fn block(&self, name: &str) -> Option<&'a DecodedBlock> {
        self.report.blocks.get(name)
    }

    fn note(&self, kind: &str, msg: String) {
        self.notes.borrow_mut().push((kind.to_string(), msg));
    }

    fn line(&mut self, text: &str) {
        for _ in 0..self.indent {
            self.out.push_str("    ");
        }
        let _ = writeln!(self.out, "{}", text);
    }

    fn inp(&self, b: &'a DecodedBlock, name: &str) -> Option<&'a DecodedInput> {
        b.inputs.iter().find(|i| i.iname == name)
    }

    /// TUPLEPARAM_* inputs sorted by trailing index -> exprs.
    fn tuple_vals(&self, b: &'a DecodedBlock, prefix: &str) -> Vec<String> {
        let mut tagged: Vec<(u32, &DecodedInput)> = Vec::new();
        for i in &b.inputs {
            if let Some(rest) = i.iname.strip_prefix(prefix) {
                if let Ok(n) = rest.parse::<u32>() {
                    tagged.push((n, i));
                }
            }
        }
        tagged.sort_by_key(|(n, _)| *n);
        tagged.into_iter().map(|(_, i)| self.expr_input(i)).collect()
    }

    // ----- expressions -----

    fn expr_input(&self, i: &DecodedInput) -> String {
        if i.use_var {
            return self.expr_binding(&i.var);
        }
        let v = i.value.as_deref().unwrap_or("");
        match i.vtype.as_deref().unwrap_or("") {
            "Number" => fmt_num(Some(v)),
            "String" => quote(v),
            "Bool" => bool_str(v),
            "Nil" => "nil".to_string(),
            "Object" => v.to_string(),
            "Vector3" => {
                let c = comps(v, 3);
                format!("Vector3.new({}, {}, {})", c[0], c[1], c[2])
            }
            "Vector2" => {
                let c = comps(v, 2);
                format!("Vector2.new({}, {})", c[0], c[1])
            }
            "UDim2" => {
                let c = comps(v, 4);
                format!("UDim2.new({}, {}, {}, {})", c[0], c[1], c[2], c[3])
            }
            "Color3" => {
                let c: Vec<String> = v
                    .split(',')
                    .map(|p| i64::from_str_radix(p.trim(), 16).map(|n| n.to_string()).unwrap_or_else(|_| "0".into()))
                    .collect();
                format!("Color3.fromRGB({}, {}, {})", c.first().unwrap_or(&"0".into()), c.get(1).unwrap_or(&"0".into()), c.get(2).unwrap_or(&"0".into()))
            }
            "BrickColor" => format!("BrickColor.new({})", fmt_num(Some(v))),
            "" => {
                // untyped: try hex number (with optional fraction), else string
                if v.is_empty() {
                    "nil".to_string()
                } else if looks_hex_num(v) {
                    fmt_num(Some(v))
                } else {
                    quote(v)
                }
            }
            other => {
                // unknown explicit type: fall back to a raw string
                let _ = other;
                quote(v)
            }
        }
    }

    fn expr_binding(&self, binding: &str) -> String {
        let Some(owner_name) = self.owner.get(binding) else {
            return binding.to_string();
        };
        let Some(b) = self.block(owner_name) else {
            return binding.to_string();
        };
        match b.block_type.as_str() {
            // variables/tables/functions/loop counters: the binding IS the
            // Luau name
            "SetVariable1" | "CreateTable" | "DefineFunction" | "ForLoop2"
            | "LoopThroughTable" | "LoopThroughChildren" => binding.to_string(),
            _ => {
                // var-assigned bindings were emitted as an assignment in the
                // sequence walk; temps are inlined as expressions
                if !is_temp(binding) {
                    return binding.to_string();
                }
                self.expr_of_block(b)
            }
        }
    }

    /// Reconstruct the expression a block computes (used for temp bindings
    /// and for pure blocks whose output lands in a variable).
    fn expr_of_block(&self, b: &'a DecodedBlock) -> String {
        let binding = b
            .outputs
            .first()
            .map(|(_, s)| s.clone())
            .unwrap_or_default();
        match b.block_type.as_str() {
            // variables/tables/functions/loop counters: the binding IS the
            // Luau name
            "SetVariable1" | "CreateTable" | "DefineFunction" | "ForLoop2"
            | "LoopThroughTable" | "LoopThroughChildren" => binding.to_string(),
            "BindableFunctionInvoke" => {
                let f = self.obj_expr(b, "BindableFunction");
                let args = self.tuple_vals(b, "TUPLEPARAM_Parameters_");
                if args.is_empty() {
                    format!("{}:Invoke()", f)
                } else {
                    format!("{}:Invoke({})", f, args.join(", "))
                }
            }
            "SolveEquation" => self
                .inp(b, "Equation")
                .map(|i| i.value.clone().unwrap_or_default())
                .unwrap_or_else(|| binding.to_string()),
            "Addition" => self.binop(b, "Number1", "Number2", "+"),
            "Subtraction" => self.binop(b, "Number1", "Number2", "-"),
            "Multiplication" => self.binop(b, "Number1", "Number2", "*"),
            "Division" => self.binop(b, "Number1", "Number2", "/"),
            "Modulus" => self.binop(b, "Number1", "Number2", "%"),
            "Exponentiation" => self.binop(b, "Number1", "Number2", "^"),
            "Comparison" => self.comparison(b),
            "ANDAny" => self.binop(b, "A", "B", "and"),
            "ORAny" => self.binop(b, "A", "B", "or"),
            "NOTAny" => format!("not ({})", self.arg(b, "A")),
            "Concatenate" => self.binop(b, "FirstString", "SecondString", ".."),
            "GetObjectProperty" => {
                let obj = self.obj_expr(b, "Object");
                let prop = self.raw(b, "Property");
                prop_path(&obj, &prop)
            }
            "GetTableValue" => {
                let t = self.obj_expr(b, "Table");
                let k = self.arg(b, "Key");
                format!("{}[{}]", t, k)
            }
            "GetTableLength" | "GetLength" => {
                let what = if b.block_type == "GetLength" { "String" } else { "Table" };
                format!("#{}", self.obj_expr(b, what))
            }
            "ToStringAny" => format!("tostring({})", self.arg(b, "Input")),
            "ToNumberAny" => format!("tonumber({})", self.arg(b, "Input")),
            "TypeOf" => format!("typeof({})", self.arg(b, "Variable")),
            "WaitForChild" => {
                let p = self.obj_expr(b, "Parent");
                let n = self.arg(b, "ChildName");
                let t = self.arg(b, "TimeOut");
                format!("{}:WaitForChild({}, {})", p, n, t)
            }
            "FindFirstChildNew" => {
                let p = self.obj_expr(b, "Parent");
                let n = self.arg(b, "ChildName");
                let rec = self
                    .inp(b, "Recursive")
                    .map(|i| bool_str(i.value.as_deref().unwrap_or("0")))
                    .unwrap_or_else(|| "false".into());
                if rec == "true" {
                    format!("{}:FindFirstChild({}, true)", p, n)
                } else {
                    format!("{}:FindFirstChild({})", p, n)
                }
            }
            "FindFirstChildOfClassNew" => {
                let p = self.obj_expr(b, "Parent");
                let c = self.arg(b, "ChildClass");
                format!("{}:FindFirstChildOfClass({})", p, c)
            }
            "FindFirstChildWhichIsANew" => {
                let p = self.obj_expr(b, "Parent");
                let c = self.arg(b, "ChildClass");
                format!("{}:FindFirstChildWhichIsA({})", p, c)
            }
            "GetChildren" => format!("{}:GetChildren()", self.obj_expr(b, "Parent")),
            "GetDescendants" => format!("{}:GetDescendants()", self.obj_expr(b, "Parent")),
            "IsA" => format!(
                "{}:IsA({})",
                self.obj_expr(b, "Object"),
                self.arg(b, "Class")
            ),
            "GetPlayerFromCharacter" => format!(
                "game.Players:GetPlayerFromCharacter({})",
                self.arg(b, "Character")
            ),
            "ConstructVector3" => format!(
                "Vector3.new({}, {}, {})",
                self.arg(b, "X"),
                self.arg(b, "Y"),
                self.arg(b, "Z")
            ),
            "ConstructVector2" => {
                format!("Vector2.new({}, {})", self.arg(b, "X"), self.arg(b, "Y"))
            }
            "ConstructUDim2" => format!(
                "UDim2.new({}, {}, {}, {})",
                self.arg(b, "XScale"),
                self.arg(b, "XOffset"),
                self.arg(b, "YScale"),
                self.arg(b, "YOffset")
            ),
            "ConstructColor3" => format!(
                "Color3.fromRGB({}, {}, {})",
                self.arg(b, "R"),
                self.arg(b, "G"),
                self.arg(b, "B")
            ),
            "ConstructCFrame" => format!("CFrame.new({})", self.arg(b, "Position")),
            "Angles" => format!(
                "CFrame.Angles({}, {}, {})",
                self.arg(b, "rx"),
                self.arg(b, "ry"),
                self.arg(b, "rz")
            ),
            "DistanceBetweenPoints" => format!(
                "({} - {}).Magnitude",
                self.arg(b, "Point 1"),
                self.arg(b, "Point 2")
            ),
            "MathLibrarySingleInput" => {
                let f = self.raw(b, "Function");
                format!("{}({})", math_fn(&f), self.arg(b, "Number"))
            }
            "MathLibraryDoubleInput" => {
                let f = self.raw(b, "Function");
                format!(
                    "{}({}, {})",
                    math_fn(&f),
                    self.arg(b, "Number1"),
                    self.arg(b, "Number2")
                )
            }
            "RandomNumber" => format!(
                "math.random({}, {})",
                self.arg(b, "Minimum"),
                self.arg(b, "Maximum")
            ),
            "Round" => format!("math.round({})", self.arg(b, "Number")),
            "OsTime" => "os.time()".to_string(),
            "OsClock" => "os.clock()".to_string(),
            "GetServerTimeNow" => "GetServerTimeNow()".to_string(),
            "ExecuteFunction" => self.call_expr(b),
            "TablePack" => format!("table.pack({})", self.tuple_vals(b, "TUPLEPARAM_Values_").join(", ")),
            "TableUnpack" => format!("table.unpack({})", self.obj_expr(b, "Table")),
            "TableFind" => {
                let t = self.obj_expr(b, "Table");
                let v = self.arg(b, "Value");
                match self.inp(b, "Init") {
                    Some(init) => format!("table.find({}, {}, {})", t, v, self.expr_input(init)),
                    None => format!("table.find({}, {})", t, v),
                }
            }
            "FindString" => format!(
                "string.find({}, {})",
                self.arg(b, "String"),
                self.arg(b, "Pattern")
            ),
            "Lower" => format!("{}:lower()", self.arg(b, "String")),
            "Upper" => format!("{}:upper()", self.arg(b, "String")),
            "Substring" => format!(
                "string.sub({}, {}, {})",
                self.arg(b, "String"),
                self.arg(b, "StartPoint"),
                self.arg(b, "EndPoint")
            ),
            "GSubString" => format!(
                "string.gsub({}, {}, {})",
                self.arg(b, "String"),
                self.arg(b, "Pattern"),
                self.arg(b, "Replacement")
            ),
            "Lerp1" => format!(
                "{}:Lerp({})",
                self.arg(b, "CFrame1"),
                self.arg(b, "CFrame2")
            ),
            "InverseCFrame1" => format!("{}:Inverse()", self.arg(b, "CFrame")),
            "ToObjectSpace1" => format!("{}:ToObjectSpace()", self.arg(b, "CFrame")),
            "ToWorldSpace1" => format!("{}:ToWorldSpace()", self.arg(b, "CFrame")),
            "CloneObject" => format!("{}:Clone()", self.obj_expr(b, "Object")),
            "CreateObject" => {
                let cls = self.arg(b, "ClassName");
                let parent = self.obj_expr(b, "Parent");
                if parent.is_empty() || parent == "nil" {
                    format!("Instance.new({})", cls)
                } else {
                    format!("Instance.new({}, {})", cls, parent)
                }
            }
            "FindPartOnRay" => format!(
                "workspace:Raycast({}, {})",
                self.arg(b, "Origin"),
                self.arg(b, "Direction")
            ),
            "Vector3Dot" => format!(
                "{}:Dot({})",
                self.arg(b, "VectorA"),
                self.arg(b, "VectorB")
            ),
            "Vector3Cross" => format!(
                "{}:Cross({})",
                self.arg(b, "VectorA"),
                self.arg(b, "VectorB")
            ),
            "IsServer" => "game.RunService:IsServer()".to_string(),
            "SplitVector3" => {
                let v = self.arg(b, "Vector3");
                format!("Vector3.new({}.X, {}.Y, {}.Z)", v, v, v)
            }
            "SplitVector2" => {
                let v = self.arg(b, "Vector2");
                format!("Vector2.new({}.X, {}.Y)", v, v)
            }
            "ToEulerAnglesXYZ" => format!("{}:ToEulerAnglesXYZ()", self.arg(b, "CFrame")),
            "SplitString" => format!(
                "string.split({}, {})",
                self.arg(b, "String"),
                self.arg(b, "Seperator")
            ),
            "GetModelBoundingBox" => {
                format!("{}:GetBoundingBox()", self.obj_expr(b, "Model"))
            }
            // unknown pure block: treat its name as an opaque var reference
            // so surrounding code still reconstructs
            other => {
                self.note(
                    "GAP",
                    format!("expression block {:?} reconstructed as a placeholder reference", other),
                );
                binding.to_string()
            }
        }
    }

    /// Literal input (never a var ref): used for Object/Function name slots.
    fn raw(&self, b: &'a DecodedBlock, name: &str) -> String {
        self.inp(b, name)
            .map(|i| i.value.clone().unwrap_or_default())
            .unwrap_or_default()
    }

    fn arg(&self, b: &'a DecodedBlock, name: &str) -> String {
        match self.inp(b, name) {
            Some(i) => self.expr_input(i),
            None => "nil".to_string(),
        }
    }

    /// Object/Parent/Table slot: var binding or Object-typed string naming a
    /// variable/global, or a dotted path like "script.Parent.Remotes.X".
    fn obj_expr(&self, b: &'a DecodedBlock, name: &str) -> String {
        match self.inp(b, name) {
            Some(i) => {
                if i.use_var {
                    self.expr_binding(&i.var)
                } else {
                    let v = i.value.clone().unwrap_or_default();
                    match i.vtype.as_deref() {
                        Some("String") => {
                            // emitter passes var names / Luau paths as strings
                            v
                        }
                        _ => v,
                    }
                }
            }
            None => "nil".to_string(),
        }
    }

    fn binop(&self, b: &'a DecodedBlock, l: &str, r: &str, op: &str) -> String {
        format!("({} {} {})", self.arg(b, l), op, self.arg(b, r))
    }

    fn comparison(&self, b: &'a DecodedBlock) -> String {
        let l = self.arg(b, "Value 1");
        let r = self.arg(b, "Value 2");
        let op = self.raw(b, "ComparisonType");
        format!("({} {} {})", l, op, r)
    }

    fn call_expr(&self, b: &'a DecodedBlock) -> String {
        let f = self.obj_expr(b, "Function");
        let args = self.tuple_vals(b, "TUPLEPARAM_Parameters_");
        if args.is_empty() {
            format!("{}()", f)
        } else {
            format!("{}({})", f, args.join(", "))
        }
    }

    // ----- conditions -----

    /// If/While inputs: Value 1 [op Value 2]. The emitter's direct_if_inputs
    /// lowers plain conditions to `(cond == true)`.
    fn condition(&self, b: &'a DecodedBlock) -> String {
        let Some(v1) = self.inp(b, "Value 1") else {
            return "true".to_string();
        };
        let left = self.expr_input(v1);
        let (Some(op_i), Some(v2_i)) = (self.inp(b, "ComparisonType"), self.inp(b, "Value 2"))
        else {
            return left;
        };
        let op = op_i.value.as_deref().unwrap_or("==");
        // `x == true` -> x;  `x == false` -> not x
        if op == "==" {
            if v2_i.vtype.as_deref() == Some("Bool") {
                match v2_i.value.as_deref().unwrap_or("0") {
                    "1" => return left,
                    "0" => return format!("not ({})", left),
                    _ => {}
                }
            }
            if v2_i.vtype.as_deref() == Some("Nil") {
                return format!("({} == nil)", left);
            }
        }
        format!("({} {} {})", left, op, self.expr_input(v2_i))
    }

    // ----- statements -----

    fn declare_name(&self, name: &str) -> bool {
        // true if already declared in ANY enclosing scope
        self.scopes.iter().any(|s| s.contains(name))
    }

    /// Emits `local x = expr` / `x = expr` for each non-tuple output binding.
    fn assign_expr_outputs(&mut self, b: &'a DecodedBlock, expr: &str) {
        let var_outs: Vec<String> = b
            .outputs
            .iter()
            .filter(|(n, bnd)| {
                !bnd.is_empty() && !n.starts_with("TUPLE_ReturnedValues") && !is_temp(bnd)
            })
            .map(|(_, bnd)| bnd.clone())
            .collect();
        for var in &var_outs {
            if self.declare_name(var) {
                self.line(&format!("{} = {}", var, expr));
            } else {
                self.line(&format!("local {} = {}", var, expr));
                if let Some(s) = self.scopes.last_mut() {
                    s.insert(var.clone());
                }
            }
        }
        if var_outs.is_empty() {
            self.line(expr);
        }
    }

    fn bind_output_name(&self, b: &'a DecodedBlock, out_name: &str) -> Option<String> {
        b.outputs
            .iter()
            .find(|(n, _)| n == out_name)
            .map(|(_, binding)| binding.clone())
    }

    fn event_spec(&self, b: &'a DecodedBlock) -> (String, String) {
        // (dotted object expression, event name)
        let obj = |s: &str| self.obj_expr(b, s);
        match b.block_type.as_str() {
            "PartTouched" => (obj("Part"), "Touched".into()),
            "PartTouchEnded" => (obj("Part"), "TouchEnded".into()),
            "PlayerAdded" => ("game.Players".into(), "PlayerAdded".into()),
            "PlayerRemoving" => ("game.Players".into(), "PlayerRemoving".into()),
            "CharacterAdded" => (obj("Player"), "CharacterAdded".into()),
            "Chatted" => (obj("Player"), "Chatted".into()),
            "DescendantAdded" => (obj("Object"), "DescendantAdded".into()),
            "AnyPropertyChanged" => (obj("Object"), "Changed".into()),
            "GUILeftMouseButtonClick" => (obj("Object"), "MouseButton1Click".into()),
            "TextBoxFocusLost" => (obj("Object"), "FocusLost".into()),
            "InputBegan" => ("game.UserInputService".into(), "InputBegan".into()),
            "InputChanged" => ("game.UserInputService".into(), "InputChanged".into()),
            "InputEnded" => ("game.UserInputService".into(), "InputEnded".into()),
            "Heartbeat" => ("game.RunService".into(), "Heartbeat".into()),
            "RenderStepped" => ("game.RunService".into(), "RenderStepped".into()),
            "RemoteRecieveServer2" => (obj("RemoteEvent"), "OnServerEvent".into()),
            "RemoteRecieveClient2" => (obj("RemoteEvent"), "OnClientEvent".into()),
            "BindableRecieve2" => (obj("BindableEvent"), "Event".into()),
            "BindableFunctionOnInvoke" => (obj("BindableFunction"), "OnInvoke".into()),
            "PropertyChanged" => (obj("Object"), "PropertyChanged".into()),
            other => (obj("Object"), format!("{}", other)),
        }
    }

    fn event_params(&self, b: &'a DecodedBlock) -> Vec<String> {
        let mut params: Vec<String> = Vec::new();
        match b.block_type.as_str() {
            "RemoteRecieveServer2" => {
                if let Some(p) = self.bind_output_name(b, "Player") {
                    params.push(p);
                }
                params.extend(self.output_tuple(b, "TUPLEPARAM_Parameters_"));
            }
            "RemoteRecieveClient2" | "BindableRecieve2" | "BindableFunctionOnInvoke" => {
                params.extend(self.output_tuple(b, "TUPLEPARAM_Parameters_"));
            }
            _ => {
                // single-out event blocks: any non-TUPLE output is a param
                for (n, binding) in &b.outputs {
                    if n != "TUPLE_Parameters" && !binding.starts_with("TUPLE") {
                        params.push(binding.clone());
                    }
                }
            }
        }
        params
    }

    /// TUPLEPARAM_* OUTPUT bindings sorted by index.
    fn output_tuple(&self, b: &'a DecodedBlock, prefix: &str) -> Vec<String> {
        let mut tagged: Vec<(u32, String)> = Vec::new();
        for (n, binding) in &b.outputs {
            if let Some(rest) = n.strip_prefix(prefix) {
                if let Ok(idx) = rest.parse::<u32>() {
                    tagged.push((idx, binding.clone()));
                }
            }
        }
        tagged.sort_by_key(|(i, _)| *i);
        tagged.into_iter().map(|(_, s)| s).collect()
    }

    fn walk_sequence(&mut self, names: &[String]) {
        self.walk_seq(names, false);
    }

    /// `count_depth` is true only when descending into a BODY (if/loop/
    /// function/event); leaf continuations chain linearly and must not
    /// grow the nesting depth.
    fn walk_seq(&mut self, names: &[String], count_depth: bool) {
        if count_depth {
            self.depth += 1;
            if self.depth > 96 {
                self.note("GAP", "sequence nesting exceeded 96 levels; stopped".into());
                self.depth -= 1;
                return;
            }
        }
        for name in names {
            let Some(b) = self.block(name) else { continue };
            let bt = b.block_type.as_str();
            if bt == "Comment2" {
                let text = self.raw(b, "Comment");
                if text != "ROOT" {
                    let escaped = text.replace('\n', " ");
                    self.line(&format!("-- {}", escaped));
                }
                let kids = b.children.clone();
                self.walk_sequence(&kids);
                continue;
            }
            if bt == "Comment" {
                let text = self.raw(b, "Comment");
                let escaped = text.replace('\n', " ");
                self.line(&format!("-- {}", escaped));
                let kids = b.children.clone();
                self.walk_sequence(&kids);
                continue;
            }
            // pure expression block: outputs bound to a variable name are
            // ASSIGNMENTS (`i = i + 1` emits Addition with Result -> $i);
            // temp outputs (t<N>) are intermediates of the consuming
            // statement's chain and are reconstructed inline there.
            if !effectful(bt) {
                let var_outs: Vec<String> = b
                    .outputs
                    .iter()
                    .filter(|(_, bnd)| !bnd.is_empty() && !is_temp(bnd))
                    .map(|(_, bnd)| bnd.clone())
                    .collect();
                if !var_outs.is_empty() {
                    let val = self.expr_of_block(b);
                    for var in &var_outs {
                        if self.declare_name(var) {
                            self.line(&format!("{} = {}", var, val));
                        } else {
                            self.line(&format!("local {} = {}", var, val));
                            if let Some(s) = self.scopes.last_mut() {
                                s.insert(var.clone());
                            }
                        }
                    }
                }
                let kids = b.children.clone();
                self.walk_seq(&kids, true);
                continue;
            }
            match bt {
                "If" => {
                    let cond = self.condition(b);
                    let kids = b.children.clone();
                    let els = b.else_child.clone();
                    self.line(&format!("if {} then", cond));
                    self.indent += 1;
                    self.scopes.push(HashSet::new());
                    self.walk_seq(&kids, true);
                    self.scopes.pop();
                    self.indent -= 1;
                    if let Some(first) = els {
                        self.line("else");
                        self.indent += 1;
                        self.scopes.push(HashSet::new());
                        self.walk_seq(&[first], true);
                        self.scopes.pop();
                        self.indent -= 1;
                    }
                    self.line("end");
                }
                "WhileLoop3" => {
                    let cond = self.condition(b);
                    let kids = b.children.clone();
                    self.line(&format!("while {} do", cond));
                    self.indent += 1;
                    self.scopes.push(HashSet::new());
                    self.walk_seq(&kids, true);
                    self.scopes.pop();
                    self.indent -= 1;
                    self.line("end");
                }
                "ForLoop2" => {
                    let init = self.arg(b, "Initial");
                    let max = self.arg(b, "Maximum");
                    let inc = self.inp(b, "Increment").map(|i| self.expr_input(i));
                    let var = self
                        .bind_output_name(b, "Place")
                        .unwrap_or_else(|| "_i".to_string());
                    let kids = b.children.clone();
                    let header = match inc {
                        Some(i) if i != "1" => format!("for {} = {}, {}, {} do", var, init, max, i),
                        _ => format!("for {} = {}, {} do", var, init, max),
                    };
                    self.line(&header);
                    self.indent += 1;
                    self.scopes.push(HashSet::from([var.clone()]));
                    self.walk_seq(&kids, true);
                    self.scopes.pop();
                    self.indent -= 1;
                    self.line("end");
                }
                "LoopThroughTable" => {
                    let t = self.obj_expr(b, "Table");
                    let val = self
                        .bind_output_name(b, "Value")
                        .unwrap_or_else(|| "_v".to_string());
                    let kids = b.children.clone();
                    self.line(&format!("for _, {} in pairs({}) do", val, t));
                    self.indent += 1;
                    self.scopes.push(HashSet::from([val.clone()]));
                    self.walk_seq(&kids, true);
                    self.scopes.pop();
                    self.indent -= 1;
                    self.line("end");
                }
                "LoopThroughChildren" => {
                    let o = self.obj_expr(b, "Object");
                    let child = self
                        .bind_output_name(b, "Child")
                        .unwrap_or_else(|| "_c".to_string());
                    let kids = b.children.clone();
                    self.line(&format!("for _, {} in {}:GetChildren() do", child, o));
                    self.indent += 1;
                    self.scopes.push(HashSet::from([child.clone()]));
                    self.walk_seq(&kids, true);
                    self.scopes.pop();
                    self.indent -= 1;
                    self.line("end");
                }
                "DefineFunction" => {
                    let fname = self
                        .bind_output_name(b, "Function")
                        .unwrap_or_else(|| "_fn".to_string());
                    let params = self.output_tuple(b, "TUPLEPARAM_Parameters_");
                    let kids = b.children.clone();
                    if params.is_empty() {
                        self.line(&format!("local function {}()", fname));
                    } else {
                        self.line(&format!("local function {}({})", fname, params.join(", ")));
                    }
                    self.indent += 1;
                    let mut sc = HashSet::new();
                    sc.insert(fname.clone());
                    for p in &params {
                        sc.insert(p.clone());
                    }
                    self.scopes.push(sc);
                    self.walk_seq(&kids, true);
                    self.scopes.pop();
                    self.indent -= 1;
                    self.line("end");
                }
                "FunctionReturn" => {
                    let vals = self.tuple_vals(b, "TUPLEPARAM_Returns_");
                    if vals.is_empty() {
                        self.line("return");
                    } else {
                        self.line(&format!("return {}", vals.join(", ")));
                    }
                }
                "BreakLoop" => self.line("break"),
                "ContinueLoop" => self.line("continue"),
                "SetVariable1" => {
                    let Some(var) = self.bind_output_name(b, "VariableName") else {
                        continue;
                    };
                    let val = self.arg(b, "Value");
                    if self.declare_name(&var) {
                        self.line(&format!("{} = {}", var, val));
                    } else {
                        self.line(&format!("local {} = {}", var, val));
                        if let Some(s) = self.scopes.last_mut() {
                            s.insert(var);
                        }
                    }
                }
                "SetTableValue" => {
                    let t = self.obj_expr(b, "Table");
                    let k = self.arg(b, "Key");
                    let v = self.arg(b, "Value");
                    self.line(&format!("{}[{}] = {}", t, k, v));
                }
                "TableAppend" => {
                    let t = self.obj_expr(b, "Table");
                    let v = self.arg(b, "Value");
                    self.line(&format!("table.insert({}, {})", t, v));
                }
                "TableRemove" => {
                    let t = self.obj_expr(b, "Table");
                    let p = self.arg(b, "Position");
                    self.line(&format!("table.remove({}, {})", t, p));
                }
                "TableInsert" => {
                    let t = self.obj_expr(b, "Table");
                    let v = self.arg(b, "Value");
                    match self.inp(b, "Position") {
                        Some(p) => {
                            let pe = self.expr_input(p);
                            if pe == "nil" {
                                self.line(&format!("table.insert({}, {})", t, v));
                            } else {
                                self.line(&format!("table.insert({}, {}, {})", t, pe, v));
                            }
                        }
                        None => self.line(&format!("table.insert({}, {})", t, v)),
                    }
                }
                "SetObjectProperty" => {
                    let o = self.obj_expr(b, "Object");
                    let prop = self.raw(b, "Property");
                    let v = self.arg(b, "Value");
                    self.line(&format!("{}{} = {}", o, prop_path("", &prop), v));
                }                "CreateObject" => {
                    let cls = self.arg(b, "ClassName");
                    let parent = self.obj_expr(b, "Parent");
                    let var = self.bind_output_name(b, "Object");
                    let call = if parent.is_empty() || parent == "nil" {
                        format!("Instance.new({})", cls)
                    } else {
                        format!("Instance.new({}, {})", cls, parent)
                    };
                    match var {
                        Some(v) => {
                            if self.declare_name(&v) {
                                self.line(&format!("{} = {}", v, call));
                            } else {
                                self.line(&format!("local {} = {}", v, call));
                                if let Some(s) = self.scopes.last_mut() {
                                    s.insert(v);
                                }
                            }
                        }
                        None => self.line(&call),
                    }
                }
                "CloneObject" => {
                    let o = self.obj_expr(b, "Object");
                    let parent = self.obj_expr(b, "Parent");
                    let var = self.bind_output_name(b, "Object");
                    let call = format!("{}:Clone()", o);
                    if let Some(v) = &var {
                        if self.declare_name(v) {
                            self.line(&format!("{} = {}", v, call));
                        } else {
                            self.line(&format!("local {} = {}", v, call));
                            if let Some(s) = self.scopes.last_mut() {
                                s.insert(v.clone());
                            }
                        }
                    } else {
                        self.line(&call);
                    }
                    if !parent.is_empty() && parent != "nil" {
                        if let Some(v) = &var {
                            self.line(&format!("{}.Parent = {}", v, parent));
                        }
                    }
                }
                "DestroyObject" => {
                    let o = self.obj_expr(b, "Object");
                    self.line(&format!("{}:Destroy()", o));
                }
                "FindPartOnRay" => {
                    let call = format!(
                        "workspace:Raycast({}, {})",
                        self.arg(b, "Origin"),
                        self.arg(b, "Direction")
                    );
                    self.assign_expr_outputs(b, &call);
                }
                "BindableFunctionInvoke" => {
                    let f = self.obj_expr(b, "BindableFunction");
                    let args = self.tuple_vals(b, "TUPLEPARAM_Parameters_");
                    let call = if args.is_empty() {
                        format!("{}:Invoke()", f)
                    } else {
                        format!("{}:Invoke({})", f, args.join(", "))
                    };
                    let outs = self.output_tuple(b, "TUPLEPARAM_ReturnedValues_");
                    if outs.is_empty() {
                        self.line(&call);
                    } else {
                        self.assign_expr_outputs(b, &call);
                    }
                }
                "ExecuteFunction" => {
                    let call = self.call_expr(b);
                    let outs = self.output_tuple(b, "TUPLEPARAM_ReturnedValues_");
                    if outs.is_empty() {
                        self.line(&call);
                    } else {
                        let new: Vec<bool> = outs
                            .iter()
                            .map(|o| !self.declare_name(o))
                            .collect();
                        let kw = if new.iter().all(|x| *x) {
                            "local"
                        } else {
                            ""
                        };
                        if kw.is_empty() {
                            self.line(&format!("{} = {}", outs.join(", "), call));
                        } else {
                            self.line(&format!("local {} = {}", outs.join(", "), call));
                            if let Some(s) = self.scopes.last_mut() {
                                for (o, is_new) in outs.iter().zip(&new) {
                                    if *is_new {
                                        s.insert(o.clone());
                                    }
                                }
                            }
                        }
                    }
                }
                "Spawn" => {
                    let f = self.obj_expr(b, "Function");
                    let args = self.tuple_vals(b, "TUPLEPARAM_Parameters_");
                    if args.is_empty() {
                        self.line(&format!("task.spawn({})", f));
                    } else {
                        self.line(&format!("task.spawn({}, {})", f, args.join(", ")));
                    }
                }
                "Print" => {
                    let tuple = self.tuple_vals(b, "TUPLEPARAM_");
                    if tuple.is_empty() {
                        self.line(&format!("print({})", self.arg(b, "Text")));
                    } else {
                        self.line(&format!("print({})", tuple.join(", ")));
                    }
                }
                "Warn" => {
                    self.line(&format!("warn({})", self.arg(b, "Text")));
                }
                "Wait" => {
                    self.line(&format!("task.wait({})", self.arg(b, "Time")));
                }
                "TakeDamage" => {
                    let p = self.obj_expr(b, "Player");
                    self.line(&format!("{}:TakeDamage({})", p, self.arg(b, "Damage")));
                }
                "PlaySound" => {
                    self.line(&format!("{}:Play()", self.obj_expr(b, "Sound")));
                }
                "StopSound" => {
                    self.line(&format!("{}:Stop()", self.obj_expr(b, "Sound")));
                }
                "RemoteFireServer2" => {
                    let r = self.obj_expr(b, "RemoteEvent");
                    let args = self.tuple_vals(b, "TUPLEPARAM_Parameters_");
                    if args.is_empty() {
                        self.line(&format!("{}:FireServer()", r));
                    } else {
                        self.line(&format!("{}:FireServer({})", r, args.join(", ")));
                    }
                }
                "RemoteFireAllClients2" => {
                    let r = self.obj_expr(b, "RemoteEvent");
                    let args = self.tuple_vals(b, "TUPLEPARAM_Parameters_");
                    if args.is_empty() {
                        self.line(&format!("{}:FireAllClients()", r));
                    } else {
                        self.line(&format!("{}:FireAllClients({})", r, args.join(", ")));
                    }
                }
                "RemoteFireClient2" => {
                    let r = self.obj_expr(b, "RemoteEvent");
                    let p = self.obj_expr(b, "Player");
                    let args = self.tuple_vals(b, "TUPLEPARAM_Parameters_");
                    if args.is_empty() {
                        self.line(&format!("{}:FireClient({})", r, p));
                    } else {
                        self.line(&format!("{}:FireClient({}, {})", r, p, args.join(", ")));
                    }
                }
                "BindableFire2" => {
                    let r = self.obj_expr(b, "BindableEvent");
                    let args = self.tuple_vals(b, "TUPLEPARAM_Parameters_");
                    if args.is_empty() {
                        self.line(&format!("{}:Fire()", r));
                    } else {
                        self.line(&format!("{}:Fire({})", r, args.join(", ")));
                    }
                }
                other if BODY_BLOCKS.contains(&other) => {
                    let (obj, event) = self.event_spec(b);
                    let params = self.event_params(b);
                    let kids = b.children.clone();
                    if params.is_empty() {
                        self.line(&format!("{}.{}:Connect(function()", obj, event));
                    } else {
                        self.line(&format!(
                            "{}.{}:Connect(function({})",
                            obj,
                            event,
                            params.join(", ")
                        ));
                    }
                    self.indent += 1;
                    let mut sc = HashSet::new();
                    for p in &params {
                        sc.insert(p.clone());
                    }
                    self.scopes.push(sc);
                    self.walk_seq(&kids, true);
                    self.scopes.pop();
                    self.indent -= 1;
                    self.line("end)");
                }
                unknown => {
                    self.note(
                        "GAP",
                        format!("statement block {:?} not reconstructed", unknown),
                    );
                    self.line(&format!("-- [detranspile] unsupported block: {}", unknown));
                }
            }
            // Leaf statements chain their continuation in `children`
            // (emit.rs sequence model). Body-owning blocks (If/loops/events/
            // DefineFunction/comments) already consumed their children above.
            if !BODY_BLOCKS.contains(&bt)
                && bt != "DefineFunction"
                && bt != "Comment2"
                && bt != "Comment"
                && bt != "FunctionReturn"
            {
                let kids = b.children.clone();
                self.walk_seq(&kids, false);
            }
        }
        if count_depth {
            self.depth -= 1;
        }
    }
}

/// Handled-statement check used by tests and the wasm bridge.
pub fn is_handled_statement(btype: &str) -> bool {
    HANDLED_STATEMENTS.contains(&btype) || BODY_BLOCKS.contains(&btype)
}

/// The emitter names intermediate bindings `t1`, `t2`, ... (see emit.rs
/// temp_counter). Anything else bound by a pure block is a source variable.
fn is_temp(binding: &str) -> bool {
    let rest = match binding.strip_prefix('t') {
        Some(r) if !r.is_empty() => r,
        _ => return false,
    };
    rest.bytes().all(|b| b.is_ascii_digit())
}

/// Hex number shape: optional '-', hex digits, optional '.' + hex digits.
fn looks_hex_num(v: &str) -> bool {
    let core = v.strip_prefix('-').unwrap_or(v);
    if core.is_empty() {
        return false;
    }
    let (whole, frac) = match core.split_once('.') {
        Some((w, f)) => (w, Some(f)),
        None => (core, None),
    };
    let hexish = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_hexdigit());
    hexish(whole) && frac.map(hexish).unwrap_or(true)
}

fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

fn bool_str(v: &str) -> String {
    if v == "1" {
        "true".to_string()
    } else {
        "false".to_string()
    }
}

fn comps(v: &str, n: usize) -> Vec<String> {
    let parts: Vec<String> = v
        .split(',')
        .map(|p| fmt_num(Some(p.trim())))
        .collect();
    let mut out = parts;
    while out.len() < n {
        out.push("0".to_string());
    }
    out.truncate(n);
    out
}

/// math fn: catalog stores e.g. "abs" or "math.abs" — normalize.
fn math_fn(f: &str) -> String {
    let f = f.trim();
    if f.starts_with("math.") {
        f.to_string()
    } else {
        format!("math.{}", f)
    }
}

/// `obj` + prop -> `obj.Prop` or `obj["Prop"]` for non-identifiers.
fn prop_path(obj: &str, prop: &str) -> String {
    let ident = !prop.is_empty()
        && prop
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        && !prop.bytes().next().unwrap_or(b'0').is_ascii_digit();
    if obj.is_empty() {
        if ident {
            format!(".{}", prop)
        } else {
            format!("[{}]", quote(prop))
        }
    } else if ident {
        format!("{}.{}", obj, prop)
    } else {
        format!("{}[{}]", obj, quote(prop))
    }
}
