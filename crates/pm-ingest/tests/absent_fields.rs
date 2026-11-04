//! Every optional field of every built-in pack can be missing from an
//! export without stopping the build: the recipe then never asks for it, and
//! whatever needed it is left out and named.
use pm_ingest::pack::{Answer, BUILT_IN, Exported, Pack, build_recipe_with};
use serde_json::Value;
use std::collections::BTreeMap;

/// Whether `rule` (reading `table`, or looking it up) still names `field`.
fn names(rule: &Value, table: &str, field: &str) -> bool {
    let reads = rule["input"] == table;
    let mut found = false;
    for key in ["lookup", "lookups"] {
        let lookups = match &rule[key] {
            Value::Array(items) => items.iter().collect::<Vec<_>>(),
            lookup @ Value::Object(_) => vec![lookup],
            _ => Vec::new(),
        };
        for lookup in lookups {
            if lookup["input"] == table {
                found |= ["key", "matching", "columns"].iter().any(|k| {
                    lookup[*k]
                        .as_array()
                        .is_some_and(|a| a.iter().any(|v| v == field))
                });
            }
        }
    }
    if !reads {
        return found;
    }
    fn walk(value: &Value, field: &str, skip_lookups: bool) -> bool {
        match value {
            Value::Array(items) => items.iter().any(|v| walk(v, field, skip_lookups)),
            Value::Object(map) => map.iter().any(|(k, v)| {
                if skip_lookups && (k == "lookup" || k == "lookups") {
                    return false;
                }
                match k.as_str() {
                    "id" | "input" | "namespace" | "object_type" | "qualifier" | "object"
                    | "event" | "name" => false,
                    "column" | "date" | "time" | "timestamp" => v == field || walk(v, field, false),
                    "columns" | "key" | "matching" | "event_key" => v
                        .as_array()
                        .is_some_and(|a| a.iter().any(|x| x == field || walk(x, field, false))),
                    _ => walk(v, field, false),
                }
            }),
            _ => false,
        }
    }
    found || walk(rule, field, true)
}

#[test]
fn every_optional_field_can_be_missing() {
    let mut checked = 0;
    for (id, text) in BUILT_IN {
        let pack = Pack::parse(text).unwrap();
        let answers = pack
            .settings
            .iter()
            .map(|s| {
                (
                    format!("setting:{}", s.id),
                    Answer {
                        choice: "value".into(),
                        value: Some(s.example.clone()),
                        confirmed_by: "Test".into(),
                        at: "2026-10-16T00:00:00Z".into(),
                        question: String::new(),
                        carries: Vec::new(),
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        let hashes = pack
            .tables
            .iter()
            .map(|t| (t.name.clone(), "0".repeat(64)))
            .collect::<BTreeMap<_, _>>();
        for table in &pack.tables {
            for field in &table.fields {
                let exported = BTreeMap::from([(
                    table.name.clone(),
                    Exported {
                        fallbacks: BTreeMap::new(),
                        absent: vec![field.clone()],
                    },
                )]);
                let built = build_recipe_with(&pack, &hashes, &answers, &exported)
                    .unwrap_or_else(|e| panic!("{id}: {} without {field}: {e}", table.name));
                let recipe = serde_json::to_value(&built.recipe).unwrap();
                // Single-column times are read in UTC, as pack exports give
                // them, so a time written without its zone is still read.
                for event in recipe["events"].as_array().into_iter().flatten() {
                    assert!(
                        event["timestamp"].is_object(),
                        "{id}: {} keeps a bare timestamp column",
                        event["id"]
                    );
                }
                for category in ["objects", "events", "relationships", "event_links"] {
                    for rule in recipe[category].as_array().into_iter().flatten() {
                        assert!(
                            !names(rule, &table.name, field),
                            "{id}: {} without {field}: rule {} still uses it",
                            table.name,
                            rule["id"]
                        );
                    }
                }
                checked += 1;
            }
        }
    }
    assert!(checked > 100, "{checked}");
}
