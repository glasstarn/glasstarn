//! Real CSV inventory -> reviewed OCEL construction -> native Rust analysis.
use serde_json::{Value, json};
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
fn success(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
fn fixture() -> (tempfile::TempDir, String, Value) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let example =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/connected-sales-to-cash");
    let mut recipe: Value =
        serde_json::from_slice(&fs::read(example.join("recipe.template.json")).unwrap()).unwrap();
    let project = success(cli(
        root,
        &[
            "workspace",
            "create",
            "--name",
            "Connected cash",
            "--purpose",
            "improvement",
        ],
    ));
    let id = project["project"]["id"].as_str().unwrap().to_owned();
    for input in recipe["inputs"].as_array_mut().unwrap() {
        let name = input["id"].as_str().unwrap();
        let path = root.join(format!("{name}.csv"));
        fs::copy(example.join(format!("{name}.csv")), &path).unwrap();
        let inventory = success(cli(
            root,
            &["workspace", "inventory", &id, path.to_str().unwrap()],
        ));
        input["sha256"] = inventory["inputs"][0]["asset"]["sha256"].clone();
    }
    fs::write(
        root.join("recipe.json"),
        serde_json::to_vec(&recipe).unwrap(),
    )
    .unwrap();
    (dir, id, recipe)
}
fn prepare(root: &Path, id: &str) -> Output {
    cli(
        root,
        &[
            "workspace",
            "prepare-ocel",
            id,
            "--recipe",
            root.join("recipe.json").to_str().unwrap(),
        ],
    )
}
#[test]
fn split_orders_shared_and_partial_payments_stay_atomic_and_reproducible() {
    let (dir, id, _) = fixture();
    let root = dir.path();
    let result = success(prepare(root, &id));
    assert_eq!(result["status"], "complete");
    assert_eq!(result["preparation"]["objects"], 10);
    assert_eq!(result["preparation"]["events"], 10);
    assert_eq!(result["preparation"]["object_relationships"], 10);
    assert_eq!(result["preparation"]["event_relationships"], 20);
    let state = success(cli(root, &["workspace", "show", &id]));
    assert_eq!(state["data_kind"], "ocel");
    let project = Path::new(state["project"]["path"].as_str().unwrap());
    let path = project.join(result["saved_source"].as_str().unwrap());
    let log = pm_ingest::read_ocel(&path).unwrap();
    assert_eq!(log.events.len(), 10);
    assert!(
        path.extension()
            .is_some_and(|extension| extension == "sqlite")
    );
    let payment = (0..log.events.len())
        .find(|index| {
            log.event_label(pm_core::OcelEventId(*index as u32))
                .unwrap()
                .contains("P1")
        })
        .unwrap();
    let payment_links = log
        .event_object_relations
        .iter()
        .filter(|relation| relation.event.0 as usize == payment)
        .count();
    assert_eq!(payment_links, 3); // payment + two invoices, one event
    let invoices = (0..log.objects.len())
        .filter(|index| {
            log.object_type(log.objects[*index].object_type)
                .unwrap()
                .name
                == "invoice"
        })
        .map(|index| {
            log.object_label(pm_core::ObjectId(index as u32))
                .unwrap()
                .to_owned()
        })
        .collect::<Vec<_>>();
    assert_eq!(invoices.len(), 3);
    assert_ne!(invoices[0], invoices[2]); // same invoice number in different companies
    assert!(invoices[0].contains("0010"));
    let lineage: Value = {
        use std::io::Read;
        let file = fs::File::open(
            Path::new(result["evidence"].as_str().unwrap()).join("relationship-lineage.json.gz"),
        )
        .unwrap();
        let mut text = String::new();
        flate2::read::GzDecoder::new(file)
            .read_to_string(&mut text)
            .unwrap();
        serde_json::from_str(&text).unwrap()
    };
    assert_eq!(lineage.as_array().unwrap().len(), 30);
    assert_eq!(lineage[0]["input_sha256"].as_str().unwrap().len(), 64);
    let rerun = success(prepare(root, &id));
    assert_eq!(rerun["saved_source"], result["saved_source"]);
    let analysis = success(cli(root, &["workspace", "analyze", &id]));
    assert_eq!(analysis["kind"], "ocel");
}
#[test]
fn broken_links_and_duplicate_keys_block_without_replacing_previous_analysis() {
    for (input, extra, code) in [
        (
            "payment_invoices",
            "P1,A,9999\n",
            "unmatched_object_reference",
        ),
        (
            "payment_invoices",
            "P1,A,0010\n",
            "duplicate_event_relationship",
        ),
        (
            "invoices",
            "A,0010,0007,2026-01-03T09:00:00Z\n",
            "duplicate_object_key",
        ),
    ] {
        let (dir, id, mut recipe) = fixture();
        let root = dir.path();
        let previous = success(prepare(root, &id));
        let path = root.join(format!("{input}.csv"));
        let mut text = fs::read_to_string(&path).unwrap();
        text.push_str(extra);
        fs::write(&path, text).unwrap();
        let inventory = success(cli(
            root,
            &["workspace", "inventory", &id, path.to_str().unwrap()],
        ));
        for reference in recipe["inputs"].as_array_mut().unwrap() {
            if reference["id"] == input {
                reference["sha256"] = inventory["inputs"][0]["asset"]["sha256"].clone();
            }
        }
        fs::write(
            root.join("recipe.json"),
            serde_json::to_vec(&recipe).unwrap(),
        )
        .unwrap();
        let output = prepare(root, &id);
        assert_eq!(output.status.code(), Some(3));
        let blocked: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(blocked["status"], "blocked");
        assert!(
            blocked["preparation"]["issues"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v["code"] == code)
        );
        let state = success(cli(root, &["workspace", "show", &id]));
        assert_eq!(state["source"]["relative_path"], previous["saved_source"]);
    }
}
#[test]
fn unknown_recipe_code_and_hashes_fail_closed() {
    let (dir, id, mut recipe) = fixture();
    let root = dir.path();
    recipe["script"] = json!("do something");
    fs::write(
        root.join("recipe.json"),
        serde_json::to_vec(&recipe).unwrap(),
    )
    .unwrap();
    assert!(!prepare(root, &id).status.success());
    recipe.as_object_mut().unwrap().remove("script");
    recipe["inputs"][0]["sha256"] = json!("a".repeat(64));
    fs::write(
        root.join("recipe.json"),
        serde_json::to_vec(&recipe).unwrap(),
    )
    .unwrap();
    assert!(!prepare(root, &id).status.success());
}

