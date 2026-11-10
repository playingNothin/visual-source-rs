//! RetroStudio support layer (separated from the transpiler core - user
//! directive): everything about mapping Luau constructs that have no direct
//! block onto proven block shapes lives HERE, not in emit.rs.
//!
//! Contents:
//! - the unsupported deny-list + in-script Comment2 markers (user spec:
//!   detect, drop, comment inside the imported script, alert in CLI/GUI)
//! - synthesis for constructs that ARE compilable from proven ops:
//!     * Random.new / :NextNumber / :NextInteger -> Park-Miller LCG
//!       equations (proven ops: * / math.floor; % PROVEN in equations
//!       since run 7 E3 - [patch19] kept the synth as-is because it
//!       works). The binding returned by Random.new IS the state;
//!       next-calls advance it by name (RS name-resolved vars, T8).
//! - is_math_call routing (unknown math.* -> SolveEquation; G1/G4 proved
//!   RS's equation evaluator knows noise/abs/...)
//! - the BrickColor name palette (wire form IS the palette number)
//!
//! Adding support for a new construct = add/extend a synth fn in THIS file.
//! The core emitter only routes. No emit.rs surgery needed anymore.

use super::ast::Expr;
use super::emit::{EmittedBlock, InputVal, Transpiler};

/// Confirmed RetroStudio-unsupported constructs (user-verified:
/// string.format). Anything NOT in this list and not otherwise armed gets
/// flagged dynamically by the call/method fallbacks - implementing an arm
/// for a construct automatically un-flags it.
pub(crate) fn rs_unsupported(name: &str) -> bool {
    matches!(name, "string.format") || matches!(name, "pcall" | "xpcall")
}

/// [patch19] Run-7 experiment ledger (user-tested in RetroStudio, one
/// Solve Equation block per row). Do NOT re-test these:
/// PROVEN inside SolveEquation: `%` (7 % 3), `^` (2 ^ 10), string
/// relational `<` ("a" < "b"), and table-building braces `{1, 2, 3}`
/// whose Result wires to a table variable - plus the older proofs
/// (#len, dot member reads, math.*, ~= nil truthy wraps).
/// REJECTED inside SolveEquation: `..` concat (E1 - always decompose to
/// Concatenate) and all string.* functions (E2 - string.format was
/// already denied; string.byte/sub/find/char all fail). Luau has no
/// `//` operator, so E4's integer-division failure is moot. Reading
/// table values inside equations (bracket indexing) was reported
/// working by the user; key semantics are unverified - keep
/// GetTableValue blocks until proven.
/// [patch21] run 8 (2026-11-08): explicit-key table constructors FAIL
/// TO COMPILE - `{["!"] = 33}` (string keys) and `{[32] = " "}` (number
/// keys) both raised "Error compiling equation: attempt to index nil
/// with number" on Solve Equation1/2. The text survives import/re-save
/// byte-identical; only runtime COMPILATION rejects it. patch20's
/// dictionary fast path was rolled back - only `{1, 2, 3}` positional
/// numbers may ride an equation.
/// [patch22] run 9 (2026-11-08, user root-cause): RetroStudio parses
/// the equation as ONE raw chunk - NOT Roblox per-value tokenizing.
/// Strings carry no escape sequences: space, `/`, `\`, brackets and
/// single quotes inside a literal are ordinary characters; the only
/// terminator is `"`, so a quote-bearing literal makes `"""` and
/// shreds the parse (that - not explicit keys - is what killed the v20
/// dictionaries and patch21's E9 probe). Explicit-key braces are
/// re-armed: quote-free strings ride RAW (no escaping - `\\` would
/// silently become two backslashes); quote-bearing entries defer to
/// one SetTableValue each.

/// BrickColor name -> palette number (RS wire form IS the number; user type
/// table ground truth). Extend as needed - unknown names produce a GAP note
/// + BrickColor 0.
pub(crate) fn brickcolor_number(name: &str) -> Option<f64> {
    Some(match name {
        "White" => 1.0,
        "Brick yellow" => 5.0,
        "Bright red" => 21.0,
        "Bright blue" => 23.0,
        "Bright yellow" => 28.0,
        "Bright green" => 37.0,
        "Earth green" => 141.0,
        "Medium stone grey" => 194.0,
        "Dark stone grey" => 199.0,
        "Reddish brown" => 217.0,
        "Institutional white" => 235.0,
        _ => return None,
    })
}

/// Unknown math.* calls route to SolveEquation (user tip: RS's equation
/// evaluator is the universal escape hatch - G1 proved math.noise, G4
/// proved math.abs).
pub(crate) fn is_math_call(f: &Expr) -> bool {
    matches!(
        f,
        Expr::Index(b, _) if matches!(&**b, Expr::Name(ns) if ns == "math")
    )
}

