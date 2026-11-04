//! The consultant workflow on made-up SAP tables, through the command line:
//! export checklist, inventory, data check, answers confirmed by a pretend
//! analyst, build, then a refresh with a new export that reuses the answers.
use pm_testgen::sap_p2p::{Settings, TABLES, write};
use serde_json::Value;
use std::path::Path;
use std::process::{Command, Output};

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
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}
fn success(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    json(&output)
}
fn inventory(root: &Path, id: &str, dir: &Path) {
    let mut args = vec!["workspace".to_owned(), "inventory".into(), id.into()];
    args.extend(
        TABLES
            .iter()
            .map(|t| dir.join(format!("{t}.csv")).display().to_string()),
    );
    success(cli(
        root,
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
    ));
}
fn answer(root: &Path, id: &str, question: &str, choice: &str, value: Option<&str>) -> Output {
    let mut args = vec![
        "workspace",
        "pack",
        "answer",
        id,
        "--pack",
        "sap-purchase-to-pay",
        "--question",
        question,
        "--choice",
        choice,
        "--confirmed-by",
        "Purchasing analyst (pretend)",
    ];
    if let Some(value) = value {
        args.extend(["--value", value]);
    }
    cli(root, &args)
}
fn open_ids(check: &Value) -> Vec<String> {
    check["open_questions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|q| q["id"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn checklist_check_answer_build_and_refresh() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let list = success(cli(
        root,
        &[
            "workspace",
            "pack",
            "checklist",
            "sap-purchase-to-pay",
            "--full",
        ],
    ));
    assert_eq!(list["status"], "tested");
    assert_eq!(list["tables"].as_array().unwrap().len(), 12);

    let project = success(cli(
        root,
        &[
            "workspace",
            "create",
            "--name",
            "P2P",
            "--purpose",
            "improvement",
        ],
    ));
    let id = project["project"]["id"].as_str().unwrap().to_owned();
    let first = root.join("export-1");
    let truth = write(&first, &Settings::default()).unwrap();
    inventory(root, &id, &first);

    let check = success(cli(
        root,
        &[
            "workspace",
            "pack",
            "check",
            &id,
            "--pack",
            "sap-purchase-to-pay",
        ],
    ));
    assert_eq!(check["status"], "ready");
    assert!(serde_json::to_vec(&check).unwrap().len() <= 4096 + 512);
    let open = open_ids(&check);
    assert!(open.contains(&"setting:offset".to_owned()));
    assert!(open.contains(&"custom:EKKO.ZZAPPR_DAT".to_owned()));
    assert!(open.contains(&"link:paid-invoices-came-through-purchasing".to_owned()));

    // Nothing is built before the clock question is answered.
    let blocked = cli(
        root,
        &[
            "workspace",
            "pack",
            "build",
            &id,
            "--pack",
            "sap-purchase-to-pay",
        ],
    );
    assert!(!blocked.status.success());
    assert!(String::from_utf8_lossy(&blocked.stderr).contains("offset"));
    // Answers must fit the question.
    assert!(
        !answer(root, &id, "setting:offset", "value", Some("CET"))
            .status
            .success()
    );
    assert!(
        !answer(root, &id, "custom:EKKO.ZZAPPR_DAT", "maybe", None)
            .status
            .success()
    );
    assert!(
        !answer(root, &id, "custom:EKKO.ZZAPPR_DAT", "step", None)
            .status
            .success()
    );

    success(answer(root, &id, "setting:offset", "value", Some("+01:00")));
    success(answer(
        root,
        &id,
        "custom:EKKO.ZZAPPR_DAT",
        "step",
        Some("Approve purchase order"),
    ));
    success(answer(
        root,
        &id,
        "link:paid-invoices-came-through-purchasing",
        "expected",
        None,
    ));

    let built = success(cli(
        root,
        &[
            "workspace",
            "pack",
            "build",
            &id,
            "--pack",
            "sap-purchase-to-pay",
        ],
    ));
    assert_eq!(built["status"], "complete");
    assert_eq!(built["pack"]["changes"].as_array().unwrap().len(), 1);
    let rules = &built["preparation"]["rules"];
    assert_eq!(
        rules["custom_ekko_zzappr_dat"]["emitted"],
        truth.orders_with_custom_approver
    );
    assert_eq!(rules["invoice_recorded"]["emitted"], truth.invoices);

    // Answers are saved in the project and every one is in the audit log.
    let project_dir = Path::new(built["project"]["path"].as_str().unwrap()).to_owned();
    let log = std::fs::read_to_string(project_dir.join("audit/log.jsonl")).unwrap();
    assert_eq!(log.matches("\"pack_answer\"").count(), 3);
    assert!(log.contains("\"pack_build\""));

    // Refresh: a new export under the same file names. The latest inventory
    // wins, saved answers are reused and no question is asked again.
    let second = root.join("export-2");
    let refreshed = write(
        &second,
        &Settings {
            orders: 500,
            seed: 11,
            ..Settings::default()
        },
    )
    .unwrap();
    inventory(root, &id, &second);
    let check = success(cli(
        root,
        &[
            "workspace",
            "pack",
            "check",
            &id,
            "--pack",
            "sap-purchase-to-pay",
        ],
    ));
    assert!(open_ids(&check).is_empty(), "{:?}", open_ids(&check));
    let built = success(cli(
        root,
        &[
            "workspace",
            "pack",
            "build",
            &id,
            "--pack",
            "sap-purchase-to-pay",
        ],
    ));
    let rules = &built["preparation"]["rules"];
    assert_eq!(rules["order_created"]["emitted"], refreshed.orders);
    assert_eq!(
        rules["custom_ekko_zzappr_dat"]["emitted"],
        refreshed.orders_with_custom_approver
    );
}

