use visual_source_rs::VisualSource;

#[test]
fn test_decode_script_fixture() {
    let script_vs = include_str!("../fixtures/script.vs");
    let report = VisualSource::decode(script_vs).expect("Failed to decode script.vs");

    assert_eq!(report.version, 4);
    assert_eq!(report.blocks.len(), 278, "Script must have exactly 278 blocks");
    assert_eq!(report.comments.len(), 0);

    // Verify key block types are present
    let types: Vec<&str> = report.blocks.values().map(|b| b.block_type.as_str()).collect();
    assert!(types.contains(&"HumanoidFreeFalling"));
    assert!(types.contains(&"IsServer"));
    assert!(types.contains(&"RemoteRecieveServer2"));
    assert!(types.contains(&"RemoteFireAllClients2"));
    assert!(types.contains(&"ForLoop2"));
    assert!(types.contains(&"WhileLoop3"));

    // Verify roots and order
    let roots: Vec<&String> = report.block_order.iter().filter(|name| {
        report.blocks.get(*name).unwrap().parents.is_empty()
    }).collect();
    assert_eq!(roots[0], "Humanoid Free Falling1");
    assert_eq!(roots[1], "To World Space1");
    assert_eq!(roots[2], "Receive Bindable Function1");
    assert_eq!(roots[3], "Vector3 Dot1");
    assert_eq!(roots[4], "Split UDim21");

    // Check specific inputs and outputs matching MonikaOS_DecodedVS.txt
    let hff = &report.blocks["Humanoid Free Falling1"];
    assert_eq!(hff.outputs, vec![("Active".to_string(), "".to_string())]);

    let rbf = &report.blocks["Receive Bindable Function1"];
    assert_eq!(rbf.outputs, vec![("TUPLE_Parameters".to_string(), "0".to_string())]);

    let v3d = &report.blocks["Vector3 Dot1"];
    assert_eq!(v3d.inputs[0].iname, "VectorA");
    assert_eq!(v3d.inputs[0].value.as_deref(), Some("0,0,0"));
    assert_eq!(v3d.inputs[1].iname, "VectorB");
    assert_eq!(v3d.inputs[1].value.as_deref(), Some("0,0,0"));

    // Check tree rendering produces non-empty output
    let tree = report.render_tree();
    assert!(!tree.is_empty());
    assert!(tree.contains("[Humanoid Free Falling1] HumanoidFreeFalling   (root - runs at startup)"));
    assert!(tree.contains("in  VectorA = \"0,0,0\"  [hex parts: (0, 0, 0)]"));
    assert!(tree.contains("out Parameters (tuple of 0) -> (no elements)"));

    // Check conversion to VisualSource struct
    let vs = report.to_visual_source();
    assert_eq!(vs.blocks.len(), 278);
}

#[test]
fn test_decode_local_script_fixture() {
    let local_vs = include_str!("../fixtures/local_script.vs");
    let report = VisualSource::decode(local_vs).expect("Failed to decode local_script.vs");

    assert_eq!(report.version, 4);
    assert_eq!(report.blocks.len(), 277, "LocalScript must have exactly 277 blocks");
    assert_eq!(report.comments.len(), 0);

    // Verify client-specific block types
    let types: Vec<&str> = report.blocks.values().map(|b| b.block_type.as_str()).collect();
    assert!(types.contains(&"RemoteRecieveClient2"));
    assert!(types.contains(&"RemoteFireServer2"));
    assert!(types.contains(&"RenderStepped"));
    assert!(types.contains(&"InputBegan"));
}

#[test]
fn test_decode_module_script_fixture() {
    let module_vs = include_str!("../fixtures/module_script.vs");
    let report = VisualSource::decode(module_vs).expect("Failed to decode module_script.vs");

    assert_eq!(report.version, 4);
    assert_eq!(report.blocks.len(), 1, "ModuleScript must have exactly 1 block");
    assert_eq!(report.comments.len(), 0);

    let module_return = report.blocks.get("Module Return1").expect("Module Return1 block must exist");
    assert_eq!(module_return.block_type, "ModuleReturn");
    assert_eq!(module_return.inputs.len(), 1);
    assert_eq!(module_return.inputs[0].iname, "Returns");

    let report_text = report.build_report("ModuleScript - Only Return");
    assert!(report_text.contains("SOURCE: ModuleScript - Only Return"));
    assert!(report_text.contains("blocks: 1 | comments: 0"));
    assert!(report_text.contains("block types: ModuleReturn x1"));
    assert!(report_text.contains("[Module Return1] ModuleReturn   (root - runs at startup)"));
    assert!(report_text.contains("in  Returns = \"\"   [type: String]"));
    assert!(report_text.contains("parents: (root)"));
    assert!(report_text.contains("(none)"));
    assert!(report_text.contains("editor: zoom=0.92 camera=-1.F3,1.4A"));
}

#[test]
fn test_output_lines_tuple_grouping() {
    use visual_source_rs::decoder::fmt_output_lines;

    // The encoded shape a DefineFunction block carries: one plain output, the
    // tuple container (count as hex string), and one wire per parameter.
    let outputs = vec![
        ("Function".to_string(), "check".to_string()),
        ("TUPLE_Parameters".to_string(), "5".to_string()),
        ("TUPLEPARAM_Parameters_1".to_string(), "id".to_string()),
        ("TUPLEPARAM_Parameters_2".to_string(), "name".to_string()),
        ("TUPLEPARAM_Parameters_3".to_string(), "ok".to_string()),
        ("TUPLEPARAM_Parameters_4".to_string(), "expected".to_string()),
        ("TUPLEPARAM_Parameters_5".to_string(), "got".to_string()),
    ];
    let lines = fmt_output_lines(&outputs);
    assert_eq!(lines[0], "out Function -> $check");
    assert_eq!(
        lines[1],
        "out Parameters (tuple of 5) -> ($id, $name, $ok, $expected, $got)"
    );

    // A 10-element tuple proves the count is read as hex, not decimal.
    let mut ten = vec![("TUPLE_Parameters".to_string(), "a".to_string())];
    for i in 1..=10 {
        ten.push((format!("TUPLEPARAM_Parameters_{}", i), format!("p{}", i)));
    }
    let lines = fmt_output_lines(&ten);
    assert!(lines[0].starts_with("out Parameters (tuple of 10) -> ("));
    assert!(lines[0].ends_with("$p10)"));

    // Elements without their container still group, by element count.
    let loose = vec![("TUPLEPARAM_Parameters_1".to_string(), "x".to_string())];
    let lines = fmt_output_lines(&loose);
    assert_eq!(lines[0], "out Parameters (tuple) -> ($x)");
}
