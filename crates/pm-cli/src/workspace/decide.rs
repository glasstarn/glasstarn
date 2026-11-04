//! `workspace decide`: record a decision that shapes the measured facts — a
//! rule to check, whether a performer is a person or a system, or the amount
//! field. An agent records suggestions (`--status suggested --by agent`); a decision is
//! recorded with the name or role of the person who confirmed it. Every entry
//! goes to the audit log, and the revision goes up so an open results page
//! refreshes.
use super::extract::project_path;
use super::{print_json, root_option};
use pm_core::ResourceKind;
use pm_core::declare::Template;
use pm_project::improvement::{Improvement, Said, SavedPerformer, SavedRule, Status};
use serde_json::json;
use std::collections::BTreeMap;
use std::process::ExitCode;

const USAGE: &str = "workspace decide <project> rule|performer|value-field ... \
--status suggested|confirmed|rejected --by <agent, or the person who confirmed> [--reason <why>]";

pub(super) fn command(args: &[String]) -> Result<ExitCode, String> {
    let (root, args) = root_option(args)?;
    let mut positional = Vec::new();
    let mut flags = BTreeMap::<String, String>::new();
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--json" {
        } else if let Some(name) = arg.strip_prefix("--") {
            i += 1;
            let value = args.get(i).ok_or(format!("{arg} needs a value"))?;
            if flags.insert(name.to_owned(), value.clone()).is_some() {
                return Err(format!("{arg} is given twice"));
            }
        } else {
            positional.push(arg.clone());
        }
        i += 1;
    }
    let [project, what] = positional.as_slice() else {
        return Err(USAGE.into());
    };
    let flag = |name: &str| flags.get(name).map(String::as_str);
    let need = |name: &str| flag(name).ok_or(format!("workspace decide {what} needs --{name}"));
    // Writes the decisions file only, under its own lock, so a person can
    // confirm in chat while the project is open in the app.
    let folder = project_path(&root, project)?;
    let manifest = pm_project::workspace::WorkspaceFolder::inspect(&folder)
        .map_err(|error| error.to_string())?;
    let said = || -> Result<Said, String> {
        let status = match need("status")? {
            "suggested" => Status::Suggested,
            "confirmed" => Status::Confirmed,
            "rejected" => Status::Rejected,
            other => {
                return Err(format!(
                    "--status expects suggested, confirmed or rejected, not '{other}'"
                ));
            }
        };
        let by = need("by")?.trim();
        if by.is_empty() || by.len() > 120 {
            return Err("--by needs a name or role (at most 120 characters)".into());
        }
        Ok(Said {
            status,
            by: by.to_owned(),
            at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            reason: flag("reason").map(str::to_owned),
        })
    };
    let allowed: &[&str] = match what.as_str() {
        "rule" => &["template", "a", "b", "status", "by", "reason"],
        "performer" => &["name", "kind", "status", "by", "reason"],
        "value-field" => &["field"],
        // Where a step belongs and what it costs are suggestions; they are
        // discussed in the chat, not saved in the project.
        "bucket" | "effort" => {
            return Err(format!(
                "workspace decide {what} is no longer available: suggest it in the conversation instead"
            ));
        }
        _ => return Err(USAGE.into()),
    };
    if let Some(unknown) = flags.keys().find(|k| !allowed.contains(&k.as_str())) {
        return Err(format!(
            "unknown workspace decide {what} option '--{unknown}'"
        ));
    }
    let (improvement, entry) =
        Improvement::change(&folder, &format!("decide_{what}"), |improvement| {
            let entry = match what.as_str() {
                "rule" => {
                    let template: Template =
                serde_json::from_value(json!(need("template")?)).map_err(|_| {
                    "--template expects existence, at_most_once, init, end, responded_existence, \
                     response, chain_response, precedence, chain_precedence, succession, \
                     not_coexistence, not_succession or different_performers"
                        .to_owned()
                })?;
                    let b = flag("b").map(str::to_owned);
                    if template.is_binary() != b.is_some() {
                        return Err(if template.is_binary() {
                            "this rule names two steps: give --a and --b".into()
                        } else {
                            "this rule names one step: give --a only".into()
                        });
                    }
                    let rule = SavedRule {
                        template,
                        a: need("a")?.to_owned(),
                        b,
                        said: said()?,
                    };
                    let entry = json!(rule);
                    improvement.put_rule(rule);
                    entry
                }
                "performer" => {
                    let kind = match need("kind")? {
                        "person" | "human" => ResourceKind::Human,
                        "system" => ResourceKind::System,
                        "nobody" | "unrecorded" => ResourceKind::Unrecorded,
                        other => {
                            return Err(format!(
                                "--kind expects person, system or nobody, not '{other}'"
                            ));
                        }
                    };
                    let saved = SavedPerformer {
                        kind,
                        said: said()?,
                    };
                    let entry = json!({"name": need("name")?, "performer": saved});
                    improvement
                        .performers
                        .insert(need("name")?.to_owned(), saved);
                    entry
                }
                _ => {
                    let field = need("field")?.trim();
                    improvement.value_field = (!field.is_empty()).then(|| field.to_owned());
                    json!({"value_field": improvement.value_field})
                }
            };
            improvement.last_change = Some(describe(what, &entry));
            Ok((entry.clone(), json!({"entry": entry})))
        })?;
    print_json(&json!({
        "status": "saved",
        "what": what,
        "entry": entry,
        "revision": improvement.revision,
        "project": {"id": manifest.id, "name": manifest.name, "path": folder},
        "next": "Run workspace query with {\"type\":\"improvement\"} to see the effect; a decision needs the person's own confirmation, a suggestion does not.",
    }))?;
    Ok(ExitCode::SUCCESS)
}

/// The change in words, for the open results page.
fn describe(what: &str, entry: &serde_json::Value) -> String {
    let said = |v: &serde_json::Value| {
        let by = v["by"].as_str().unwrap_or("someone");
        match v["status"].as_str() {
            Some("confirmed") => format!("{by} confirmed"),
            Some("rejected") => format!("{by} rejected"),
            _ => format!("{by} suggested"),
        }
    };
    let text = |s: &str| s.to_owned();
    match what {
        "rule" => {
            let template: Template =
                serde_json::from_value(entry["template"].clone()).unwrap_or(Template::Response);
            let a = entry["a"].as_str().unwrap_or_default();
            let b = entry["b"].as_str().unwrap_or(a);
            format!("{}: {}", said(entry), template.describe(a, b))
        }
        "performer" => format!(
            "{}: {} is {}",
            said(&entry["performer"]),
            entry["name"].as_str().unwrap_or_default(),
            match entry["performer"]["kind"].as_str() {
                Some("system") => "a system",
                Some("human") => "a person",
                _ => "nobody recorded",
            }
        ),
        _ => text("The amount field changed"),
    }
}
