//! A company's export with a few records that do not fit: the check asks the
//! person once, and with their approval the build leaves those records out,
//! lists every one and records the decision.
use serde_json::Value;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

fn cli(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_glasstarn"))
        .args(args)
        .arg("--workspace-dir")
        .arg(root.join("projects"))
        .output()
        .unwrap()
}

fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
        panic!(
            "{}{}",
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

const PACK: &str = "appian-record-events";

#[test]
fn records_that_do_not_fit_are_set_aside_only_with_the_persons_approval() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let created = json(&cli(root, &["workspace", "create", "--name", "Rentals"]));
    let id = created["project"]["id"].as_str().unwrap().to_owned();
    let project = Path::new(created["project"]["path"].as_str().unwrap()).to_owned();
    let exports = root.join("exports");
    fs::create_dir_all(&exports).unwrap();
    let mut records = String::from("id,createdOn,status\n");
    let mut events = String::from("id,recordId,eventTypeId,timestamp,user\n");
    for n in 1..=20 {
        records.push_str(&format!("{n},2026-03-01T09:00:00Z,Open\n"));
        events.push_str(&format!("{n},{n},1,2026-03-01T09:00:00Z,pat\n"));
    }
    // One event belongs to a record that is not in the export.
    events.push_str("21,99,2,2026-03-02T09:00:00Z,sam\n");
    fs::write(exports.join("Record.csv"), records).unwrap();
    fs::write(exports.join("RecordEvent.csv"), events).unwrap();
    fs::write(
        exports.join("RecordEventType.csv"),
        "id,value\n1,Created\n2,Approved\n",
    )
    .unwrap();
    let files = ["Record", "RecordEvent", "RecordEventType"].map(|t| {
        exports
            .join(format!("{t}.csv"))
            .to_string_lossy()
            .into_owned()
    });
    let mut args = vec!["workspace", "inventory", id.as_str()];
    args.extend(files.iter().map(String::as_str));
    assert!(cli(root, &args).status.success());

    // The check is ready but asks the person about the record that does
    // not fit, first.
    let check = cli(root, &["workspace", "pack", "check", &id, "--pack", PACK]);
    assert!(check.status.success());
    let check = json(&check);
    assert_eq!(check["data_problems"].as_array().unwrap().len(), 1);
    assert_eq!(check["open_questions"][0]["id"], "data:set-aside");
    assert_eq!(check["open_questions"][0]["recommended"], "set_aside");
    assert!(check["next"].as_str().unwrap().contains("data:set-aside"));

    // Nothing is built until the person decides.
    let build = cli(root, &["workspace", "pack", "build", &id, "--pack", PACK]);
    assert!(!build.status.success());
    assert!(String::from_utf8_lossy(&build.stderr).contains("data:set-aside"));

    let answer = |choice: &str| {
        cli(
            root,
            &[
                "workspace",
                "pack",
                "answer",
                &id,
                "--pack",
                PACK,
                "--question",
                "data:set-aside",
                "--choice",
                choice,
                "--confirmed-by",
                "Operations lead",
            ],
        )
    };
    assert!(answer("new_export").status.success());
    let build = cli(root, &["workspace", "pack", "build", &id, "--pack", PACK]);
    assert!(!build.status.success());
    assert!(String::from_utf8_lossy(&build.stderr).contains("corrected export"));

    assert!(answer("set_aside").status.success());
    let build = cli(root, &["workspace", "pack", "build", &id, "--pack", PACK]);
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stdout)
    );
    let build = json(&build);
    assert_eq!(build["status"], "complete");
    assert_eq!(build["preparation"]["events"], 20);
    assert_eq!(build["preparation"]["set_aside_count"], 1);
    assert_eq!(
        build["preparation"]["set_aside"][0],
        serde_json::json!({"rule": "appian_event", "row": 21, "code": "unmatched_object_reference"})
    );
    assert!(
        build["pack"]["changes"]
            .to_string()
            .contains("Operations lead")
    );
    assert!(
        build["next"]
            .as_str()
            .unwrap()
            .contains("data-quality finding")
    );
    // Every row set aside is kept with the evidence, and the decision and
    // its effect are in the audit log.
    let evidence = Path::new(build["evidence"].as_str().unwrap());
    let listed: Value =
        serde_json::from_slice(&fs::read(evidence.join("set-aside.json")).unwrap()).unwrap();
    assert_eq!(listed.as_array().unwrap().len(), 1);
    let audit = fs::read_to_string(project.join("audit").join("log.jsonl")).unwrap();
    assert!(audit.contains("\"data:set-aside\""));
    assert!(audit.contains("preparation_set_aside"));
}

#[test]
fn a_build_that_finds_records_the_check_could_not_see_offers_the_same_choice() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let created = json(&cli(root, &["workspace", "create", "--name", "Rentals"]));
    let id = created["project"]["id"].as_str().unwrap().to_owned();
    let exports = root.join("exports");
    fs::create_dir_all(&exports).unwrap();
    fs::write(
        exports.join("Record.csv"),
        "id,createdOn,status\n1,2026-03-01T09:00:00Z,Open\n2,2026-03-01T09:00:00Z,Open\n",
    )
    .unwrap();
    // The second event has no time, which only the build finds.
    fs::write(
        exports.join("RecordEvent.csv"),
        "id,recordId,eventTypeId,timestamp,user\n1,1,1,2026-03-01T09:00:00Z,pat\n2,2,1,,pat\n",
    )
    .unwrap();
    fs::write(exports.join("RecordEventType.csv"), "id,value\n1,Created\n").unwrap();
    let files = ["Record", "RecordEvent", "RecordEventType"].map(|t| {
        exports
            .join(format!("{t}.csv"))
            .to_string_lossy()
            .into_owned()
    });
    let mut args = vec!["workspace", "inventory", id.as_str()];
    args.extend(files.iter().map(String::as_str));
    assert!(cli(root, &args).status.success());
    let check = json(&cli(
        root,
        &["workspace", "pack", "check", &id, "--pack", PACK],
    ));
    assert!(check["data_problems"].as_array().unwrap().is_empty());
    let blocked = cli(root, &["workspace", "pack", "build", &id, "--pack", PACK]);
    assert_eq!(blocked.status.code(), Some(3));
    let blocked = json(&blocked);
    assert!(blocked["next"].as_str().unwrap().contains("data:set-aside"));
    let answered = cli(
        root,
        &[
            "workspace",
            "pack",
            "answer",
            &id,
            "--pack",
            PACK,
            "--question",
            "data:set-aside",
            "--choice",
            "set_aside",
            "--confirmed-by",
            "Operations lead",
        ],
    );
    assert!(
        answered.status.success(),
        "{}",
        String::from_utf8_lossy(&answered.stderr)
    );
    let built = json(&cli(
        root,
        &["workspace", "pack", "build", &id, "--pack", PACK],
    ));
    assert_eq!(built["status"], "complete");
    assert_eq!(built["preparation"]["events"], 1);
    assert_eq!(
        built["preparation"]["set_aside"][0]["code"],
        "invalid_or_missing_timestamp"
    );
}
