//! "How this data was built": the project's own evidence, gathered for the
//! person checking it. Extraction reports, the connected-preparation recipe
//! and its reconciliation counts are read from the project folder as saved;
//! nothing here is recalculated or inferred.
use super::*;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::Path;

/// Bounded read of a saved evidence file; anything unreadable is skipped.
fn evidence(path: &Path) -> Option<Value> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > 4 * 1024 * 1024 {
        return None;
    }
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

/// Run folders under `dir` whose names start with `prefix`, newest first.
fn runs(dir: &Path, prefix: &str) -> Vec<std::path::PathBuf> {
    let mut found = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().starts_with(prefix))
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| Some((entry.metadata().ok()?.modified().ok()?, entry.path())))
        .collect::<Vec<_>>();
    found.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    found.into_iter().map(|(_, path)| path).collect()
}

pub(super) fn provenance(root: &Path) -> Value {
    let extractions = runs(&root.join("extraction"), "extract-")
        .into_iter()
        .filter_map(|run| evidence(&run.join("report.json")))
        .map(|report| {
            json!({"system":report["system"],"connection":report["target_org"],
                "window":report["requested_range"],"approved_by":report["approved_by"],
                "approved_at":report["approved_at"],
                "objects":report["objects"].as_array().into_iter().flatten()
                    .map(|object| json!({"name":object["object"],"rows":object["rows"],"rows_before_window":object["rows_before_window"]}))
                    .collect::<Vec<_>>()})
        })
        .collect::<Vec<_>>();
    let preparation = runs(&root.join("preparation"), "ocel-preparation-")
        .into_iter()
        .next()
        .and_then(|run| {
            Some((
                evidence(&run.join("recipe.json"))?,
                evidence(&run.join("report.json"))?,
            ))
        })
        .map(|(recipe, report)| connected_preparation(&recipe, &report));
    json!({"extractions":extractions,"preparation":preparation,"packs":packs(&root.join("preparation")),
        "audit_log":root.join("audit/log.jsonl").is_file()})
}

/// Process packs used in the project, with every saved answer: the question
/// as asked, the option chosen in words, any value, who confirmed it and when.
fn packs(dir: &Path) -> Vec<Value> {
    let mut found = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let id = name
                .strip_prefix("pack-")?
                .strip_suffix("-check.json")?
                .to_owned();
            Some(id)
        })
        .collect::<Vec<_>>();
    found.sort();
    found
        .into_iter()
        .filter_map(|id| {
            let check = evidence(&dir.join(format!("pack-{id}-check.json")))?;
            let answers = evidence(&dir.join(format!("pack-{id}-answers.json")))
                .map(|saved| saved["answers"].clone())
                .unwrap_or(Value::Null);
            let questions = check["report"]["questions"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            let answers = answers
                .as_object()
                .into_iter()
                .flatten()
                .map(|(question, answer)| {
                    let chosen = questions
                        .iter()
                        .find(|q| q["id"] == question.as_str())
                        .and_then(|q| q["options"].as_array())
                        .and_then(|options| options.iter().find(|o| o["id"] == answer["choice"]))
                        .and_then(|o| o["label"].as_str())
                        .unwrap_or_else(|| answer["choice"].as_str().unwrap_or_default());
                    json!({"question":answer["question"],"answer":chosen,"value":answer["value"],
                        "confirmed_by":answer["confirmed_by"],"at":answer["at"]})
                })
                .collect::<Vec<_>>();
            Some(
                json!({"id":id,"status":check["pack_status"],"answers":answers,
                "open_questions":questions.len().saturating_sub(answers.len())}),
            )
        })
        .collect()
}

/// Key columns in words: a whole column, or a character range of one.
fn columns(value: &Value) -> Value {
    Value::Array(
        value
            .as_array()
            .into_iter()
            .flatten()
            .map(|part| match part {
                Value::Object(slice) => json!(format!(
                    "{} characters {} to {}",
                    slice.get("column").and_then(Value::as_str).unwrap_or("?"),
                    slice.get("start").and_then(Value::as_u64).unwrap_or(0) + 1,
                    slice.get("start").and_then(Value::as_u64).unwrap_or(0)
                        + slice.get("length").and_then(Value::as_u64).unwrap_or(0)
                )),
                other => other.clone(),
            })
            .collect(),
    )
}

