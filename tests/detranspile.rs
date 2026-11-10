// Roundtrip stability test: Luau -> .vs -> Luau -> .vs' and compare the
// decoded block trees. Reconstruction is semantic, not byte-identical: the
// emitter folds constants and restructures guards, so we compare block-type
// multisets and report drift.
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use visual_source_rs::{transpile, VisualSource};

fn type_multiset(report: &visual_source_rs::decoder::DecodedReport) -> BTreeMap<String, usize> {
    let mut m: BTreeMap<String, usize> = BTreeMap::new();
    for b in report.blocks.values() {
        *m.entry(b.block_type.clone()).or_insert(0) += 1;
    }
    m
}

fn roundtrip_fixture(path: &PathBuf) -> Result<(usize, usize, String), String> {
    let src = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let label = path.file_name().unwrap().to_string_lossy().to_string();

    // 1) forward transpile
    let out1 = transpile::transpile(&src, &label, true)?;
    let payload1 = visual_source_rs::encoder::encode_payload(&out1.report).map_err(|e| e.to_string())?;

    // 2) detranspile payload -> Luau
    let dec1 = VisualSource::decode(&payload1).map_err(|e| e.to_string())?;
    let dt = transpile::detranspile::detranspile(&dec1);
    let gaps = dt.notes.iter().filter(|(k, _)| k == "GAP").count();
    if gaps > 0 {
        return Err(format!("{} gap(s) in detranspile: {:?}", gaps,
            dt.notes.iter().filter(|(k, _)| k == "GAP").map(|(_, m)| m.clone()).take(3).collect::<Vec<_>>()));
    }

    // 3) retranspile the reconstructed Luau
    let out2 = transpile::transpile(&dt.luau, &format!("{}<roundtrip>", label), true)?;
    let payload2 = visual_source_rs::encoder::encode_payload(&out2.report).map_err(|e| e.to_string())?;

    // 4) compare decoded trees
    let dec2 = VisualSource::decode(&payload2).map_err(|e| e.to_string())?;
    let m1 = type_multiset(&dec1);
    let m2 = type_multiset(&dec2);
    let total1: usize = m1.values().sum();
    let total2: usize = m2.values().sum();
    let mut same = 0usize;
    let mut drift = String::new();
    let keys: std::collections::BTreeSet<&String> = m1.keys().chain(m2.keys()).collect();
    for k in keys {
        let a = m1.get(k).copied().unwrap_or(0);
        let b = m2.get(k).copied().unwrap_or(0);
        same += a.min(b);
        if a != b {
            drift.push_str(&format!("\n    {}: {} -> {}", k, a, b));
        }
    }
    let ratio = if total1.max(total2) > 0 {
        same as f64 / total1.max(total2) as f64
    } else {
        1.0
    };
    Ok((
        total1,
        total2,
        format!("blocks {} -> {} | match {:.1}%{}", total1, total2, ratio * 100.0, drift),
    ))
}

#[test]
fn roundtrip_all_fixtures() {
    const MIN_MATCH: f64 = 0.5;
    let mut fixtures: Vec<PathBuf> = Vec::new();
    for d in ["fixtures/simon_tests", "fixtures"] {
        if let Ok(rd) = fs::read_dir(d) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().map(|x| x == "luau").unwrap_or(false) {
                    fixtures.push(p);
                }
            }
        }
    }
    fixtures.sort();
    assert!(fixtures.len() >= 10, "expected the fixture corpus, found {}", fixtures.len());

    let mut failures = Vec::new();
    for f in &fixtures {
        let name = f.file_name().unwrap().to_string_lossy().to_string();
        match roundtrip_fixture(f) {
            Ok((t1, t2, summary)) => {
                println!("  OK  {}: {}", name, summary);
                let ratio = if t1.max(t2) > 0 {
                    // recompute the match ratio from the summary line is
                    // wasteful; roundtrip_fixture already returns counts, and
                    // the drift details are printed for human review
                    let same = summary
                        .split("match ")
                        .nth(1)
                        .and_then(|s| s.split('%').next())
                        .and_then(|s| s.parse::<f64>().ok())
                        .unwrap_or(0.0);
                    same / 100.0
                } else {
                    1.0
                };
                if ratio < MIN_MATCH {
                    failures.push(format!(
                        "{}: block-type match {:.1}% below {:.0}% floor",
                        name,
                        ratio * 100.0,
                        MIN_MATCH * 100.0
                    ));
                }
            }
            Err(e) => failures.push(format!("{}: {}", name, e)),
        }
    }
    for f in &failures {
        println!("  FAIL {}", f);
    }
    assert!(failures.is_empty(), "{} fixture(s) failed roundtrip", failures.len());
}
