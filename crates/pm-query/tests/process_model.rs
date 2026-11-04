//! Fixed synthetic SOP/interview facts, separate from recorded invoice history.
use pm_query::process_model::ProcessModel;
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn fixture() -> Value {
    serde_json::from_str(include_str!(
        "../../../testdata/consultant/invoice-process.json"
    ))
    .unwrap()
}
fn snapshots() -> BTreeMap<String, Value> {
    BTreeMap::from([(
        "exports/query-1.json".into(),
        json!({
            "request":{"type":"map"},"result":{
                "nodes":[{"activity":"Issue invoice"},{"activity":"Pay invoice"}],
                "edges":[{"from":"Issue invoice","to":"Pay invoice"}]
            }
        }),
    )])
}
fn validate(value: Value) -> Result<(), String> {
    serde_json::from_value::<ProcessModel>(value)
        .map_err(|e| e.to_string())?
        .validate(&snapshots())
}
#[test]
fn documents_and_reported_work_supplement_history_without_becoming_events() {
    let value = fixture();
    validate(value.clone()).unwrap();
    let model: ProcessModel = serde_json::from_value(value).unwrap();
    assert_eq!(model.steps.len(), 3);
    assert_eq!(model.queries().count(), 2);
    assert_eq!(model.open_questions.len(), 2);
    assert!(
        !serde_json::to_string(&model)
            .unwrap()
            .contains("duration_seconds")
    );
}
#[test]
fn manual_steps_and_connections_cannot_claim_absent_recorded_evidence() {
    let mut step = fixture();
    step["steps"][1]["evidence"] = json!([{"kind":"observed","query":"exports/query-1.json"}]);
    assert!(validate(step).unwrap_err().contains("absent"));
    let mut edge = fixture();
    edge["connections"][0]["evidence"] =
        json!([{"kind":"observed","query":"exports/query-1.json"}]);
    assert!(validate(edge).unwrap_err().contains("absent"));
}
#[test]
fn rejects_fabricated_measurements_broken_sources_and_unbounded_execution_claims() {
    let mut measured = fixture();
    measured["steps"][1]["duration_seconds"] = json!(600);
    assert!(validate(measured).unwrap_err().contains("unknown field"));
    let mut source = fixture();
    source["steps"][1]["evidence"][0]["source"] = json!("interview");
    assert!(validate(source).unwrap_err().contains("document source"));
    let mut edge = fixture();
    edge["connections"][0]["to"] = json!("missing");
    assert!(validate(edge).unwrap_err().contains("destination"));
}

#[test]
fn followed_histories_support_recorded_steps_without_claiming_transaction_links() {
    let mut model = fixture();
    model["connections"] = json!([{"from":"issue","to":"pay","evidence":[{"kind":"observed","query":"exports/query-1.json"}]}]);
    let evidence = BTreeMap::from([(
        "exports/query-1.json".into(),
        json!({"request":{"type":"follow","detail":"full"},"result":{"analysis":{"mapping":{"object_history":true},"nodes":[{"activity":"Issue invoice"},{"activity":"Pay invoice"}],"edges":[{"from":"Issue invoice","to":"Pay invoice"}]}}}),
    )]);
    let typed: ProcessModel = serde_json::from_value(model.clone()).unwrap();
    assert!(
        typed
            .validate(&evidence)
            .unwrap_err()
            .contains("object-history adjacency")
    );
    model["connections"] = json!([]);
    serde_json::from_value::<ProcessModel>(model)
        .unwrap()
        .validate(&evidence)
        .unwrap();
}