/// Tables, the links between item types with the columns they match on, and
/// how many rows matched, from a connected-preparation recipe and report.
fn connected_preparation(recipe: &Value, report: &Value) -> Value {
    let rules = &report["rules"];
    let object_types = recipe["objects"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|rule| {
            (
                rule["id"].as_str().unwrap_or_default(),
                rule["object_type"].clone(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let type_of = |reference: &Value| {
        object_types
            .get(reference["object"].as_str().unwrap_or_default())
            .cloned()
            .unwrap_or(Value::Null)
    };
    let links = recipe["relationships"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|rule| {
            let counts = &rules[rule["id"].as_str().unwrap_or_default()];
            json!({"from":type_of(&rule["source"]),"to":type_of(&rule["target"]),
                "on":columns(&rule["target"]["columns"]),"meaning":rule["target"]["qualifier"],
                "matched":counts["emitted"],"unmatched":counts["unmatched"]})
        })
        .collect::<Vec<_>>();
    let steps = recipe["events"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|rule| {
            let counts = &rules[rule["id"].as_str().unwrap_or_default()];
            json!({"table":rule["input"],"activity":rule["activity"]["value"].as_str().map_or_else(|| json!(format!("from column {}", rule["activity"]["column"].as_str().unwrap_or("?"))), |value| json!(value)),
                "items":rule["objects"].as_array().into_iter().flatten().map(|reference| json!({"type":type_of(reference),"on":columns(&reference["columns"])})).collect::<Vec<_>>(),
                "rows":counts["rows"],"emitted":counts["emitted"],"unmatched":counts["unmatched"]})
        })
        .collect::<Vec<_>>();
    json!({"tables":recipe["inputs"].as_array().into_iter().flatten().map(|input| input["id"].clone()).collect::<Vec<_>>(),
        "links":links,"steps":steps,
        "totals":{"items":report["objects"],"steps":report["events"],"item_links":report["object_relationships"],"step_links":report["event_relationships"],"issues":report["issue_count"]}})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_extractions_and_the_matching_rules_as_saved() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let extract = root.join("extraction/extract-a");
        fs::create_dir_all(&extract).unwrap();
        fs::write(extract.join("report.json"), json!({"system":"salesforce","target_org":"org","approved_by":"Pat",
            "approved_at":"2026-10-10T00:00:00Z","requested_range":{"start":"2026-01-01T00:00:00Z","end_exclusive":"2026-02-01T00:00:00Z"},
            "objects":[{"object":"Opportunity","rows":2,"rows_before_window":1}]}).to_string()).unwrap();
        let run = root.join("preparation/ocel-preparation-b");
        fs::create_dir_all(&run).unwrap();
        fs::write(run.join("recipe.json"), json!({"inputs":[{"id":"orders"},{"id":"deals"}],
            "objects":[{"id":"orders","object_type":"order"},{"id":"deals","object_type":"opportunity"}],
            "events":[{"id":"created","input":"orders","activity":{"value":"Order created"},"objects":[{"object":"orders","columns":["company","number"]}]}],
            "relationships":[{"id":"order-deal","source":{"object":"orders"},"target":{"object":"deals","columns":["deal_id"],"qualifier":"from deal"}}]}).to_string()).unwrap();
        fs::write(run.join("report.json"), json!({"rules":{"order-deal":{"emitted":3,"unmatched":0},"created":{"rows":3,"emitted":3,"unmatched":0}},
            "objects":4,"events":3,"object_relationships":3,"event_relationships":3,"issue_count":0}).to_string()).unwrap();
        let found = provenance(root);
        assert_eq!(found["extractions"][0]["approved_by"], "Pat");
        assert_eq!(
            found["extractions"][0]["objects"][0]["rows_before_window"],
            1
        );
        let link = &found["preparation"]["links"][0];
        assert_eq!(
            (
                link["from"].as_str(),
                link["to"].as_str(),
                link["matched"].as_u64()
            ),
            (Some("order"), Some("opportunity"), Some(3))
        );
        assert_eq!(link["on"][0], "deal_id");
        assert_eq!(
            found["preparation"]["steps"][0]["items"][0]["type"],
            "order"
        );
        assert_eq!(found["preparation"]["totals"]["issues"], 0);
        assert_eq!(found["packs"], json!([]));
        assert_eq!(found["audit_log"], false);
    }

    #[test]
    fn lists_pack_answers_in_words_and_key_ranges_as_text() {
        let dir = tempfile::tempdir().unwrap();
        let prep = dir.path().join("preparation");
        fs::create_dir_all(&prep).unwrap();
        fs::write(
            prep.join("pack-p2p-check.json"),
            json!({"pack_status":"untested","report":{"questions":[
            {"id":"custom:EKKO.ZZ","options":[{"id":"step","label":"Yes, show it as a step"}]},
            {"id":"setting:offset","options":[{"id":"value","label":"Give the value"}]}]}})
            .to_string(),
        )
        .unwrap();
        fs::write(prep.join("pack-p2p-answers.json"), json!({"answers":{"custom:EKKO.ZZ":{"choice":"step",
            "value":"Approve order","confirmed_by":"Analyst","at":"2026-10-12T00:00:00Z","question":"Is it a step?"}}}).to_string()).unwrap();
        let found = provenance(dir.path());
        let pack = &found["packs"][0];
        assert_eq!(pack["status"], "untested");
        assert_eq!(pack["open_questions"], 1);
        assert_eq!(pack["answers"][0]["answer"], "Yes, show it as a step");
        assert_eq!(pack["answers"][0]["confirmed_by"], "Analyst");
        assert_eq!(
            columns(&json!(["BUKRS", {"column":"AWKEY","start":10,"length":4}])),
            json!(["BUKRS", "AWKEY characters 11 to 14"])
        );
    }
}
