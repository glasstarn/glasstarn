//! Salesforce discovery: what an org holds for the objects a process needs,
//! read from metadata only. For each object it lists the company's own
//! fields, which fields keep history, and the history object; with a pack's
//! needs it names what is missing, such as status changes that are not
//! tracked. No business records are read, so no approval is needed; the
//! person's saved, authenticated Salesforce CLI alias is reused as is.
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::Path;
use std::process::{Command, Stdio};

/// What a pack needs from one object's field history.
#[derive(Debug, Clone)]
pub struct HistoryNeed {
    pub object: String,
    pub history_table: String,
    pub fields: Vec<String>,
    /// Fields the pack reads from the object itself.
    pub required: Vec<String>,
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_')
}

/// The history object Salesforce keeps for an object's tracked fields.
pub fn history_object(object: &str) -> String {
    if let Some(base) = object.strip_suffix("__c") {
        format!("{base}__History")
    } else if object == "Opportunity" {
        "OpportunityFieldHistory".into()
    } else {
        format!("{object}History")
    }
}

fn run(sf: &Path, args: &[&str]) -> Result<Value, String> {
    let output = Command::new(sf)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|_| {
            "Salesforce CLI could not start. Install the official sf CLI and authenticate an approved read-only org alias.".to_owned()
        })?;
    let value: Value = serde_json::from_slice(&output.stdout)
        .map_err(|_| "Salesforce CLI returned no readable answer".to_owned())?;
    if value["status"] != 0 {
        return Err(value["message"]
            .as_str()
            .unwrap_or("Salesforce CLI reported an error")
            .chars()
            .take(200)
            .collect());
    }
    Ok(value["result"].clone())
}

/// Compares a field the pack needs with the tracked fields; history rows
/// name owner changes `Owner` while metadata calls the field `OwnerId`.
fn tracked_covers(tracked: &BTreeSet<String>, field: &str) -> bool {
    tracked.contains(field)
        || tracked.contains(&format!("{field}Id"))
        || field
            .strip_suffix("Id")
            .is_some_and(|base| tracked.contains(base))
}

/// Reads describe metadata and tracked fields for each object.
pub fn salesforce(
    sf: &Path,
    org: &str,
    api_version: u32,
    objects: &[String],
    needs: &[HistoryNeed],
) -> Result<Value, String> {
    if !identifier(&org.replace('-', "_")) {
        return Err("--org must be a saved Salesforce CLI alias".into());
    }
    if objects.is_empty() || objects.len() > 20 || !objects.iter().all(|o| identifier(o)) {
        return Err("name one to twenty objects by their API names".into());
    }
    let api = format!("{api_version}.0");
    let mut found = Vec::new();
    let mut gaps = Vec::new();
    for object in objects {
        let described = run(
            sf,
            &[
                "sobject",
                "describe",
                "--sobject",
                object,
                "--target-org",
                org,
                "--api-version",
                &api,
                "--json",
            ],
        )?;
        let fields = described["fields"].as_array().cloned().unwrap_or_default();
        let names = fields
            .iter()
            .filter_map(|f| f["name"].as_str().map(str::to_owned))
            .collect::<BTreeSet<_>>();
        let custom = fields
            .iter()
            .filter(|f| f["custom"] == true)
            .take(40)
            .map(|f| json!({"name": f["name"], "label": f["label"], "type": f["type"]}))
            .collect::<Vec<_>>();
        let query = format!(
            "SELECT QualifiedApiName FROM FieldDefinition WHERE EntityDefinition.QualifiedApiName = '{object}' AND IsFieldHistoryTracked = true"
        );
        let tracked = run(
            sf,
            &[
                "data",
                "query",
                "--query",
                &query,
                "--target-org",
                org,
                "--api-version",
                &api,
                "--json",
            ],
        )?["records"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|r| r["QualifiedApiName"].as_str().map(str::to_owned))
            .collect::<BTreeSet<_>>();
        let history = history_object(object);
        let has_history = run(
            sf,
            &[
                "sobject",
                "describe",
                "--sobject",
                &history,
                "--target-org",
                org,
                "--api-version",
                &api,
                "--json",
            ],
        )
        .is_ok();
        for need in needs.iter().filter(|n| &n.object == object) {
            if need.fields.is_empty() {
                continue;
            }
            if !has_history {
                gaps.push(format!(
                    "{object} keeps no field history ({history} does not exist), so the steps the pack reads from {} cannot be seen. Field history tracking must be switched on for {object}; it records from then on.",
                    need.history_table
                ));
                continue;
            }
            let untracked = need
                .fields
                .iter()
                .filter(|f| !tracked_covers(&tracked, f))
                .cloned()
                .collect::<Vec<_>>();
            if !untracked.is_empty() {
                gaps.push(format!(
                    "History tracking is off for {} on {object}, so those changes cannot be seen. Switching it on records changes from then on; earlier changes are not recovered.",
                    untracked.join(", ")
                ));
            }
        }
        for need in needs.iter().filter(|n| &n.object == object) {
            let missing = need
                .required
                .iter()
                .filter(|f| !f.starts_with("{setting:") && !names.contains(*f))
                .cloned()
                .collect::<Vec<_>>();
            if !missing.is_empty() {
                gaps.push(format!(
                    "{object} has no field {} in this org.",
                    missing.join(", ")
                ));
            }
        }
        found.push(json!({
            "object": object,
            "fields": names.len(),
            "custom_fields": custom,
            "tracked_fields": tracked,
            "history_object": if has_history { json!(history) } else { Value::Null },
        }));
    }
    Ok(json!({
        "system": "salesforce",
        "org": org,
        "objects": found,
        "gaps": gaps,
        "history_retention": "Salesforce keeps field history for 18 months, and 24 months through the API, unless Field Audit Trail extends it. Ask whether the org uses Field Audit Trail before asking for older history.",
    }))
}