#[test]
fn timestamp_and_key_issues_are_counted_with_bounded_examples() {
    let (dir, id, mut recipe) = fixture();
    let root = dir.path();
    let path = root.join("payments.csv");
    let mut text = fs::read_to_string(&path).unwrap();
    for _ in 0..60 {
        text.push_str(",2026-01-06T09:00:00Z\n");
    }
    text.push_str("P4,not-a-timestamp\n");
    fs::write(&path, text).unwrap();
    let inventory = success(cli(
        root,
        &["workspace", "inventory", &id, path.to_str().unwrap()],
    ));
    recipe["inputs"][3]["sha256"] = inventory["inputs"][0]["asset"]["sha256"].clone();
    fs::write(
        root.join("recipe.json"),
        serde_json::to_vec(&recipe).unwrap(),
    )
    .unwrap();
    let output = prepare(root, &id);
    assert_eq!(output.status.code(), Some(3));
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["preparation"]["issue_count"], 121);
    // The printed answer is held to the agent budget and says what it cut.
    let shown = result["preparation"]["issues"].as_array().unwrap().len();
    assert_eq!(
        shown as u64
            + result["preparation"]["issues_omitted"]
                .as_u64()
                .unwrap_or(0),
        50
    );
    assert!(result["shortened"]["how_to_get_everything"].is_string());
    assert_eq!(result["preparation"]["issues_truncated"], true);
    // --full prints every recorded issue example.
    let recipe_path = root.join("recipe.json");
    let full = cli(
        root,
        &[
            "workspace",
            "prepare-ocel",
            &id,
            "--recipe",
            recipe_path.to_str().unwrap(),
            "--full",
        ],
    );
    let full: Value = serde_json::from_slice(&full.stdout).unwrap();
    assert_eq!(full["preparation"]["issues"].as_array().unwrap().len(), 50);
    assert!(full.get("shortened").is_none());
    let state = success(cli(root, &["workspace", "show", &id]));
    assert!(state["source"].is_null());
}
