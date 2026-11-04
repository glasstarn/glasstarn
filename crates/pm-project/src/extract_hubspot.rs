//! HubSpot extracts: contacts, deals, tickets and quotes with activity in a
//! window, with the full history of chosen properties (lifecycle stage, deal
//! stage, ticket status, quote status …) up to the window end, the display
//! label of every stage, and the associations between the records read.
//! Read through HubSpot's CRM API (v3 objects, v4 associations) with a
//! service key (a private app's access token on older accounts).
//!
//! The transport is the system `curl`, given its URL and the token on
//! standard input. The token comes from `HUBSPOT_TOKEN` in the app's
//! environment and is never written anywhere.
//!
//! Like every extract, a spec is only filed by an agent; a person approves it
//! in the app, and the app runs it.
use crate::extract_jira::Client;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::io::Write;
use std::path::Path;

pub const API: &str = "https://api.hubapi.com";
/// Records per search page; HubSpot caps it at 200.
const SEARCH_PAGE: usize = 200;
/// Records per batch read when history is asked for; HubSpot caps it at 50.
const HISTORY_BATCH: usize = 50;
/// Records per association batch.
const ASSOCIATION_BATCH: usize = 1000;
/// A search returns at most 10,000 results; a larger scope needs a shorter
/// window, which the extract says rather than silently cutting.
const SEARCH_LIMIT: usize = 10_000;

/// The objects an extract may read, with the file each is written to and the
/// property that holds its last change.
pub const OBJECTS: [(&str, &str, &str); 4] = [
    ("contacts", "HubSpotContact", "lastmodifieddate"),
    ("deals", "HubSpotDeal", "hs_lastmodifieddate"),
    ("tickets", "HubSpotTicket", "hs_lastmodifieddate"),
    ("quotes", "HubSpotQuote", "hs_lastmodifieddate"),
];
pub const HISTORY_FILE: &str = "HubSpotHistory";
pub const ASSOCIATION_FILE: &str = "HubSpotAssociation";

/// Properties that point at people; kept as IDs, never labelled with names.
const PEOPLE: [&str; 3] = [
    "hubspot_owner_id",
    "hs_created_by_user_id",
    "hs_updated_by_user_id",
];

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Spec {
    pub version: u32,
    pub system: String,
    pub start: String,
    pub end: String,
    pub objects: Vec<ObjectSpec>,
    /// Pairs of objects whose links to write, such as `["deals","contacts"]`.
    #[serde(default)]
    pub associations: Vec<[String; 2]>,
    pub history_gaps: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectSpec {
    pub object: String,
    /// Properties to write with their current values.
    pub properties: Vec<String>,
    /// Properties whose full history to write, such as `dealstage`.
    #[serde(default)]
    pub history: Vec<String>,
}

pub struct Plan {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub objects: Vec<ObjectPlan>,
    pub associations: Vec<(String, String)>,
}

pub struct ObjectPlan {
    pub object: String,
    pub file: String,
    pub modified: String,
    pub created: String,
    pub properties: Vec<String>,
    pub history: Vec<String>,
}

fn name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && value
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
}

pub fn plan(spec: &Spec) -> Result<Plan, String> {
    if spec.version != 1 || spec.system != "hubspot" {
        return Err("this runner supports HubSpot extract spec version 1".into());
    }
    let instant = |value: &str| {
        DateTime::parse_from_rfc3339(value)
            .map(|d| d.with_timezone(&Utc))
            .map_err(|_| "start and end must be RFC3339 timestamps".to_owned())
    };
    let (start, end) = (instant(&spec.start)?, instant(&spec.end)?);
    if start >= end {
        return Err("start must be before end".into());
    }
    if spec.objects.is_empty() || spec.objects.len() > OBJECTS.len() {
        return Err("name one or more objects: contacts, deals, tickets or quotes".into());
    }
    let mut objects: Vec<ObjectPlan> = Vec::new();
    for object in &spec.objects {
        let (_, file, modified) =
            OBJECTS
                .iter()
                .find(|(o, _, _)| *o == object.object)
                .ok_or(format!(
                    "object '{}' is not one this extract reads: contacts, deals, tickets, quotes",
                    object.object
                ))?;
        if objects.iter().any(|o| o.object == object.object) {
            return Err(format!("object '{}' is listed twice", object.object));
        }
        if object.properties.len() > 100
            || object.history.len() > 20
            || !object
                .properties
                .iter()
                .chain(&object.history)
                .all(|p| name(p))
        {
            return Err(
                "properties must be up to 100 internal names such as dealstage, and history up to 20"
                    .into(),
            );
        }
        let created = if object.object == "quotes" {
            "hs_createdate"
        } else {
            "createdate"
        };
        let mut properties = vec![created.to_owned()];
        for p in object.properties.iter().chain(&object.history) {
            if !properties.contains(p) {
                properties.push(p.clone());
            }
        }
        objects.push(ObjectPlan {
            object: object.object.clone(),
            file: (*file).into(),
            modified: (*modified).into(),
            created: created.into(),
            properties,
            history: object.history.clone(),
        });
    }
    let mut associations = Vec::new();
    for [from, to] in &spec.associations {
        let listed = |o: &String| objects.iter().any(|p| &p.object == o);
        if !listed(from) || !listed(to) || from == to {
            return Err("associations must pair two different objects the spec reads".into());
        }
        associations.push((from.clone(), to.clone()));
    }
    Ok(Plan {
        start,
        end,
        objects,
        associations,
    })
}