/// RNG synthesis: does this method name have a compiled form?
pub(crate) fn rng_has_form(name: &str) -> bool {
    matches!(name, "NextNumber" | "NextInteger")
}

impl Transpiler {
    /// Drops the construct: [UNSUPPORTED] note + a Comment2 marker in the
    /// imported script (deduped per construct) so the failure is visible in
    /// exactly the place the user works. Caller returns a neutral value
    /// (nil - never the old truthy-garbage Bool(true)).
    pub(crate) fn flag_unsupported(&mut self, name: &str, chain: &mut Vec<String>) {
        self.note(
            "UNSUPPORTED",
            format!(
                "{} isn't supported by RetroStudio/Compiler, implement it yourself",
                name
            ),
        );
        if self.unsup_emitted.insert(name.to_string()) {
            let text = format!(
                "-- {} isn't supported by RetroStudio/Compiler, implement it yourself",
                name
            );
            let cname = self.display_name("Comment2");
            self.validate("Comment2", &[]);
            self.blocks.push(EmittedBlock {
                name: cname.clone(),
                btype: "Comment2".into(),
                inputs: vec![
                    ("Color".into(), InputVal::Implicit("3E9".into())),
                    ("Comment".into(), InputVal::Implicit(text)),
                ],
                outputs: Vec::new(),
                children: Vec::new(),
                else_child: None,
            });
            self.index.insert(cname.clone(), self.blocks.len() - 1);
            chain.push(cname);
        }
    }

    /// Random.new(seed): Park-Miller state init. The RETURNED BINDING IS THE
    /// STATE - :NextNumber/:NextInteger advance it by name. No seed ->
    /// OsClock (tick-alias precedent).
    pub(crate) fn rs_synth_random_new(
        &mut self,
        args: &[Expr],
        out: &str,
        chain: &mut Vec<String>,
    ) {
        let seed_eq = match args.first() {
            Some(e) => self.eq_expr(e),
            None => {
                let s = self.temp();
                self.emit(
                    "OsClock",
                    vec![],
                    vec![("Answer".into(), s.clone())],
                    chain,
                );
                s
            }
        };
        // state = seed - 2147483647*floor(seed / 2147483647)
        // (Park-Miller modulus WITHOUT %; exact in f64 for seed < 2^53)
        let eq = format!(
            "({0} - (2147483647 * math.floor(({0} / 2147483647))))",
            seed_eq
        );
        self.emit(
            "SolveEquation",
            vec![(
                "Equation".into(),
                InputVal::Literal { value: eq, vtype: "String".into() },
            )],
            vec![("Result".into(), out.to_string())],
            chain,
        );
    }

    /// :NextNumber() / :NextNumber(lo, hi) / :NextInteger(lo, hi) on a
    /// VARIABLE base (the binding IS the LCG state). Advances the state
    /// (write-back by name) then computes the result. Returns true when
    /// synthesized (result already emitted into `out`); false when not
    /// applicable (caller flags unsupported).
    pub(crate) fn rs_try_synth_rng(
        &mut self,
        method: &str,
        state: &str,
        margs: &[Expr],
        out: &str,
        chain: &mut Vec<String>,
    ) -> bool {
        if !rng_has_form(method) {
            return false;
        }
        let shape_ok = matches!(
            (method, margs.len()),
            ("NextNumber", 0) | ("NextNumber", 2) | ("NextInteger", 2)
        );
        if !shape_ok {
            self.note(
                "GAP",
                format!(
                    ":{} with {} args has no synthesized form",
                    method,
                    margs.len()
                ),
            );
            return false;
        }
        // advance: state = state*16807 - 2147483647*floor(state*16807/2147483647)
        let adv = format!(
            "(({0} * 16807) - (2147483647 * math.floor(({0} * 16807) / 2147483647)))",
            state
        );
        self.emit(
            "SolveEquation",
            vec![(
                "Equation".into(),
                InputVal::Literal { value: adv, vtype: "String".into() },
            )],
            vec![("Result".into(), state.to_string())],
            chain,
        );
        // result
        let eq = match (method, margs.len()) {
            ("NextNumber", 0) => format!("({} / 2147483647)", state),
            ("NextNumber", 2) => format!(
                "({0} + (({1} / 2147483647) * ({2} - {0})))",
                self.eq_expr(&margs[0]),
                state,
                self.eq_expr(&margs[1])
            ),
            _ => format!(
                "({0} + math.floor((({1} / 2147483647) * (({2} - {0}) + 1))))",
                self.eq_expr(&margs[0]),
                state,
                self.eq_expr(&margs[1])
            ),
        };
        self.emit(
            "SolveEquation",
            vec![(
                "Equation".into(),
                InputVal::Literal { value: eq, vtype: "String".into() },
            )],
            vec![("Result".into(), out.to_string())],
            chain,
        );
        true
    }
}
