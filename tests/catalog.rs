//! Phase 3 enforcement: the block catalog (src/vs_blocks.json) must cover
//! every block type observed in the fixtures, with matching input names,
//! output names (incl. tuple marking), and no deprecated types in use.
//!
//! Tuple outputs are stored in the catalog as nested lists `["Name"]`;
//! the game serializes them as `TUPLE_Name` (see docs/FORMAT.md 2.2).

use serde_json::Value;
use visual_source_rs::VisualSource;

const CATALOG: &str = include_str!("../src/vs_blocks.json");

fn cat_output_names(entry: &Value) -> Vec<String> {
    entry["Outputs"]
        .as_array()
        .map(|outs| {
            outs.iter()
                .map(|o| {
                    if o.is_array() {
                        o[0].as_str().unwrap_or_default().to_string()
                    } else {
                        o.as_str().unwrap_or_default().to_string()
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

fn cat_tuple_outputs(entry: &Value) -> Vec<String> {
    entry["Outputs"]
        .as_array()
        .map(|outs| {
            outs.iter()
                .filter(|o| o.is_array())
                .map(|o| o[0].as_str().unwrap_or_default().to_string())
                .collect()
        })
        .unwrap_or_default()
}

fn check_fixture(catalog: &Value, label: &str, blob: &str) {
    let report = VisualSource::decode(blob)
        .unwrap_or_else(|e| panic!("[{}] decode failed: {}", label, e));

    let mut checked_types = std::collections::BTreeSet::new();
    for (name, b) in report.blocks.iter() {
        let entry = catalog
            .get(&b.block_type)
            .unwrap_or_else(|| {
                panic!(
                    "[{}] block {:?} has type {:?} which is MISSING from vs_blocks.json",
                    label, name, b.block_type
                )
            });
        if entry["Category"].as_str() == Some("Deprecated") {
            panic!(
                "[{}] block {:?} uses DEPRECATED type {:?} - must never appear in fixtures",
                label, name, b.block_type
            );
        }
        if !checked_types.insert(b.block_type.clone()) {
            continue;
        }

        // Input names must agree as a set (slot order is fixture-verified at
        // generation time; instances only prove membership).
        let mut cat_in: Vec<&str> = entry["Inputs"]
            .as_array()
            .map(|ins| {
                ins.iter()
                    .map(|i| i["name"].as_str().unwrap_or_default())
                    .collect()
            })
            .unwrap_or_default();
        cat_in.sort();
        let mut fx_in: Vec<&str> = b.inputs.iter().map(|i| i.iname.as_str()).collect();
        fx_in.sort();
        assert_eq!(
            cat_in, fx_in,
            "[{}] type {:?}: catalog input names differ from fixture",
            label, b.block_type
        );

        // Output names (tuple outputs serialized as TUPLE_Name in payloads).
        let mut cat_out = cat_output_names(entry);
        cat_out.sort();
        let mut fx_out: Vec<(String, bool)> = b
            .outputs
            .iter()
            .map(|(oname, var)| {
                let is_tuple = oname.starts_with("TUPLE_") || var == "0";
                let bare = oname.strip_prefix("TUPLE_").unwrap_or(oname).to_string();
                (bare, is_tuple)
            })
            .collect();
        fx_out.sort();
        let mut cat_out_typed: Vec<(String, bool)> = {
            let tuples = cat_tuple_outputs(entry);
            cat_output_names(entry)
                .into_iter()
                .map(|n| (n.clone(), tuples.contains(&n)))
                .collect()
        };
        cat_out_typed.sort();
        assert_eq!(
            cat_out_typed, fx_out,
            "[{}] type {:?}: catalog outputs (name, is_tuple) differ from fixture",
            label, b.block_type
        );
        let _ = cat_out;
    }
    println!(
        "[{}] {} distinct types all covered",
        label,
        checked_types.len()
    );
}

#[test]
fn catalog_covers_script_fixture() {
    let catalog: Value = serde_json::from_str(CATALOG).unwrap();
    check_fixture(&catalog, "script", include_str!("../fixtures/script.vs"));
}

#[test]
fn catalog_covers_local_script_fixture() {
    let catalog: Value = serde_json::from_str(CATALOG).unwrap();
    check_fixture(
        &catalog,
        "local_script",
        include_str!("../fixtures/local_script.vs"),
    );
}

#[test]
fn catalog_covers_module_script_fixture() {
    let catalog: Value = serde_json::from_str(CATALOG).unwrap();
    check_fixture(
        &catalog,
        "module_script",
        include_str!("../fixtures/module_script.vs"),
    );
}

#[test]
fn catalog_is_wellformed() {
    let catalog: Value = serde_json::from_str(CATALOG).unwrap();
    let obj = catalog.as_object().expect("catalog root must be an object");
    for (name, entry) in obj {
        assert!(entry["Inputs"].is_array(), "{}: Inputs must be a list", name);
        assert!(entry["Outputs"].is_array(), "{}: Outputs must be a list", name);
        assert!(
            entry["Category"].is_string(),
            "{}: Category must be a string, got {:?}",
            name,
            entry["Category"]
        );
        for i in entry["Inputs"].as_array().unwrap() {
            assert!(i["name"].is_string(), "{}: input without name", name);
            assert!(
                i["value_type"].is_string() || i["value_type"].is_array(),
                "{}: input {} value_type must be string or list",
                name,
                i["name"]
            );
        }
    }
}
