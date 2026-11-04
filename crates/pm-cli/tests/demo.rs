//! The five-minute demo command, run on a small stand-in for the BPI
//! Challenge 2019 file with the same attributes.
use std::process::Command;

const STAND_IN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<log xes.version="1.0">
  <trace>
    <string key="concept:name" value="4507000001_00010"/>
    <string key="Company" value="companyID_0000"/>
    <string key="Document Type" value="Standard PO"/>
    <string key="GR-Based Inv. Verif." value="false"/>
    <string key="Goods Receipt" value="true"/>
    <string key="Item Category" value="3-way match, invoice before GR"/>
    <string key="Item Type" value="Standard"/>
    <string key="Spend area text" value="Packaging"/>
    <string key="Sub spend area text" value="Labels"/>
    <string key="Spend classification text" value="PR"/>
    <string key="Vendor" value="vendorID_0001"/>
    <string key="Name" value="vendor_0001"/>
    <string key="Purch. Doc. Category name" value="Purchase order"/>
    <string key="Purchasing Document" value="4507000001"/>
    <string key="Source" value="sourceSystemID_0000"/>
    <event>
      <string key="concept:name" value="Create Purchase Order Item"/>
      <string key="org:resource" value="user_001"/>
      <string key="User" value="user_001"/>
      <float key="Cumulative net worth (EUR)" value="420.0"/>
      <date key="time:timestamp" value="2018-01-02T09:00:00.000+01:00"/>
    </event>
    <event>
      <string key="concept:name" value="Record Goods Receipt"/>
      <string key="org:resource" value="batch_02"/>
      <string key="User" value="batch_02"/>
      <float key="Cumulative net worth (EUR)" value="420.0"/>
      <date key="time:timestamp" value="2018-01-09T09:00:00.000+01:00"/>
    </event>
    <event>
      <string key="concept:name" value="Clear Invoice"/>
      <string key="org:resource" value="NONE"/>
      <string key="User" value="NONE"/>
      <float key="Cumulative net worth (EUR)" value="420.0"/>
      <date key="time:timestamp" value="2018-02-20T09:00:00.000+01:00"/>
    </event>
  </trace>
</log>
"#;

#[test]
fn the_demo_builds_a_ready_project_from_the_bpic_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("BPI_Challenge_2019.xes");
    std::fs::write(&file, STAND_IN).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_glasstarn"))
        .args(["workspace", "demo", "bpic2019"])
        .arg(&file)
        .arg("--workspace-dir")
        .arg(dir.path().join("projects"))
        .arg("--json")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["status"], "complete");
    assert_eq!(value["project"]["name"], "BPIC 2019 purchasing (demo)");
    assert_eq!(value["headline"]["summary"]["cases"], 1);
    assert_eq!(
        value["headline"]["number_fields"][0],
        "Cumulative net worth (EUR)"
    );
    // Not the published file: it says so, and still builds.
    assert!(stderr.contains("not the published BPI Challenge 2019 file"));
    assert!(stderr.contains("docs/demo-bpic2019.md"));
}

#[test]
fn every_invented_demo_builds_a_ready_project_with_no_file() {
    let dir = tempfile::tempdir().unwrap();
    for (name, title) in [
        ("purchase-to-pay", "Purchase to pay (demo)"),
        ("order-to-cash", "Order to cash (demo)"),
        ("sales-pipeline", "Sales pipeline (demo)"),
        ("connected", "Order to cash, connected (demo)"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_glasstarn"))
            .args(["workspace", "demo", name, "--json"])
            .arg("--workspace-dir")
            .arg(dir.path().join("projects"))
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{name}: {stderr}");
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["status"], "complete", "{name}");
        assert_eq!(value["project"]["name"], title);
        assert!(
            stderr.contains("Invented data, no real company"),
            "{stderr}"
        );
        // Every case-log finding names the filter for exactly its items.
        if name != "connected" {
            let findings = value["headline"]["findings"].as_array().unwrap();
            assert!(!findings.is_empty(), "{name}");
            for finding in findings {
                assert!(finding["filter"].is_object(), "{name}: {finding}");
            }
        }
    }
}

#[test]
fn an_invented_demo_takes_no_file_and_unknown_names_are_listed() {
    let dir = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_glasstarn"))
            .args(args)
            .arg("--workspace-dir")
            .arg(dir.path())
            .output()
            .unwrap()
    };
    let extra = run(&["workspace", "demo", "order-to-cash", "orders.csv"]);
    assert!(!extra.status.success());
    let unknown = run(&["workspace", "demo", "payroll"]);
    let message = String::from_utf8_lossy(&unknown.stderr);
    assert!(
        message.contains("purchase-to-pay, order-to-cash, connected, sales-pipeline"),
        "{message}"
    );
}