#[test]
fn public_cloud_pack_and_accounts_payable_variant_from_the_command_line() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let packs = success(cli(root, &["workspace", "pack", "list"]));
    assert_eq!(packs["packs"].as_array().unwrap().len(), 59);

    // Public cloud: different files and field names, same workflow.
    let project = success(cli(
        root,
        &[
            "workspace",
            "create",
            "--name",
            "Cloud",
            "--purpose",
            "improvement",
        ],
    ));
    let id = project["project"]["id"].as_str().unwrap().to_owned();
    let files = root.join("cloud");
    let truth = pm_testgen::sap_p2p::write_cloud(&files, &Settings::default()).unwrap();
    let mut args = vec!["workspace".to_owned(), "inventory".into(), id.clone()];
    args.extend(
        pm_testgen::sap_p2p::CLOUD_TABLES
            .iter()
            .map(|t| files.join(format!("{t}.csv")).display().to_string()),
    );
    success(cli(
        root,
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
    ));
    let pack = "sap-s4-cloud-purchase-to-pay";
    success(cli(
        root,
        &["workspace", "pack", "check", &id, "--pack", pack],
    ));
    success(cli(
        root,
        &[
            "workspace",
            "pack",
            "answer",
            &id,
            "--pack",
            pack,
            "--question",
            "setting:offset",
            "--choice",
            "value",
            "--value",
            "+01:00",
            "--confirmed-by",
            "Analyst",
        ],
    ));
    let built = success(cli(
        root,
        &["workspace", "pack", "build", &id, "--pack", pack],
    ));
    assert_eq!(
        built["preparation"]["rules"]["invoice_recorded"]["emitted"],
        truth.invoices
    );

    // Accounts payable only: four on-premise tables are enough.
    let project = success(cli(
        root,
        &[
            "workspace",
            "create",
            "--name",
            "AP",
            "--purpose",
            "improvement",
        ],
    ));
    let id = project["project"]["id"].as_str().unwrap().to_owned();
    let files = root.join("ap");
    let truth = write(&files, &Settings::default()).unwrap();
    let mut args = vec!["workspace".to_owned(), "inventory".into(), id.clone()];
    args.extend(
        ["LFA1", "RBKP", "BKPF", "BSAK"]
            .iter()
            .map(|t| files.join(format!("{t}.csv")).display().to_string()),
    );
    success(cli(
        root,
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
    ));
    let checked = success(cli(
        root,
        &[
            "workspace",
            "pack",
            "check",
            &id,
            "--pack",
            "sap-purchase-to-pay",
            "--variant",
            "accounts-payable",
        ],
    ));
    assert_eq!(checked["pack"]["variant"], "accounts-payable");
    success(answer(root, &id, "setting:offset", "value", Some("+01:00")));
    let built = success(cli(
        root,
        &[
            "workspace",
            "pack",
            "build",
            &id,
            "--pack",
            "sap-purchase-to-pay",
        ],
    ));
    assert_eq!(built["pack"]["variant"], "accounts-payable");
    assert_eq!(
        built["preparation"]["rules"]["payment_of_invoice"]["emitted"],
        truth.invoices_paid
    );
}

