//! Catalog enforcement: src/vs_blocks.json must be well-formed - every
//! entry carries Inputs / Outputs lists and a Category string, and every
//! input has a name and a value_type. The transpiler test suite additionally
//! asserts nothing Deprecated/Unverified is ever emitted.

use serde_json::Value;

const CATALOG: &str = include_str!("../src/vs_blocks.json");

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
