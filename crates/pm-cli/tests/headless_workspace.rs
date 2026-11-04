//! End-to-end checks for the browser-free project workflow exposed to agents.
use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

struct Fixture(PathBuf);

impl Fixture {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "pm-headless-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Service(Child);

impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn serve(root: &Path) -> (Service, String) {
    let mut child = Service(
        Command::new(env!("CARGO_BIN_EXE_glasstarn"))
            .args(["serve", "--no-open", "--workspace-dir"])
            .arg(root)
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let mut output = BufReader::new(child.0.stdout.take().unwrap());
    loop {
        let mut line = String::new();
        assert!(output.read_line(&mut line).unwrap() > 0);
        if let Some(url) = line.trim().strip_prefix("open ") {
            return (child, url.into());
        }
    }
}

fn post(url: &str, path: &str, body: Value) -> Value {
    let (address, token) = url
        .strip_prefix("http://")
        .unwrap()
        .split_once("/#token=")
        .unwrap();
    let body = body.to_string();
    let mut stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    write!(
        stream,
        "POST {path} HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {token}\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    let mut response = String::new();
    let _ = stream.read_to_string(&mut response);
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap()
}

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_glasstarn"))
        .args(args)
        .output()
        .unwrap()
}

fn success(args: &[&str]) -> Value {
    let output = run(args);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn fixture_source(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata")
        .join(relative)
}

#[test]
fn harness_neutral_queries_preserve_baselines_and_reject_stale_findings() {
    let fixture = Fixture::new("investigation");
    let root = fixture.0.join("projects");
    let root_text = root.to_str().unwrap();
    let created = success(&[
        "workspace",
        "create",
        "--name",
        "Audit",
        "--purpose",
        "audit",
        "--workspace-dir",
        root_text,
    ]);
    let id = created["project"]["id"].as_str().unwrap();
    let initial_request = fixture.0.join("initial-question.json");
    std::fs::write(&initial_request, serde_json::json!({"version":1,"question":"Does the log follow the expected process?","missing_data":["Approval history and the approved process model."]}).to_string()).unwrap();
    let initial = success(&[
        "workspace",
        "record",
        id,
        "--request",
        initial_request.to_str().unwrap(),
        "--workspace-dir",
        root_text,
    ]);
    let initial_note: Value = serde_json::from_slice(
        &std::fs::read(root.join(id).join(initial["artifact"].as_str().unwrap())).unwrap(),
    )
    .unwrap();
    assert!(initial_note["scope"]["source"].is_null());
    let source = fixture_source("canonical/01-sequential.csv");
    success(&[
        "workspace",
        "import",
        id,
        source.to_str().unwrap(),
        "--workspace-dir",
        root_text,
    ]);
    let request = fixture.0.join("request.json");
    let call = |action: &str, value: Value| {
        std::fs::write(&request, value.to_string()).unwrap();
        run(&[
            "workspace",
            action,
            id,
            "--request",
            request.to_str().unwrap(),
            "--workspace-dir",
            root_text,
        ])
    };
    let complete = |action: &str, value: Value| -> Value {
        let output = call(action, value);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    };
    let discovery = complete("query", serde_json::json!({"type":"discover"}));
    let baseline = fixture.0.join("baseline.json");
    std::fs::write(&baseline, discovery["response"]["result"].to_string()).unwrap();
    let replay = complete(
        "query",
        serde_json::json!({"type":"conformance","model":baseline,"algorithm":"token","limit":1}),
    );
    assert_eq!(replay["response"]["result"]["deviating_cases"], 0);
    std::fs::remove_file(&baseline).unwrap();
    let alignment = complete(
        "query",
        serde_json::json!({"type":"conformance","algorithm":"alignment","limit":1}),
    );
    assert_eq!(
        alignment["response"]["scope"]["case_model"],
        replay["response"]["scope"]["case_model"]
    );
    let invalid = fixture.0.join("invalid.json");
    std::fs::write(&invalid, "{}").unwrap();
    assert!(
        !call(
            "query",
            serde_json::json!({"type":"conformance","model":invalid,"algorithm":"token","limit":1})
        )
        .status
        .success()
    );
    let ready = complete("query", serde_json::json!({"type":"readiness"}));
    assert_eq!(ready["response"]["result"]["multiple_event_cases"], 3);
    let map = complete(
        "query",
        serde_json::json!({"type":"map","selection":{"mode":"limits","activities":0,"connections":0}}),
    );
    assert_eq!(map["response"]["result"]["cases_total"], 3);
    assert!(
        map["response"]["result"]["top_route"]
            .as_array()
            .unwrap()
            .len()
            > 1
    );
    let findings = complete("query", serde_json::json!({"type":"findings"}));
    assert!(findings["response"]["result"]["findings"].is_array());
    // This log has no amount field, and the answer says so.
    let no_amounts = complete("query", serde_json::json!({"type":"values"}));
    assert_eq!(
        no_amounts["response"]["result"]["number_fields"],
        serde_json::json!([])
    );
    assert!(
        no_amounts["response"]["result"]["note"]
            .as_str()
            .unwrap()
            .contains("counts and times")
    );
    let routes = complete("query", serde_json::json!({"type":"map"}));
    assert_eq!(
        routes["response"]["result"]["selection"]["variants"],
        serde_json::json!([1])
    );
    let timeline = complete("query", serde_json::json!({"type":"case","id":"ORDER-1"}));
    assert_eq!(
        timeline["response"]["result"]["events"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
    let evidence = complete(
        "query",
        serde_json::json!({"type":"activity","activity":"Receive Order","limit":1}),
    );
    assert_eq!(evidence["response"]["result"]["matching_cases"], 3);
    assert_eq!(evidence["response"]["result"]["truncated"], true);
    let saved = complete(
        "record",
        serde_json::json!({"version":1,"question":"Does the log follow the expected process?","findings":[{"kind":"observed","title":"The recorded process matches the baseline", "statement":"All recorded cases fit this baseline.","evidence":[replay["artifact"]]}]}),
    );
    assert!(
        root.join(id)
            .join(saved["artifact"].as_str().unwrap())
            .is_file()
    );
    assert!(!call("record", serde_json::json!({"version":1,"question":"x","findings":[{"kind":"observed","statement":"Unsupported","evidence":[]}]})).status.success());
    assert!(!call("record", serde_json::json!({"version":1,"question":"x","findings":[{"kind":"observed","statement":"Unsafe path","evidence":["exports/../../request.json"]}]})).status.success());
    assert!(
        !call(
            "query",
            serde_json::json!({"type":"activity","activity":"Receive","limit":101})
        )
        .status
        .success()
    );
    let persisted: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join(id).join(saved["artifact"].as_str().unwrap())).unwrap(),
    )
    .unwrap();
    assert_eq!(
        persisted["note"]["findings"][0]["title"],
        "The recorded process matches the baseline"
    );
    // A consultant's answer: headline, process read and next questions.
    let answered = complete(
        "record",
        serde_json::json!({"version":1,"question":"Health check","answer":"Orders take about a day; credit checks hold most of it.",
            "process":"Order handling; the goal is shipping sooner.","process_type":"logistics","next_questions":["Shall I compare large and small orders?"],
            "findings":[{"kind":"observed","statement":"All recorded cases fit this baseline.","evidence":[replay["artifact"]]}]}),
    );
    let answered: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join(id).join(answered["artifact"].as_str().unwrap())).unwrap(),
    )
    .unwrap();
    assert_eq!(
        answered["note"]["answer"],
        "Orders take about a day; credit checks hold most of it."
    );
    assert_eq!(
        answered["note"]["next_questions"][0],
        "Shall I compare large and small orders?"
    );
    assert_eq!(answered["process_profile"]["time_label"], "Delivery time");
    assert_eq!(answered["process_profile"]["source"], "agent");
    for bad in [
        serde_json::json!({"answer": "x".repeat(401)}),
        serde_json::json!({"process": " "}),
        serde_json::json!({"next_questions": ["a", "b", "c", "d", "e", "f"]}),
    ] {
        let mut request = serde_json::json!({"version":1,"question":"x"});
        request
            .as_object_mut()
            .unwrap()
            .extend(bad.as_object().unwrap().clone());
        assert!(!call("record", request).status.success());
    }
    for title in [" ".to_owned(), "x".repeat(161)] {
        assert!(!call("record", serde_json::json!({"version":1,"question":"x","findings":[{"kind":"hypothesis","title":title,"statement":"Possible explanation","evidence":[]}]})).status.success());
    }
    // Another invocation/harness can reopen the folder and recover its records.
    let shown = success(&["workspace", "show", id, "--workspace-dir", root_text]);
    assert_eq!(shown["investigation_records"][0], initial["artifact"]);
    assert_eq!(shown["investigation_records"][1], saved["artifact"]);
    // Replacing the source leaves earlier notes but disallows presenting old evidence as current.
    let replacement = fixture.0.join("replacement.csv");
    std::fs::write(
        &replacement,
        "case_id,activity,timestamp\nX,A,2026-01-01T09:00:00Z\nX,B,2026-01-01T10:00:00Z\n",
    )
    .unwrap();
    success(&[
        "workspace",
        "import",
        id,
        replacement.to_str().unwrap(),
        "--workspace-dir",
        root_text,
    ]);
    let old = call(
        "record",
        serde_json::json!({"version":1,"question":"x","findings":[{"kind":"observed","statement":"Old","evidence":[evidence["artifact"]]}]}),
    );
    assert!(!old.status.success());
    assert!(String::from_utf8_lossy(&old.stderr).contains("older settings"));
    let snapshot = fixture.0.join("snapshot.csv");
    std::fs::write(&snapshot, "case_id,activity,timestamp\nX,Current,2026-01-01T09:00:00Z\nY,Current,2026-01-01T10:00:00Z\n").unwrap();
    success(&[
        "workspace",
        "import",
        id,
        snapshot.to_str().unwrap(),
        "--workspace-dir",
        root_text,
    ]);
    let readiness = complete("query", serde_json::json!({"type":"readiness"}));
    assert_eq!(
        readiness["response"]["result"]["all_cases_have_one_event"],
        true
    );
    // The saved filter controls evidence, even when the same case exists in the source.
    {
        let mut folder = pm_project::workspace::WorkspaceFolder::open(&root.join(id)).unwrap();
        let mut state = folder.state().unwrap();
        state
            .case_project
            .as_mut()
            .unwrap()
            .view_filter
            .without_activities
            .insert("Current".into());
        folder.save_state(&state).unwrap();
    }
    assert!(
        !call("query", serde_json::json!({"type":"case","id":"X"}))
            .status
            .success()
    );
    assert_eq!(
        complete("query", serde_json::json!({"type":"readiness"}))["response"]["result"]["cases"],
        0
    );
}

#[test]
fn headless_ocel_followups_use_native_objects_and_retained_baseline() {
    let fixture = Fixture::new("ocel-followups");
    let root = fixture.0.join("projects");
    let root_text = root.to_str().unwrap();
    let created = success(&[
        "workspace",
        "create",
        "--name",
        "Objects",
        "--workspace-dir",
        root_text,
    ]);
    let id = created["project"]["id"].as_str().unwrap();
    let source = fixture_source("ocel/order-management.jsonocel");
    success(&[
        "workspace",
        "import",
        id,
        source.to_str().unwrap(),
        "--workspace-dir",
        root_text,
    ]);
    let request = fixture.0.join("request.json");
    let query = |value: Value| {
        std::fs::write(&request, value.to_string()).unwrap();
        success(&[
            "workspace",
            "query",
            id,
            "--request",
            request.to_str().unwrap(),
            "--workspace-dir",
            root_text,
        ])
    };
    let object = query(serde_json::json!({"type":"object","id":"order-1"}));
    assert_eq!(object["response"]["result"]["object"], "order-1");
    let discovery = query(serde_json::json!({"type":"discover"}));
    let model = fixture.0.join("ocel-model.json");
    std::fs::write(&model, discovery["response"]["result"].to_string()).unwrap();
    let replay = query(
        serde_json::json!({"type":"conformance","model":model,"algorithm":"token","limit":1}),
    );
    assert!(
        replay["response"]["result"]["replayed_objects"]
            .as_u64()
            .unwrap()
            > 0
    );
    std::fs::remove_file(model).unwrap();
    let alignment =
        query(serde_json::json!({"type":"conformance","algorithm":"alignment","limit":1}));
    assert_eq!(
        replay["response"]["scope"]["ocel_model"],
        alignment["response"]["scope"]["ocel_model"]
    );
    query(serde_json::json!({"type":"bottlenecks"}));
    // Following an item type gives the ordinary case figures for those items.
    // By default the agent gets the headline; the full analysis on request.
    let followed = query(serde_json::json!({"type":"follow","object_type":"item"}));
    let result = &followed["response"]["result"];
    assert_eq!(result["follow"]["cases"], 2);
    assert_eq!(result["headline"]["summary"]["cases"], 2);
    assert_eq!(result["headline"]["summary"]["events"], 4);
    assert_eq!(result["headline"]["number_fields"][0], "price");
    assert_eq!(result["headline"]["value"]["field"], "price");
    assert!(result.get("analysis").is_none());
    assert_eq!(result["more"]["full_analysis"]["detail"], "full");
    let full = query(serde_json::json!({"type":"follow","object_type":"item","detail":"full"}));
    let result = &full["response"]["result"];
    assert_eq!(result["analysis"]["summary"]["cases"], 2);
    assert_eq!(result["analysis"]["case_attributes"][0], "price");
    assert_eq!(result["analysis"]["number_fields"][0]["name"], "price");
    // Amounts of the followed items, inside a bound.
    let totals = query(
        serde_json::json!({"type":"values","object_type":"item","field":"price",
        "at_least":0,"grouping":{"by":"last_step"}}),
    );
    let overall = &totals["response"]["result"]["overall"];
    assert_eq!(overall["cases"], 2);
    assert_eq!(overall["cases_with_value"], 2);
    std::fs::write(
        &request,
        serde_json::json!({"type":"values","field":"price"}).to_string(),
    )
    .unwrap();
    let missing_type = run(&[
        "workspace",
        "query",
        id,
        "--request",
        request.to_str().unwrap(),
        "--workspace-dir",
        root_text,
    ]);
    assert!(!missing_type.status.success());
    assert!(String::from_utf8_lossy(&missing_type.stderr).contains("object_type"));
    // A business name is display only and survives in the project.
    success(&[
        "workspace",
        "name-type",
        id,
        "--type",
        "item",
        "--name",
        "Line items",
        "--workspace-dir",
        root_text,
    ]);
    let shown = success(&["workspace", "show", id, "--workspace-dir", root_text]);
    assert_eq!(shown["item_names"]["item"], "Line items");
    success(&[
        "workspace",
        "name-type",
        id,
        "--type",
        "item",
        "--clear",
        "--workspace-dir",
        root_text,
    ]);
    let shown = success(&["workspace", "show", id, "--workspace-dir", root_text]);
    assert!(shown["item_names"].as_object().unwrap().is_empty());
}

#[test]
fn agent_can_create_import_reopen_and_reanalyze_without_the_original_file() {
    let fixture = Fixture::new("case");
    let root = fixture.0.join("projects");
    let source = fixture.0.join("orders.csv");
    std::fs::copy(fixture_source("canonical/01-sequential.csv"), &source).unwrap();
    let root_text = root.to_str().unwrap();
    let source_text = source.to_str().unwrap();

    let created = success(&[
        "workspace",
        "create",
        "--name",
        "Order review",
        "--purpose",
        "improvement",
        "--workspace-dir",
        root_text,
        "--json",
    ]);
    let id = created["project"]["id"].as_str().unwrap();
    let imported = success(&[
        "workspace",
        "import",
        id,
        source_text,
        "--case",
        "case_id",
        "--activity",
        "activity",
        "--timestamp",
        "timestamp",
        "--workspace-dir",
        root_text,
        "--json",
    ]);
    assert_eq!(imported["status"], "complete");
    assert_eq!(imported["headline"]["summary"]["cases"], 3);
    assert!(
        imported["full_analysis"]
            .as_str()
            .unwrap()
            .contains("latest-analysis.json")
    );

    std::fs::remove_file(source).unwrap();
    let reopened = success(&[
        "workspace",
        "analyze",
        id,
        "--workspace-dir",
        root_text,
        "--json",
    ]);
    assert_eq!(reopened["headline"]["summary"]["events"], 12);
    assert!(root.join(id).join("exports/latest-analysis.json").is_file());
    assert_eq!(
        success(&["workspace", "list", "--workspace-dir", root_text, "--json",])["projects"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    let (_service, hub) = serve(&root);
    let opened = post(&hub, "/api/workspaces/open", serde_json::json!({"id":id}));
    assert_eq!(opened["project"]["id"], id);
    assert!(opened["url"].as_str().unwrap().contains("#token="));
}

#[test]
fn case_and_ocel_projects_remain_isolated() {
    let fixture = Fixture::new("isolation");
    let root_text = fixture.0.to_str().unwrap();
    let case = success(&[
        "workspace",
        "create",
        "--name",
        "Cases",
        "--workspace-dir",
        root_text,
    ]);
    let objects = success(&[
        "workspace",
        "create",
        "--name",
        "Objects",
        "--purpose",
        "audit",
        "--workspace-dir",
        root_text,
    ]);
    let case_id = case["project"]["id"].as_str().unwrap();
    let object_id = objects["project"]["id"].as_str().unwrap();
    let case_source = fixture_source("canonical/01-sequential.csv");
    let object_source = fixture_source("ocel/order-management.jsonocel");

    success(&[
        "workspace",
        "import",
        case_id,
        case_source.to_str().unwrap(),
        "--case",
        "case_id",
        "--activity",
        "activity",
        "--timestamp",
        "timestamp",
        "--workspace-dir",
        root_text,
    ]);
    let imported = success(&[
        "workspace",
        "import",
        object_id,
        object_source.to_str().unwrap(),
        "--workspace-dir",
        root_text,
    ]);
    assert_eq!(imported["kind"], "ocel");
    assert_eq!(
        success(&["workspace", "show", case_id, "--workspace-dir", root_text,])["data_kind"],
        "case_log"
    );
    assert_eq!(
        success(&["workspace", "show", object_id, "--workspace-dir", root_text,])["data_kind"],
        "ocel"
    );
    assert_ne!(
        std::fs::read(root_text.to_owned() + "/" + case_id + "/workspace.json").unwrap(),
        std::fs::read(root_text.to_owned() + "/" + object_id + "/workspace.json").unwrap()
    );
}

#[test]
fn material_quality_choice_blocks_then_persists_the_answer() {
    let fixture = Fixture::new("decision");
    let root_text = fixture.0.to_str().unwrap();
    let created = success(&[
        "workspace",
        "create",
        "--name",
        "Incomplete history",
        "--workspace-dir",
        root_text,
    ]);
    let id = created["project"]["id"].as_str().unwrap();
    let source = fixture_source("canonical/07-missing-timestamps.csv");
    let common = [
        "workspace",
        "import",
        id,
        source.to_str().unwrap(),
        "--case",
        "case_id",
        "--activity",
        "activity",
        "--timestamp",
        "timestamp",
        "--workspace-dir",
        root_text,
        "--json",
    ];

    let blocked = run(&common);
    assert_eq!(blocked.status.code(), Some(3));
    let blocked: Value = serde_json::from_slice(&blocked.stdout).unwrap();
    assert_eq!(blocked["status"], "blocked");
    assert_eq!(blocked["quality"]["summary"]["open_decisions"], 1);

    let mut answered = common.to_vec();
    answered.extend(["--answer", "blank_timestamps=missing_data"]);
    assert_eq!(success(&answered)["status"], "complete");
    let shown = success(&["workspace", "show", id, "--workspace-dir", root_text]);
    assert_eq!(
        shown["case_project"]["policy"]["blank_timestamps"],
        "missing_data"
    );
}

#[test]
fn agent_can_inventory_prepare_reconcile_and_analyze_multiple_extracts() {
    let fixture = Fixture::new("prepare");
    let root = fixture.0.join("projects");
    let events = fixture.0.join("orders.csv");
    let suppliers = fixture.0.join("suppliers.csv");
    std::fs::write(
        &events,
        "order_id,created_at,approved_at,supplier_id\nO-1,2026-01-01T09:00:00Z,2026-01-01T10:00:00Z,S-1\nO-2,2026-01-02T09:00:00Z,,S-2\n",
    )
    .unwrap();
    std::fs::write(&suppliers, "supplier_id,region\nS-1,North\nS-2,South\n").unwrap();
    let root_text = root.to_str().unwrap();
    let created = success(&[
        "workspace",
        "create",
        "--name",
        "Approval delay",
        "--workspace-dir",
        root_text,
    ]);
    let id = created["project"]["id"].as_str().unwrap();
    let inventoried = success(&[
        "workspace",
        "inventory",
        id,
        events.to_str().unwrap(),
        suppliers.to_str().unwrap(),
        "--workspace-dir",
        root_text,
    ]);
    assert_eq!(inventoried["inputs"][0]["inventory"]["rows"], 2);
    let order_hash = inventoried["inputs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|input| input["asset"]["name"] == "orders.csv")
        .unwrap()["asset"]["sha256"]
        .as_str()
        .unwrap();
    let supplier_hash = inventoried["inputs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|input| input["asset"]["name"] == "suppliers.csv")
        .unwrap()["asset"]["sha256"]
        .as_str()
        .unwrap();
    let recipe = fixture.0.join("recipe.json");
    std::fs::write(
        &recipe,
        serde_json::to_vec_pretty(&serde_json::json!({
            "version":1,
            "inputs":[
                {"id":"orders","sha256":order_hash},
                {"id":"suppliers","sha256":supplier_hash}
            ],
            "events":[
                {"input":"orders","case_id":"order_id","activity":{"value":"Order created"},"timestamp":"created_at"},
                {"input":"orders","case_id":"order_id","activity":{"value":"Order approved"},"timestamp":"approved_at","optional_timestamp":true}
            ],
            "lookups":[{
                "event_input":"orders","event_key":"supplier_id","input":"suppliers","lookup_key":"supplier_id",
                "fields":[{"column":"region","as":"supplier_region"}]
            }]
        }))
        .unwrap(),
    )
    .unwrap();
    let prepared = success(&[
        "workspace",
        "prepare",
        id,
        "--recipe",
        recipe.to_str().unwrap(),
        "--workspace-dir",
        root_text,
    ]);
    assert_eq!(prepared["status"], "complete");
    assert_eq!(prepared["preparation"]["output_rows"], 3);
    assert_eq!(prepared["preparation"]["output_cases"], 2);
    assert_eq!(
        prepared["preparation"]["event_rules"][1]["optional_timestamps_blank"],
        1
    );
    assert_eq!(prepared["preparation"]["lookups"][0]["unmatched_events"], 0);
    assert_eq!(prepared["analysis"]["headline"]["summary"]["events"], 3);
    let folder = root.join(id);
    assert!(folder.join("preparation/recipe.json").is_file());
    assert!(folder.join("preparation/validation.json").is_file());
    assert!(folder.join("exports/latest-analysis.json").is_file());
    assert_eq!(
        success(&["workspace", "show", id, "--workspace-dir", root_text])["data_kind"],
        "case_log"
    );
}

#[test]
fn preparation_blocks_an_ambiguous_many_to_one_lookup() {
    let fixture = Fixture::new("prepare-blocked");
    let root = fixture.0.join("projects");
    let events = fixture.0.join("events.csv");
    let lookup = fixture.0.join("lookup.csv");
    std::fs::write(
        &events,
        "case,event,time,owner\nC-1,Opened,2026-01-01T09:00:00Z,A\nC-1,Opened,2026-01-01T09:00:00Z,A\n",
    )
    .unwrap();
    std::fs::write(&lookup, "owner,team\nA,East\nA,West\n").unwrap();
    let root_text = root.to_str().unwrap();
    let created = success(&[
        "workspace",
        "create",
        "--name",
        "Join check",
        "--workspace-dir",
        root_text,
    ]);
    let id = created["project"]["id"].as_str().unwrap();
    let inventory = success(&[
        "workspace",
        "inventory",
        id,
        events.to_str().unwrap(),
        lookup.to_str().unwrap(),
        "--workspace-dir",
        root_text,
    ]);
    let hash = |name: &str| {
        inventory["inputs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|input| input["asset"]["name"] == name)
            .unwrap()["asset"]["sha256"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let recipe = fixture.0.join("recipe.json");
    std::fs::write(
        &recipe,
        serde_json::to_vec_pretty(&serde_json::json!({
            "version":1,
            "inputs":[
                {"id":"events","sha256":hash("events.csv")},
                {"id":"owners","sha256":hash("lookup.csv")}
            ],
            "events":[{"input":"events","case_id":"case","activity":{"column":"event"},"timestamp":"time"}],
            "lookups":[{
                "event_input":"events","event_key":"owner","input":"owners","lookup_key":"owner",
                "fields":[{"column":"team","as":"team"}]
            }]
        }))
        .unwrap(),
    )
    .unwrap();
    let output = run(&[
        "workspace",
        "prepare",
        id,
        "--recipe",
        recipe.to_str().unwrap(),
        "--workspace-dir",
        root_text,
    ]);
    assert_eq!(output.status.code(), Some(3));
    let blocked: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(blocked["status"], "blocked");
    assert_eq!(blocked["preparation"]["lookups"][0]["duplicate_keys"], 1);
    assert_eq!(blocked["preparation"]["exact_duplicate_events"], 1);
    assert_eq!(
        success(&["workspace", "show", id, "--workspace-dir", root_text])["data_kind"],
        "none"
    );
}

#[test]
fn window_labels_cases_without_dropping_them() {
    let fixture = Fixture::new("window");
    let root = fixture.0.join("projects");
    let source = fixture.0.join("orders.csv");
    std::fs::copy(fixture_source("canonical/01-sequential.csv"), &source).unwrap();
    let root_text = root.to_str().unwrap();
    let created = success(&[
        "workspace",
        "create",
        "--name",
        "Window",
        "--workspace-dir",
        root_text,
        "--json",
    ]);
    let id = created["project"]["id"].as_str().unwrap();
    success(&[
        "workspace",
        "import",
        id,
        source.to_str().unwrap(),
        "--case",
        "case_id",
        "--activity",
        "activity",
        "--timestamp",
        "timestamp",
        "--workspace-dir",
        root_text,
        "--json",
    ]);
    // ORDER-1 starts at 09:00, before the window opens at 09:30.
    let saved = success(&[
        "workspace",
        "window",
        id,
        "--start",
        "2026-03-02T09:30:00Z",
        "--end",
        "2026-03-05T00:00:00Z",
        "--end-step",
        "Ship Order",
        "--workspace-dir",
        root_text,
    ]);
    assert_eq!(
        saved["observation_window"]["end_activities"][0],
        "Ship Order"
    );
    let analysis = &success(&[
        "workspace",
        "analyze",
        id,
        "--workspace-dir",
        root_text,
        "--json",
    ])["headline"];
    assert_eq!(analysis["summary"]["cases"], 3, "no case is dropped");
    let window = &analysis["window"];
    assert_eq!(
        (
            &window["complete"],
            &window["started_before"],
            &window["still_open"]
        ),
        (
            &serde_json::json!(2),
            &serde_json::json!(1),
            &serde_json::json!(0)
        )
    );
    assert_eq!(window["end_defined"], true);
}

/// The Salesforce approval demo, end to end: an agent files the extract, a
/// person approves it in the app, the app downloads through the stand-in CLI,
/// the agent prepares the connected log, and the project answers with the
/// same journey and amounts a person sees.
#[cfg(unix)]
#[test]
fn salesforce_approval_demo_runs_from_request_to_journey() {
    let demo =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/salesforce-approval-demo");
    let fixture = Fixture::new("salesforce-demo");
    let root = fixture.0.join("projects");
    let root_text = root.to_str().unwrap();
    let created = success(&[
        "workspace",
        "create",
        "--name",
        "Salesforce approval demo",
        "--workspace-dir",
        root_text,
    ]);
    let id = created["project"]["id"].as_str().unwrap().to_owned();
    let erp = ["orders", "invoices", "payments", "payment_invoices"].map(|name| {
        demo.join("erp")
            .join(format!("{name}.csv"))
            .display()
            .to_string()
    });
    let mut inventory = vec!["workspace", "inventory", id.as_str()];
    inventory.extend(erp.iter().map(String::as_str));
    inventory.extend(["--workspace-dir", root_text]);
    success(&inventory);
    let spec = demo.join("spec.json");
    let filed = success(&[
        "extract",
        "salesforce",
        &id,
        "--spec",
        spec.to_str().unwrap(),
        "--workspace-dir",
        root_text,
    ]);
    let sha = filed["spec_sha256"].as_str().unwrap().to_owned();

    // The app finds the stand-in `sf` on its PATH, as it would the real one.
    let path = format!(
        "{}:{}",
        demo.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mut child = Service(
        Command::new(env!("CARGO_BIN_EXE_glasstarn"))
            .args(["serve", "--no-open", "--workspace-dir"])
            .arg(&root)
            .env("PATH", path)
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let mut output = BufReader::new(child.0.stdout.take().unwrap());
    let hub = loop {
        let mut line = String::new();
        assert!(output.read_line(&mut line).unwrap() > 0);
        if let Some(url) = line.trim().strip_prefix("open ") {
            break url.to_owned();
        }
    };
    let opened = post(&hub, "/api/workspaces/open", serde_json::json!({"id":id}));
    let project = opened["url"].as_str().unwrap().to_owned();
    let approved = post(
        &project,
        "/api/workspace/extracts/approve",
        serde_json::json!({"spec_sha256":sha,"person":"Demo Reviewer"}),
    );
    assert_eq!(approved["request"]["approved_by"], "Demo Reviewer");
    let mut status = Value::Null;
    for _ in 0..300 {
        status = success(&["extract", "status", &id, "--workspace-dir", root_text]);
        if status["requests"][0]["status"] != "running" {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let request = &status["requests"][0];
    assert_eq!(request["status"], "complete", "{request}");
    assert_eq!(request["result"]["row_counts"][0]["rows"], 334);
    // The app holds the project while it is open; the agent prepares after.
    drop(child);

    let state = success(&["workspace", "show", &id, "--workspace-dir", root_text]);
    let sources = state["preparation_sources"].as_array().unwrap();
    let mut recipe = std::fs::read_to_string(demo.join("recipe.template.json")).unwrap();
    for source in sources {
        recipe = recipe.replace(
            &format!("<{}>", source["name"].as_str().unwrap()),
            source["sha256"].as_str().unwrap(),
        );
    }
    assert!(!recipe.contains('<'), "every input has a source");
    let recipe_path = fixture.0.join("recipe.json");
    std::fs::write(&recipe_path, recipe).unwrap();
    success(&[
        "workspace",
        "prepare-ocel",
        &id,
        "--recipe",
        recipe_path.to_str().unwrap(),
        "--json",
        "--workspace-dir",
        root_text,
    ]);

    let request_path = fixture.0.join("query.json");
    let query = |value: Value| {
        std::fs::write(&request_path, value.to_string()).unwrap();
        success(&[
            "workspace",
            "query",
            &id,
            "--request",
            request_path.to_str().unwrap(),
            "--workspace-dir",
            root_text,
        ])["response"]["result"]
            .clone()
    };
    let journey = query(serde_json::json!({"type":"journey","object_type":"opportunity"}));
    let stages = journey["stages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|stage| stage["activity"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        stages,
        [
            "Deal created",
            "Order created",
            "Invoice issued",
            "Payment recorded"
        ]
    );
    let unpaid = &journey["handoffs"][2];
    assert_eq!(unpaid["not_reached_value"]["field"], "invoice_amount");
    assert!(unpaid["started"].as_u64() > unpaid["reached"].as_u64());
    let followed = query(serde_json::json!({"type":"follow","object_type":"opportunity"}));
    assert_eq!(followed["headline"]["summary"]["cases"], 334);
    assert_eq!(followed["headline"]["routes"].as_array().unwrap().len(), 2);
    assert_eq!(followed["headline"]["value"]["field"], "amount");
    // The headline stays small enough to hand an agent whole.
    assert!(
        followed.to_string().len() < 4096,
        "{}",
        followed.to_string().len()
    );
    // Ranked calculated findings, with the item name filled in and the
    // unpaid invoices among them.
    let findings = query(serde_json::json!({"type":"findings","object_type":"opportunity"}));
    let list = findings["findings"].as_array().unwrap();
    assert!(!list.is_empty() && list.len() <= 8);
    assert!(
        list.iter()
            .all(|f| !f["title"].as_str().unwrap().contains("{items}"))
    );
    assert!(
        list.iter().any(|f| f["kind"] == "drop_out"
            && f["title"].as_str().unwrap().contains("invoice")
            && f["detail"].as_str().unwrap().contains("invoice_amount")),
        "{findings}"
    );
    assert!(followed["headline"]["findings"].as_array().unwrap().len() <= 3);
    let value =
        query(serde_json::json!({"type":"values","object_type":"opportunity","field":"amount"}));
    assert_eq!(value["overall"]["cases_with_value"], 334);
}

#[test]
fn agent_charts_publish_while_browser_owns_settings_and_reject_invented_rows() {
    let fixture = Fixture::new("live-answer");
    let root = fixture.0.join("projects");
    let root_text = root.to_str().unwrap();
    let created = success(&[
        "workspace",
        "create",
        "--name",
        "Charts",
        "--purpose",
        "improvement",
        "--workspace-dir",
        root_text,
    ]);
    let id = created["project"]["id"].as_str().unwrap();
    let source = fixture.0.join("invoices.csv");
    std::fs::write(&source,"case_id,activity,timestamp,customer,amount\nI1,Issue,2026-01-01T00:00:00Z,ABC,100\nI1,Pay,2026-01-02T00:00:00Z,ABC,100\nI2,Issue,2026-01-01T00:00:00Z,ABC,20\nI3,Issue,2026-01-01T00:00:00Z,XYZ,80\n").unwrap();
    success(&[
        "workspace",
        "import",
        id,
        source.to_str().unwrap(),
        "--case-attribute",
        "customer",
        "--case-attribute",
        "amount",
        "--workspace-dir",
        root_text,
    ]);
    let owner = pm_project::workspace::WorkspaceFolder::open(&root.join(id)).unwrap();
    let request = fixture.0.join("request.json");
    let invoke = |action: &str, value: Value| {
        std::fs::write(&request, value.to_string()).unwrap();
        run(&[
            "workspace",
            action,
            id,
            "--request",
            request.to_str().unwrap(),
            "--workspace-dir",
            root_text,
        ])
    };
    let query = invoke(
        "query",
        serde_json::json!({"type":"values","field":"amount","grouping":{"by":"attribute","attribute":"customer"},"limit":10}),
    );
    assert!(
        query.status.success(),
        "{}",
        String::from_utf8_lossy(&query.stderr)
    );
    let query: Value = serde_json::from_slice(&query.stdout).unwrap();
    let record = invoke(
        "record",
        serde_json::json!({"version":1,"question":"Amount by customer?","visuals":[{"title":"Amount by customer","evidence":query["artifact"],"kind":"bar","metric":"total"}]}),
    );
    assert!(
        record.status.success(),
        "{}",
        String::from_utf8_lossy(&record.stderr)
    );
    let record: Value = serde_json::from_slice(&record.stdout).unwrap();
    let saved: Value = serde_json::from_slice(
        &std::fs::read(root.join(id).join(record["artifact"].as_str().unwrap())).unwrap(),
    )
    .unwrap();
    assert_eq!(saved["visuals"][0]["rows"][0]["label"], "ABC");
    assert_eq!(saved["visuals"][0]["rows"][0]["value"], 120.0);
    assert_eq!(saved["visuals"][0]["rows"][0]["example_case"], "I1");
    assert!(!invoke("record",serde_json::json!({"version":1,"question":"fake?","visuals":[{"title":"Fake","evidence":query["artifact"],"kind":"bar","metric":"total","rows":[{"value":999}]}]})).status.success());
    assert!(owner.settings_are_current().unwrap());
    drop(owner);
}

#[test]
fn working_process_keeps_manual_evidence_revisions_and_intent_without_changing_history() {
    let fixture = Fixture::new("consultant-process");
    let root = fixture.0.join("projects");
    let root_text = root.to_str().unwrap();
    let created = success(&[
        "workspace",
        "create",
        "--name",
        "Invoice understanding",
        "--purpose",
        "improvement",
        "--workspace-dir",
        root_text,
    ]);
    let id = created["project"]["id"].as_str().unwrap();
    let source = fixture.0.join("invoices.csv");
    std::fs::write(&source,"case_id,activity,timestamp\nI1,Issue invoice,2026-01-01T00:00:00Z\nI1,Pay invoice,2026-01-10T00:00:00Z\n").unwrap();
    success(&[
        "workspace",
        "import",
        id,
        source.to_str().unwrap(),
        "--workspace-dir",
        root_text,
    ]);
    let mut owner = pm_project::workspace::WorkspaceFolder::open(&root.join(id)).unwrap();
    let original = owner.manifest.state.clone();
    let request = fixture.0.join("request.json");
    let invoke = |action: &str, value: Value| {
        std::fs::write(&request, value.to_string()).unwrap();
        run(&[
            "workspace",
            action,
            id,
            "--request",
            request.to_str().unwrap(),
            "--workspace-dir",
            root_text,
        ])
    };
    let call = |action: &str, value: Value| -> Value {
        let result = invoke(action, value);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        serde_json::from_slice(&result.stdout).unwrap()
    };
    let map = call(
        "query",
        serde_json::json!({"type":"map","selection":{"mode":"routes","variants":[1]}}),
    );
    assert_eq!(
        map["response"]["result"]["edges"][0]["mean_seconds"], 777600,
        "the recorded interval remains nine days"
    );
    let mut model: Value = serde_json::from_str(include_str!(
        "../../../testdata/consultant/invoice-process.json"
    ))
    .unwrap();
    for step in model["steps"].as_array_mut().unwrap() {
        for item in step["evidence"].as_array_mut().unwrap() {
            if item["kind"] == "observed" {
                item["query"] = map["artifact"].clone();
            }
        }
    }
    let first = call(
        "record",
        serde_json::json!({"version":1,"question":"How does invoice handling work?","intention":"understand","process_model":model}),
    );
    let read = |reference: &Value| -> Value {
        serde_json::from_slice(
            &std::fs::read(root.join(id).join(reference.as_str().unwrap())).unwrap(),
        )
        .unwrap()
    };
    let saved = read(&first["artifact"]);
    assert_eq!(saved["process_model"]["steps"].as_array().unwrap().len(), 3);
    assert_eq!(
        saved["supporting_evidence"][map["artifact"].as_str().unwrap()]["result"]["nodes"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(saved["intention"], "understand");
    let followup = call(
        "record",
        serde_json::json!({"version":1,"question":"Now let's improve it.","intention":"improve"}),
    );
    let continued = read(&followup["artifact"]);
    assert_eq!(continued["process_model_origin"], first["artifact"]);
    assert_eq!(continued["intention"], "improve");
    assert_eq!(continued["process_model"], saved["process_model"]);
    assert_eq!(continued["current_process_model"], saved["process_model"]);
    // Suggestions stay in the conversation: a proposal, a future process or a
    // step treatment is refused, and nothing is saved.
    let mut future = continued["process_model"].clone();
    future["view"] = serde_json::json!("proposed_future");
    assert!(
        !invoke(
            "record",
            serde_json::json!({"version":1,"question":"Redesign it.","process_model":future})
        )
        .status
        .success()
    );
    let mut treated = continued["process_model"].clone();
    treated["steps"][1]["assessment"] =
        serde_json::json!({"treatment":"human_led","rationale":"Keep it with a person."});
    assert!(
        !invoke(
            "record",
            serde_json::json!({"version":1,"question":"Who should do it?","process_model":treated})
        )
        .status
        .success()
    );
    // A revision with new facts (what someone reported) is kept, and earlier
    // revisions stay as they were.
    let mut reported = continued["process_model"].clone();
    reported["open_questions"] =
        serde_json::json!(["Email access is unavailable to the finance team."]);
    let revised = call(
        "record",
        serde_json::json!({"version":1,"question":"We cannot grant email access.","process_model":reported}),
    );
    assert_eq!(
        read(&revised["artifact"])["process_model"]["supersedes"],
        first["artifact"]
    );
    assert_eq!(
        read(&first["artifact"]),
        saved,
        "earlier revisions stay immutable"
    );
    assert_eq!(owner.manifest.state, original);
    assert!(owner.settings_are_current().unwrap());
    let cleared = call(
        "record",
        serde_json::json!({"version":1,"question":"Withdraw the working proposal.","clear_process_model":true}),
    );
    assert!(read(&cleared["artifact"])["process_model"].is_null());
    assert!(read(&cleared["artifact"])["current_process_model"].is_null());
    let after = call(
        "record",
        serde_json::json!({"version":1,"question":"Continue from the recorded history."}),
    );
    assert!(read(&after["artifact"])["process_model"].is_null());
    let mut invalid = model.clone();
    invalid["steps"][1]["duration_seconds"] = serde_json::json!(60);
    assert!(!invoke("record",serde_json::json!({"version":1,"question":"Invent manual timing?","process_model":invalid})).status.success());
    call(
        "record",
        serde_json::json!({"version":1,"question":"Restore the working understanding.","process_model":model}),
    );
    let mut changed = owner.state().unwrap();
    changed
        .case_project
        .as_mut()
        .unwrap()
        .view_filter
        .with_activities = ["Pay invoice".into()].into_iter().collect();
    owner.save_state(&changed).unwrap();
    let narrowed = call(
        "record",
        serde_json::json!({"version":1,"question":"Continue with the narrower population."}),
    );
    assert!(
        read(&narrowed["artifact"])["process_model"].is_null(),
        "changed scope never inherits the old model"
    );
    assert!(!invoke("record",serde_json::json!({"version":1,"question":"Reuse stale evidence?","process_model":model})).status.success());
}