/// Two packs combined for a company that uses both systems: the combined
/// pack is kept in the project and the usual check runs against it.
#[test]
fn two_packs_combine_in_a_project() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let packs = success(cli(root, &["workspace", "pack", "list"]));
    let sap = packs["packs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == "sap-purchase-to-pay")
        .unwrap();
    assert_eq!(sap["link_targets"][0]["id"], "purchase_order");
    let project = success(cli(
        root,
        &[
            "workspace",
            "create",
            "--name",
            "Requests to payment",
            "--purpose",
            "improvement",
        ],
    ));
    let id = project["project"]["id"].as_str().unwrap();
    let combined = success(cli(
        root,
        &[
            "workspace",
            "pack",
            "combine",
            id,
            "--pack",
            "appian-record-events",
            "--with",
            "sap-purchase-to-pay",
            "--link",
            "Record:purchase_order",
        ],
    ));
    assert_eq!(
        combined["pack"]["id"],
        "appian-record-events+sap-purchase-to-pay"
    );
    // The check finds the combined pack in the project; with nothing
    // exported yet it lists both systems' tables as missing.
    let output = cli(
        root,
        &[
            "workspace",
            "pack",
            "check",
            id,
            "--pack",
            "appian-record-events+sap-purchase-to-pay",
        ],
    );
    let report = json(&output);
    let missing = report["missing_tables"].to_string();
    assert!(
        missing.contains("Record") && missing.contains("EKKO"),
        "{report}"
    );
}

#[test]
fn the_export_request_is_saved_in_the_project_for_the_administrator() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let created = success(cli(
        root,
        &["workspace", "create", "--name", "Late payments"],
    ));
    let id = created["project"]["id"].as_str().unwrap().to_owned();
    let saved = success(cli(
        root,
        &[
            "workspace",
            "pack",
            "handout",
            &id,
            "--pack",
            "sap-purchase-to-pay",
            "--from",
            "2026-01-01",
            "--to",
            "2026-06-30",
            "--scope",
            "1000",
        ],
    ));
    let text = std::fs::read_to_string(saved["file"].as_str().unwrap()).unwrap();
    assert!(text.starts_with("# Data export request: Purchase to pay"));
    assert!(text.contains("for \"Late payments\""));
    assert!(text.contains("BUKRS = 1000"));
    assert_eq!(saved["tables"].as_array().unwrap().len(), 12);
    // The accounts payable variant asks only for its own tables.
    let variant = success(cli(
        root,
        &[
            "workspace",
            "pack",
            "handout",
            &id,
            "--pack",
            "sap-purchase-to-pay",
            "--variant",
            "accounts-payable",
        ],
    ));
    assert_eq!(variant["tables"].as_array().unwrap().len(), 6);
    let wrong = cli(
        root,
        &[
            "workspace",
            "pack",
            "handout",
            &id,
            "--pack",
            "sap-purchase-to-pay",
            "--from",
            "01/31/2026",
        ],
    );
    assert!(!wrong.status.success());
}
