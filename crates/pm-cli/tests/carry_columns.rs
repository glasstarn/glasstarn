//! An agent's case import keeps every other column by default, so each can
//! be filtered on; --leave-out drops one and --case-attribute keeps only the
//! ones listed.
use std::process::Command;

fn import(extra: &[&str]) -> serde_json::Value {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("orders.csv");
    std::fs::write(
        &source,
        "order,step,at,clerk,region,vendor\n\
         O1,Create,2025-01-01T09:00:00Z,Ann,West,Acme\n\
         O1,Ship,2025-01-03T09:00:00Z,Bo,West,Acme\n\
         O2,Create,2025-01-02T09:00:00Z,Ann,East,Bolt\n\
         O2,Ship,2025-01-09T09:00:00Z,Cy,East,Bolt\n",
    )
    .unwrap();
    let projects = dir.path().join("projects");
    let run = |args: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_glasstarn"))
            .args(args)
            .arg("--workspace-dir")
            .arg(&projects)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()
    };
    let created = run(&[
        "workspace",
        "create",
        "--name",
        "Orders",
        "--purpose",
        "improvement",
        "--json",
    ]);
    let id = created["project"]["id"].as_str().unwrap().to_owned();
    let mut args = vec![
        "workspace",
        "import",
        id.as_str(),
        source.to_str().unwrap(),
        "--case",
        "order",
        "--activity",
        "step",
        "--timestamp",
        "at",
        "--resource",
        "clerk",
        "--json",
        "--full",
    ];
    args.extend_from_slice(extra);
    run(&args)
}

fn attributes(result: &serde_json::Value) -> Vec<String> {
    let analysis = &result["analysis"];
    let list = if analysis.is_object() {
        &analysis["case_attributes"]
    } else {
        &result["headline"]["case_attributes"]
    };
    let mut names: Vec<String> = list
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

#[test]
fn every_other_column_is_kept_unless_left_out() {
    assert_eq!(attributes(&import(&[])), ["region", "vendor"]);
    assert_eq!(attributes(&import(&["--leave-out", "vendor"])), ["region"]);
    assert_eq!(
        attributes(&import(&["--case-attribute", "vendor"])),
        ["vendor"]
    );
}
