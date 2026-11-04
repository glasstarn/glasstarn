//! Discovery reads only metadata through the person's Salesforce CLI alias
//! and names what a pack needs that the org does not keep.
use serde_json::Value;
use std::path::Path;
use std::process::{Command, Output};

/// A stand-in for the official `sf` CLI: Case has Status and Owner history
/// but not Priority; Product__c is the company's own field.
#[cfg(unix)]
const SF: &str = r#"#!/bin/sh
args="$*"
# Like the real CLI: a version must read like 66.0.
prev=""
for a in "$@"; do
  [ "$prev" = "--api-version" ] && case "$a" in *.0) ;; *) printf '{"status":2,"message":"invalid api version"}'; exit 2 ;; esac
  prev="$a"
done
case "$args" in
  *"sobject describe --sobject CaseHistory"*)
    printf '{"status":0,"result":{"name":"CaseHistory","fields":[{"name":"Id","custom":false}]}}' ;;
  *"sobject describe --sobject Case "*)
    printf '{"status":0,"result":{"name":"Case","fields":[{"name":"Id","custom":false,"type":"id"},{"name":"CreatedDate","custom":false},{"name":"Status","custom":false},{"name":"Priority","custom":false},{"name":"Origin","custom":false},{"name":"IsClosed","custom":false},{"name":"OwnerId","custom":false},{"name":"Product__c","label":"Product","type":"picklist","custom":true}]}}' ;;
  *"data query"*)
    printf '{"status":0,"result":{"totalSize":2,"records":[{"QualifiedApiName":"Status"},{"QualifiedApiName":"OwnerId"}]}}' ;;
  *) printf '{"status":1,"message":"not found"}' ;;
esac
"#;

/// The same stand-in for Windows, where the CLI is `sf.cmd`: a batch file
/// that hands its arguments to this script.
#[cfg(windows)]
const SF_JS: &str = r#"
const args = process.argv.slice(2);
const all = args.join(" ") + " ";
const out = (text, code) => { process.stdout.write(text); process.exit(code); };
const at = args.indexOf("--api-version");
if (at >= 0 && !String(args[at + 1]).endsWith(".0")) out('{"status":2,"message":"invalid api version"}', 2);
if (all.includes("sobject describe --sobject CaseHistory"))
  out('{"status":0,"result":{"name":"CaseHistory","fields":[{"name":"Id","custom":false}]}}', 0);
if (all.includes("sobject describe --sobject Case "))
  out('{"status":0,"result":{"name":"Case","fields":[{"name":"Id","custom":false,"type":"id"},{"name":"CreatedDate","custom":false},{"name":"Status","custom":false},{"name":"Priority","custom":false},{"name":"Origin","custom":false},{"name":"IsClosed","custom":false},{"name":"OwnerId","custom":false},{"name":"Product__c","label":"Product","type":"picklist","custom":true}]}}', 0);
if (all.includes("data query"))
  out('{"status":0,"result":{"totalSize":2,"records":[{"QualifiedApiName":"Status"},{"QualifiedApiName":"OwnerId"}]}}', 0);
out('{"status":1,"message":"not found"}', 1);
"#;

fn cli(root: &Path, tools: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_glasstarn"))
        .args(args)
        .arg("--workspace-dir")
        .arg(root.join("projects"))
        .env("PATH", tools)
        .output()
        .unwrap()
}

#[test]
fn discovery_finds_own_fields_and_untracked_history_the_pack_needs() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let tools = root.join("tools");
    std::fs::create_dir_all(&tools).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let sf = tools.join("sf");
        std::fs::write(&sf, SF).unwrap();
        std::fs::set_permissions(&sf, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    #[cfg(windows)]
    {
        // glasstarn starts `sf.cmd` there, as the real CLI installs.
        let node = Command::new("where")
            .arg("node")
            .output()
            .expect("node is needed for the Windows stand-in");
        let node = String::from_utf8_lossy(&node.stdout)
            .lines()
            .next()
            .expect("node is needed for the Windows stand-in")
            .trim()
            .to_owned();
        std::fs::write(tools.join("sf.js"), SF_JS).unwrap();
        std::fs::write(
            tools.join("sf.cmd"),
            format!("@\"{node}\" \"%~dp0sf.js\" %*\r\n"),
        )
        .unwrap();
    }
    let created = cli(
        root,
        &tools,
        &[
            "workspace",
            "create",
            "--name",
            "Cases",
            "--purpose",
            "improvement",
        ],
    );
    let id = serde_json::from_slice::<Value>(&created.stdout).unwrap()["project"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let output = cli(
        root,
        &tools,
        &[
            "extract",
            "discover",
            &id,
            "--org",
            "approved-org",
            "--object",
            "Case",
            "--pack",
            "salesforce-case-handling",
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let found: Value = serde_json::from_slice(&output.stdout).unwrap();
    let case = &found["objects"][0];
    assert_eq!(case["custom_fields"][0]["name"], "Product__c");
    assert_eq!(case["history_object"], "CaseHistory");
    let gaps = found["gaps"].as_array().unwrap();
    assert_eq!(gaps.len(), 1, "{gaps:?}");
    assert!(
        gaps[0]
            .as_str()
            .unwrap()
            .contains("History tracking is off for Priority on Case")
    );
    // Saved in the project as evidence.
    let saved = root
        .join("projects")
        .join(&id)
        .join("extraction/discovery-salesforce.json");
    assert!(saved.is_file());
}
