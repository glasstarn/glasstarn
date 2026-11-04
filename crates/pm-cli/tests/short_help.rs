//! `glasstarn help` lists only the few commands people type; `help --all`
//! keeps the full reference agents use, and `demo` is the short form of
//! `workspace demo`.
use std::process::Command;

fn glasstarn(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_glasstarn"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn help_is_short_and_help_all_is_complete() {
    let short = String::from_utf8(glasstarn(&["help"]).stdout).unwrap();
    assert!(short.contains("glasstarn demo <name>"));
    assert!(short.contains("/glasstarn"));
    assert!(!short.contains("compare-periods"));
    assert!(short.lines().count() < 20, "{short}");
    let all = String::from_utf8(glasstarn(&["help", "--all"]).stdout).unwrap();
    for command in [
        "compare-periods",
        "workspace create",
        "mcp-policy",
        "glasstarn demo",
    ] {
        assert!(all.contains(command), "{command}");
    }
}

#[test]
fn demo_is_the_short_form_of_workspace_demo() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_str().unwrap();
    let made = glasstarn(&["demo", "sales-pipeline", "--workspace-dir", root, "--json"]);
    assert!(
        made.status.success(),
        "{}",
        String::from_utf8_lossy(&made.stderr)
    );
    let listed =
        String::from_utf8(glasstarn(&["workspace", "list", "--workspace-dir", root]).stdout)
            .unwrap();
    assert!(listed.contains("Sales pipeline (demo)"), "{listed}");
}

// The install page tells agents to confirm the install with `glasstarn --version`.
#[test]
fn version_prints_the_installed_version() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_glasstarn"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        format!("glasstarn {}", env!("CARGO_PKG_VERSION"))
    );
}