pub fn preview(spec: &Spec, plan: &Plan, sha256: &str) -> Value {
    let objects = plan
        .objects
        .iter()
        .map(|o| {
            json!({"object":o.object,"file":o.file,"properties":o.properties,"history":o.history,"created":o.created,
                "scope":format!("{} with {} on or after the window start and created before its end", o.object, o.modified)})
        })
        .collect::<Vec<_>>();
    json!({"status":"needs_approval","system":"hubspot","site":API,"spec_sha256":sha256,
        "start":spec.start,"end":spec.end,
        "scope":"Records changed in the window, with the full history of the listed properties up to the window end, and the links between them. History before the window is included.",
        "objects":objects,
        "associations":plan.associations.iter().map(|(f, t)| format!("{f} to {t}")).collect::<Vec<_>>(),
        "endpoints":["POST /crm/v3/objects/{object}/search (records changed in the window)","POST /crm/v3/objects/{object}/batch/read (properties and their history)","GET /crm/v3/properties/{object}/{property} and /crm/v3/pipelines/{object} (stage labels)","POST /crm/v4/associations/{from}/{to}/batch/read (links)"],
        "credentials":"HUBSPOT_TOKEN (a HubSpot service key, or an older private app token, with read scopes) from the app's environment; never stored",
        "history_coverage":"HubSpot keeps every property's history; a record changed more than about 20 times in one property may have older changes trimmed",
        "history_gaps":spec.history_gaps})
}

/// The service key (or private app token) from `HUBSPOT_TOKEN`.
pub fn token_from_environment() -> Result<String, String> {
    let token = std::env::var("HUBSPOT_TOKEN").unwrap_or_default();
    if token.trim().is_empty() {
        return Err("set HUBSPOT_TOKEN (a HubSpot service key with read access to the objects) in the app's environment, then approve again".into());
    }
    Ok(token.trim().to_owned())
}