/// Jira discovery: each project's issue types with their workflow statuses,
/// and the site's own fields (`customfield_…`) with their names, so an agent
/// can choose the extract's fields and name the company's own fields in
/// business words. Reads metadata only.
pub fn jira(curl: &Path, user: &str, site: &str, projects: &[String]) -> Result<Value, String> {
    let host_ok = site
        .strip_prefix("https://")
        .map(|h| h.trim_end_matches('/'))
        .is_some_and(|h| {
            !h.is_empty()
                && h.contains('.')
                && h.bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'.' || c == b'-')
        });
    if !host_ok {
        return Err("--site must be the https address of the Jira site".into());
    }
    if projects.is_empty()
        || projects.len() > 20
        || !projects.iter().all(|p| {
            !p.is_empty()
                && p.len() <= 20
                && p.bytes()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_')
        })
    {
        return Err("name one to twenty Jira project keys with --object, such as OPS".into());
    }
    let mut client = crate::extract_jira::Client::new(curl, site, user);
    let mut found = Vec::new();
    for project in projects {
        let key = crate::extract_jira::encode(project);
        let statuses = client.get(&format!("/rest/api/3/project/{key}/statuses"))?;
        let types = statuses
            .as_array()
            .into_iter()
            .flatten()
            .map(|t| {
                json!({"issue_type": t["name"], "statuses": t["statuses"].as_array().into_iter().flatten()
                    .map(|s| s["name"].clone()).collect::<Vec<_>>()})
            })
            .collect::<Vec<_>>();
        found.push(json!({"project": project, "issue_types": types}));
    }
    let fields = client.get("/rest/api/3/field")?;
    let custom = fields
        .as_array()
        .into_iter()
        .flatten()
        .filter(|f| f["custom"] == true)
        .take(80)
        .map(|f| json!({"id": f["id"], "name": f["name"], "type": f["schema"]["type"]}))
        .collect::<Vec<_>>();
    Ok(json!({
        "system": "jira",
        "site": site,
        "projects": found,
        "custom_fields": custom,
        "gaps": [],
        "history_retention": "Jira keeps an issue's change history for the life of the issue unless an administrator removed it; changes to fields the extract does not keep are left out.",
    }))
}

/// ServiceNow discovery: for each table, the state labels, the company's own
/// fields (`u_…`) and whether its audit history can be read; and the active
/// assignment groups by name. Reads metadata only.
pub fn servicenow(
    curl: &Path,
    user: &str,
    instance: &str,
    tables: &[String],
) -> Result<Value, String> {
    use crate::extract_servicenow as servicenow;
    if !servicenow::instance_ok(instance) {
        return Err("--site must be the https address of the ServiceNow instance".into());
    }
    let valid = |t: &str| {
        !t.is_empty()
            && t.len() <= 80
            && t.bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
    };
    if tables.is_empty() || !tables.iter().all(|t| valid(t)) {
        return Err(format!(
            "name one or more tables with --object: {}, or any other ticket table built on task (such as sn_hr_core_case)",
            servicenow::TABLES.map(|(t, _)| t).join(", ")
        ));
    }
    let mut client =
        crate::extract_jira::Client::new(curl, instance, user).for_system("ServiceNow");
    // Other tables are read as tickets, so they must be built on task.
    for table in tables.iter().filter(|t| servicenow::file_of(t).is_none()) {
        if !servicenow::built_on_task(&mut client, table)? {
            return Err(format!(
                "{table} is not built on ServiceNow's task table, so it cannot be read as tickets"
            ));
        }
    }
    let mut found = Vec::new();
    let mut gaps = Vec::new();
    for table in tables {
        let table = servicenow::discover_table(&mut client, table)?;
        if let Some(gap) = table["gap"].as_str() {
            gaps.push(gap.to_owned());
        }
        found.push(table);
    }
    let groups = client.get(
        "/api/now/table/sys_user_group?sysparm_query=active%3Dtrue%5EORDERBYname&sysparm_fields=name&sysparm_limit=50",
    )?;
    let groups = groups["result"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|g| g["name"].as_str().map(str::to_owned))
        .collect::<Vec<_>>();
    Ok(json!({
        "system": "servicenow",
        "instance": instance,
        "tables": found,
        "assignment_groups": groups,
        "gaps": gaps,
        "history_retention": "ServiceNow keeps audit history (sys_audit) for audited fields unless an administrator archived or removed it; fields excluded from auditing leave no history.",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_the_history_object_salesforce_keeps() {
        assert_eq!(history_object("Case"), "CaseHistory");
        assert_eq!(history_object("Opportunity"), "OpportunityFieldHistory");
        assert_eq!(history_object("Invoice__c"), "Invoice__History");
    }

    #[test]
    fn an_owner_need_is_covered_by_the_owner_id_field() {
        let tracked = BTreeSet::from(["OwnerId".to_owned(), "Status".to_owned()]);
        assert!(tracked_covers(&tracked, "Owner"));
        assert!(tracked_covers(&tracked, "Status"));
        assert!(!tracked_covers(&tracked, "Priority"));
    }
}
