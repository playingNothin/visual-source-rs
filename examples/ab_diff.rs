//! Embedded A/B fixture: RetroStudio ground truth vs THIS build's output.
//!
//!   cargo run --example ab_diff                    (self-contained demo)
//!   cargo run --example ab_diff rs.txt ours.txt    (file mode)
//!
//! Demo flow: decode embedded RetroStudio export -> compile embedded Luau with
//! the current transpiler -> print both full reports -> diff record streams ->
//! regression-check against the earlier compiled fixture -> write
//! fresh_payload.txt (paste THAT into RetroStudio).

use std::env;
use std::fs;

use visual_source_rs::decoder;
use visual_source_rs::encoder;
use visual_source_rs::transpile;

const RETRO_PAYLOAD: &str = r##"0000000000000004xkg8@tK'_&eF{~HA`NQ7M.*>1HUcNU9PNf=-01#ZoE}]OOF>[)U+0%Hs3Pa7|yZVq>87Y~8wjjslF& d!=[h&ox!=G:#HW7!0`@leFfd/4-oJ`GifT9SC)1RPX$W]rpN![r)g:J,Px/?(0~cfx]rKXlTt _@6wsC>iIKk-qYWQ MT5,:pwwHQ0uV"##;
const OLD_COMPILED_PAYLOAD: &str = r##"0000000000000004%kcwpts|A,hi2wLs!SD%_y,8&~&!U:9RkE).r.olTmlS1Fr[k`z:JW_zXO` y2b'QZl({F{-$G+Ph]F#?oLsENW[#f9Km.i*tir#_5%1cK*YV1;U_ZdPI?w,Fdt3AOC+H)bT} 2cjF]W-~1I1ey(u;CiS1M=+:04lFA"##;
const LUAU: &str = r#"local test = "hi"
local test2 = 2"#;

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();

    // ---------- file mode ----------
    if let (Some(a), Some(b)) = (args.get(0), args.get(1)) {
        let ta = fs::read_to_string(a).unwrap_or_else(|e| panic!("cannot read {}: {}", a, e));
        let tb = fs::read_to_string(b).unwrap_or_else(|e| panic!("cannot read {}: {}", b, e));
        let ra = decoder::decode_blob(&ta).expect("decode A");
        let rb = decoder::decode_blob(&tb).expect("decode B");
        println!("{}", ra.build_report(&format!("A: {}", a)));
        println!("\n{}", rb.build_report(&format!("B: {}", b)));
        println!("\n{}", decoder::diff_raw(&ra, a, &rb, b));
        return;
    }

    // ---------- A) RetroStudio ground truth ----------
    println!("================ A) RETROSTUDIO GROUND TRUTH (embedded) ================\n");
    let retro = decoder::decode_blob(RETRO_PAYLOAD)
        .expect("embedded RetroStudio payload failed to decode (mangled in transit? use file mode)");
    println!("{}", retro.build_report("RetroStudio export: local test = \"hi\""));

    // ---------- B) current transpiler ----------
    println!("\n\n================ B) CURRENT TRANSPILE (this build) ================\n");
    let out = transpile::transpile(LUAU, "ab_diff fresh", true).expect("transpile failed");
    let records = encoder::encode_records(&out.report);
    let mut fresh = out.report;
    fresh.decompressed_len = records.len();
    fresh.raw_decompressed = records;
    println!("{}", fresh.build_report("fresh compile (current code)"));

    // ---------- C) verdict ----------
    println!("\n\n================ C) DIFF (RetroStudio vs fresh) ================\n");
    println!("{}", decoder::diff_raw(&retro, "retrostudio", &fresh, "fresh"));
    if retro.raw_decompressed == fresh.raw_decompressed {
        println!("VERDICT: record streams BYTE-IDENTICAL.");
    } else {
        println!("VERDICT: record streams differ (see diff above).");
        println!("  Known-harmless deltas: CameraPosition / CameraZoom / VisualPosition.");
        println!("  ANY other '-'/'+' line is a real bug.");
    }

    // ---------- D) regression vs the fixture already tested in RS ----------
    match decoder::decode_blob(OLD_COMPILED_PAYLOAD) {
        Ok(fix) => {
            if fix.raw_decompressed == fresh.raw_decompressed {
                println!("REGRESSION: current build == the compiled fixture from earlier.");
            } else {
                println!("REGRESSION: current build != the compiled fixture from earlier");
                println!("            (transpiler behavior changed since that payload was made)");
            }
        }
        Err(e) => println!("REGRESSION: embedded compiled fixture failed to decode: {}", e),
    }

    // ---------- E) payload to paste into RetroStudio ----------
    match encoder::encode_payload(&fresh) {
        Ok(p) => {
            fs::write("fresh_payload.txt", &p).expect("write fresh_payload.txt");
            println!(
                "\nwrote fresh_payload.txt ({} chars) - paste THIS into RetroStudio",
                p.len()
            );
        }
        Err(e) => println!("\ncould not build full payload: {}", e),
    }
}