fn csv(values: &[String]) -> String {
    values
        .iter()
        .map(|v| {
            if v.contains([',', '"', '\n', '\r']) {
                format!("\"{}\"", v.replace('"', "\"\""))
            } else {
                v.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// HubSpot times as RFC 3339 in UTC; times and epoch milliseconds both occur.
fn time(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|d| d.with_timezone(&Utc))
        .ok()
        .or_else(|| {
            value
                .parse::<i64>()
                .ok()
                .and_then(DateTime::from_timestamp_millis)
        })
}

fn stamp(time: DateTime<Utc>) -> String {
    time.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// Labels for the stored values of a property: its options, and for deal and
/// ticket stages, the stages of every pipeline.
fn labels(
    client: &mut Client,
    object: &str,
    property: &str,
) -> Result<HashMap<String, String>, String> {
    let mut map = HashMap::new();
    if PEOPLE.contains(&property) {
        return Ok(map);
    }
    if matches!(
        (object, property),
        ("deals", "dealstage") | ("tickets", "hs_pipeline_stage")
    ) {
        let pipelines = client.get(&format!("/crm/v3/pipelines/{object}"))?;
        for pipeline in pipelines["results"].as_array().into_iter().flatten() {
            for stage in pipeline["stages"].as_array().into_iter().flatten() {
                map.insert(text(&stage["id"]), text(&stage["label"]));
            }
        }
        return Ok(map);
    }
    if matches!(
        (object, property),
        ("deals", "pipeline") | ("tickets", "hs_pipeline")
    ) {
        let pipelines = client.get(&format!("/crm/v3/pipelines/{object}"))?;
        for pipeline in pipelines["results"].as_array().into_iter().flatten() {
            map.insert(text(&pipeline["id"]), text(&pipeline["label"]));
        }
        return Ok(map);
    }
    let definition = client.get(&format!("/crm/v3/properties/{object}/{property}"))?;
    for option in definition["options"].as_array().into_iter().flatten() {
        map.insert(text(&option["value"]), text(&option["label"]));
    }
    Ok(map)
}

/// Downloads into `stage`: one file per object, the history file and the
/// association file. Returns a report per file.
pub fn download(
    spec: &Spec,
    plan: &Plan,
    curl: &Path,
    token: &str,
    stage: &Path,
) -> Result<Vec<Value>, String> {
    let _ = spec;
    let mut client = Client::bearer(curl, API, token).for_system("HubSpot");
    let start_ms = plan.start.timestamp_millis().to_string();
    let end_ms = plan.end.timestamp_millis().to_string();
    let mut reports = Vec::new();
    let mut ids: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut history_rows = vec![csv(&[
        "object",
        "record",
        "property",
        "value",
        "label",
        "timestamp",
        "source",
        "user",
        "first",
    ]
    .map(String::from))];
    let mut before = 0u64;
    for object in &plan.objects {
        // Pass one: the records changed in the window.
        let mut found = Vec::new();
        let mut after: Option<String> = None;
        loop {
            let mut body = json!({
                "filterGroups":[{"filters":[
                    {"propertyName":object.modified,"operator":"GTE","value":start_ms},
                    {"propertyName":object.created,"operator":"LT","value":end_ms}]}],
                "properties":[object.created],
                "sorts":[{"propertyName":object.created,"direction":"ASCENDING"}],
                "limit":SEARCH_PAGE});
            if let Some(after) = &after {
                body["after"] = json!(after);
            }
            let page = client.post(&format!("/crm/v3/objects/{}/search", object.object), &body)?;
            for result in page["results"].as_array().into_iter().flatten() {
                found.push(text(&result["id"]));
            }
            match page["paging"]["next"]["after"].as_str() {
                Some(next) if found.len() < SEARCH_LIMIT => after = Some(next.to_owned()),
                Some(_) => {
                    return Err(format!(
                        "more than {SEARCH_LIMIT} {} changed in the window, the most HubSpot's search returns; file a shorter window",
                        object.object
                    ));
                }
                None => break,
            }
        }
        // Labels for every property whose values are codes.
        let mut label_maps = HashMap::new();
        for property in &object.history {
            label_maps.insert(
                property.clone(),
                labels(&mut client, &object.object, property)?,
            );
        }
        // Pass two: properties and their history, fifty records at a time.
        let label_columns: Vec<&String> = object
            .properties
            .iter()
            .filter(|p| label_maps.get(*p).is_some_and(|m| !m.is_empty()))
            .collect();
        let mut header = vec!["id".to_owned(), "created".to_owned()];
        header.extend(
            object
                .properties
                .iter()
                .filter(|p| *p != &object.created)
                .cloned(),
        );
        header.extend(label_columns.iter().map(|p| format!("{p}_label")));
        let mut rows = vec![csv(&header)];
        let mut kept = Vec::new();
        for chunk in found.chunks(HISTORY_BATCH) {
            let body = json!({
                "inputs":chunk.iter().map(|id| json!({"id":id})).collect::<Vec<_>>(),
                "properties":object.properties,
                "propertiesWithHistory":object.history});
            let batch = client.post(
                &format!("/crm/v3/objects/{}/batch/read", object.object),
                &body,
            )?;
            for record in batch["results"].as_array().into_iter().flatten() {
                let id = text(&record["id"]);
                let props = &record["properties"];
                let Some(created) = time(&text(&props[&object.created])) else {
                    return Err(format!(
                        "HubSpot returned a {} record without a valid {} creation time; the extract was not registered",
                        object.object, object.created
                    ));
                };
                if created >= plan.end {
                    continue;
                }
                let mut row = vec![id.clone(), stamp(created)];
                for p in object.properties.iter().filter(|p| *p != &object.created) {
                    row.push(text(&props[p]));
                }
                for p in &label_columns {
                    let value = text(&props[p.as_str()]);
                    row.push(label_maps[p.as_str()].get(&value).cloned().unwrap_or(value));
                }
                rows.push(csv(&row));
                kept.push(id.clone());
                for property in &object.history {
                    let mut changes = record["propertiesWithHistory"][property]
                        .as_array()
                        .cloned()
                        .unwrap_or_default()
                        .into_iter()
                        .filter_map(|change| time(&text(&change["timestamp"])).map(|t| (t, change)))
                        .filter(|(t, _)| *t < plan.end)
                        .collect::<Vec<_>>();
                    changes.sort_by_key(|(t, _)| *t);
                    for (n, (at, change)) in changes.iter().enumerate() {
                        if *at < plan.start {
                            before += 1;
                        }
                        let value = text(&change["value"]);
                        let label = label_maps[property]
                            .get(&value)
                            .cloned()
                            .unwrap_or_else(|| value.clone());
                        history_rows.push(csv(&[
                            object.object.clone(),
                            id.clone(),
                            property.clone(),
                            value,
                            label,
                            stamp(*at),
                            text(&change["sourceType"]),
                            text(&change["updatedByUserId"]),
                            // A property's first recorded value is not necessarily
                            // an initial value: it may first be assigned days later.
                            (n == 0 && *at <= created).to_string(),
                        ]));
                    }
                }
            }
        }
        write(stage, &object.file, &rows)?;
        reports.push(
            json!({"object":object.file,"hubspot_object":object.object,"kind":"records",
            "scope":"records_changed_in_window","rows":rows.len() - 1,"fields":header,
            "history_coverage":"unverified","empty":rows.len() == 1}),
        );
        ids.insert(object.object.clone(), kept);
    }
    let history = history_rows.len() - 1;
    write(stage, HISTORY_FILE, &history_rows)?;
    reports.push(
        json!({"object":HISTORY_FILE,"kind":"history","rows":history,
        "rows_before_window":before,"history_coverage":"unverified","empty":history == 0}),
    );

    // Links between the records read.
    let mut links = vec![csv(
        &["from_object", "from_id", "to_object", "to_id"].map(String::from)
    )];
    let mut seen = BTreeSet::new();
    for (from, to) in &plan.associations {
        let wanted: BTreeSet<&String> = ids[to].iter().collect();
        for chunk in ids[from].chunks(ASSOCIATION_BATCH) {
            let body =
                json!({"inputs":chunk.iter().map(|id| json!({"id":id})).collect::<Vec<_>>()});
            let found = client.post(
                &format!("/crm/v4/associations/{from}/{to}/batch/read"),
                &body,
            )?;
            for result in found["results"].as_array().into_iter().flatten() {
                let source = text(&result["from"]["id"]);
                for target in result["to"].as_array().into_iter().flatten() {
                    let target = text(&target["toObjectId"]);
                    if wanted.contains(&target)
                        && seen.insert((from.clone(), source.clone(), target.clone()))
                    {
                        links.push(csv(&[from.clone(), source.clone(), to.clone(), target]));
                    }
                }
            }
        }
    }
    let count = links.len() - 1;
    write(stage, ASSOCIATION_FILE, &links)?;
    reports.push(json!({"object":ASSOCIATION_FILE,"kind":"links","rows":count,"empty":count == 0}));
    if let Some(first) = reports.first_mut() {
        first["api_calls"] = json!(client.calls());
    }
    Ok(reports)
}

fn write(stage: &Path, name: &str, rows: &[String]) -> Result<(), String> {
    let mut file =
        fs::File::create(stage.join(format!("{name}.csv"))).map_err(|_| "cannot stage export")?;
    for row in rows {
        writeln!(file, "{row}").map_err(|_| "cannot stage export")?;
    }
    Ok(())
}

/// The files an extract writes, by name without extension.
pub fn files(plan: &Plan) -> Vec<String> {
    plan.objects
        .iter()
        .map(|o| o.file.clone())
        .chain([HISTORY_FILE.to_owned(), ASSOCIATION_FILE.to_owned()])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(objects: Value) -> Spec {
        serde_json::from_value(json!({"version":1,"system":"hubspot","start":"2026-01-01T00:00:00Z",
            "end":"2026-02-01T00:00:00Z","objects":objects,"associations":[["deals","contacts"]],"history_gaps":[]}))
        .unwrap()
    }

    #[test]
    fn plans_only_the_objects_it_knows_with_clean_property_names() {
        let ok = spec(
            json!([{"object":"deals","properties":["amount"],"history":["dealstage"]},
                             {"object":"contacts","properties":[],"history":["lifecyclestage"]}]),
        );
        let plan = plan(&ok).unwrap();
        assert_eq!(
            plan.objects[0].properties,
            ["createdate", "amount", "dealstage"]
        );
        assert_eq!(
            files(&plan),
            [
                "HubSpotDeal",
                "HubSpotContact",
                "HubSpotHistory",
                "HubSpotAssociation"
            ]
        );
        let unknown = spec(json!([{"object":"companies","properties":[]}]));
        assert!(super::plan(&unknown).is_err());
        let odd = spec(json!([{"object":"deals","properties":["amount; drop"]}]));
        assert!(super::plan(&odd).is_err());
        // Associations must pair objects the spec reads.
        let alone = spec(json!([{"object":"deals","properties":[]}]));
        assert!(super::plan(&alone).is_err());
    }

    #[test]
    fn reads_both_kinds_of_time() {
        assert_eq!(
            stamp(time("2026-01-05T09:00:00.123Z").unwrap()),
            "2026-01-05T09:00:00.123Z"
        );
        assert_eq!(
            stamp(time("1767603600000").unwrap()),
            "2026-01-05T09:00:00.000Z"
        );
        assert!(time("yesterday").is_none());
    }
}
