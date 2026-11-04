//! A project started before there were files moves, with everything saved in
//! it, into the person's folder once their files arrive, and stays listed.
use std::process::Command;

#[test]
fn a_project_moves_into_the_persons_folder_and_keeps_its_answers() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_glasstarn"))
            .args(args)
            .env("HOME", home.path())
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice::<serde_json::Value>(&out.stdout).unwrap()
    };
    let made = run(&["workspace", "create", "--name", "Purchase orders"]);
    let id = made["project"]["id"].as_str().unwrap().to_owned();
    let plan = work.path().join("plan.json");
    std::fs::write(&plan, r#"{"version":1,"question":"Why are purchase orders slow?","answer":"Waiting for the SAP export.","findings":[]}"#).unwrap();
    run(&[
        "workspace",
        "record",
        &id,
        "--request",
        plan.to_str().unwrap(),
    ]);

    let target = work.path().join("glasstarn");
    let moved = run(&["workspace", "move", &id, "--to", target.to_str().unwrap()]);
    assert_eq!(moved["status"], "moved");
    assert!(target.join(&id).join("workspace.json").is_file());
    assert!(!home.path().join(".glasstarn/projects").join(&id).exists());
    // Its saved answer came along, and it is still listed from the default home.
    let findings = std::fs::read_dir(target.join(&id).join("findings"))
        .unwrap()
        .count();
    assert_eq!(findings, 1);
    let shown = run(&[
        "workspace",
        "show",
        &id,
        "--workspace-dir",
        target.to_str().unwrap(),
    ]);
    assert_eq!(shown["project"]["name"], "Purchase orders");
    let remembered = std::fs::read_to_string(home.path().join(".glasstarn/folders.json")).unwrap();
    assert!(remembered.contains("glasstarn"), "{remembered}");
}
