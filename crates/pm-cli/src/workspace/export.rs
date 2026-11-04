//! `workspace export`: the files the browser offers, written into the
//! project's own `exports` folder so an agent can say where they are: the
//! process map as BPMN, one row per item, and the routes with their counts.
//! The project's current filters apply, as on the page.
use super::{open_project_to_read, print_json, root_option};
use serde_json::json;
use std::collections::BTreeMap;
use std::process::ExitCode;

const USAGE: &str =
    "workspace export <project-id|folder> [--type <item type>] [--workspace-dir <directory>]";

pub(super) fn command(args: &[String]) -> Result<ExitCode, String> {
    let (root, args) = root_option(args)?;
    let mut project = None;
    let mut item_type = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--json" => {}
            "--type" => {
                i += 1;
                item_type = Some(args.get(i).ok_or("--type needs an item type")?.clone());
            }
            other if other.starts_with("--") => {
                return Err(format!("unknown workspace export option '{other}'"));
            }
            other if project.replace(other.to_owned()).is_some() => return Err(USAGE.into()),
            _ => {}
        }
        i += 1;
    }
    let project = project.ok_or(USAGE)?;
    // Works while the project is open in a page: exports are new files.
    let folder = open_project_to_read(&root, &project)?;
    let state = folder.state().map_err(|e| e.to_string())?;
    let asset = state
        .source
        .as_ref()
        .ok_or("project has no imported data")?;
    let source = folder.resolve_asset(asset).map_err(|e| e.to_string())?;
    let not_settled = |_| "the project's data checks are not settled; finish them first".to_owned();
    // The item type exported: for linked records, the one asked for, else the
    // one the page follows, with the page's filter when it is that type.
    let mut followed = None;
    let (loaded, filter) = if let Some(case) = &state.case_project {
        if item_type.is_some() {
            return Err(
                "--type is for projects with linked records; this project has one kind of item"
                    .into(),
            );
        }
        let loaded = pm_query::load_log(
            &source,
            Some(case.mapping.clone()),
            &case.policy,
            case.rules.clone(),
        )
        .map_err(|e| e.to_string())?
        .map_err(not_settled)?;
        (loaded, case.view_filter.clone())
    } else if let Some(ocel_filter) = &state.ocel_filter {
        let view = state.followed_view.as_ref();
        let types = pm_query::analyze_ocel(&source, ocel_filter)
            .map(|analysis| analysis.catalog.object_types)
            .map_err(|e| e.to_string())?;
        let listed = types.join(", ");
        let wanted = item_type
            .clone()
            .or_else(|| view.map(|view| view.object_type.clone()))
            .ok_or_else(|| format!("this project has linked records: name the item type to export with --type (one of: {listed})"))?;
        // "invoices" or "Invoice" both mean the Invoice type.
        let plain = |name: &str| name.trim().to_lowercase().trim_end_matches('s').to_owned();
        let chosen = types
            .iter()
            .find(|name| plain(name) == plain(&wanted))
            .cloned()
            .ok_or_else(|| format!("there is no item type '{wanted}' here (one of: {listed})"))?;
        let mut file = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
        let summary = pm_query::follow_ocel(&source, ocel_filter, &chosen, file.as_file_mut())
            .map_err(|e| e.to_string())?;
        let loaded = pm_query::load_log(
            file.path(),
            Some(summary.mapping()),
            &pm_ingest::finding::Policy::new(),
            Default::default(),
        )
        .map_err(|e| e.to_string())?
        .map_err(not_settled)?;
        let filter = view
            .filter(|view| view.object_type == chosen)
            .map(|view| view.filter.clone())
            .unwrap_or_default();
        followed = Some(chosen);
        (loaded, filter)
    } else {
        return Err("project has data but no saved analysis settings".into());
    };
    let filtered = filter.apply(&loaded.log);
    let log = &filtered.log;
    if log.case_count() == 0 {
        return Err("the current filters leave no items to export".into());
    }

    // Every step and connection, as the page's "Show all".
    let view = pm_query::map_view(
        log,
        &pm_query::MapSelection::Limits {
            activities: log.activities.len(),
            connections: usize::MAX,
        },
    )
    .map_err(|e| e.to_string())?;
    let title = match &followed {
        Some(item_type) => format!("{} ({item_type})", folder.manifest.name),
        None => folder.manifest.name.clone(),
    };
    let bpmn = pm_query::map_view_bpmn(&view, &title);

    let mut items = String::from("item,started,finished,days,steps,route\n");
    let mut routes: BTreeMap<Vec<String>, (usize, f64)> = BTreeMap::new();
    for trace in &log.traces {
        let (Some(first), Some(last)) = (trace.events.first(), trace.events.last()) else {
            continue;
        };
        let steps: Vec<String> = trace
            .events
            .iter()
            .map(|event| log.activity_label(event.activity).to_owned())
            .collect();
        let days = (last.at - first.at) as f64 / 86_400_000_000.0;
        items.push_str(&format!(
            "{},{},{},{:.2},{},{}\n",
            cell(log.case_label(trace.case)),
            time(first.at),
            time(last.at),
            days,
            steps.len(),
            cell(&steps.join(" > ")),
        ));
        let entry = routes.entry(steps).or_default();
        entry.0 += 1;
        entry.1 += days;
    }
    let total = log.case_count() as f64;
    let mut ranked: Vec<_> = routes.into_iter().collect();
    ranked.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(a.0.cmp(&b.0)));
    let mut variants = String::from("variant,items,share,average_days,route\n");
    for (rank, (steps, (count, days))) in ranked.iter().enumerate() {
        variants.push_str(&format!(
            "V{},{},{:.1}%,{:.2},{}\n",
            rank + 1,
            count,
            *count as f64 / total * 100.0,
            days / *count as f64,
            cell(&steps.join(" > ")),
        ));
    }

    let prefix = followed
        .as_deref()
        .map(|item_type| {
            let safe: String = item_type
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() {
                        c.to_ascii_lowercase()
                    } else {
                        '-'
                    }
                })
                .collect();
            format!("{safe}-")
        })
        .unwrap_or_default();
    let stamp = chrono::Local::now().format("%Y-%m-%d-%H%M%S").to_string();
    let mut written = Vec::new();
    for (name, bytes) in [
        (
            format!("process-map-{prefix}{stamp}.bpmn"),
            bpmn.into_bytes(),
        ),
        (format!("items-{prefix}{stamp}.csv"), items.into_bytes()),
        (
            format!("variants-{prefix}{stamp}.csv"),
            variants.into_bytes(),
        ),
    ] {
        let path = folder
            .write_artifact("exports", &name, &bytes)
            .map_err(|e| e.to_string())?;
        written.push(path.display().to_string());
    }
    print_json(&json!({
        "status": "exported",
        "project": {"id": folder.manifest.id, "name": folder.manifest.name},
        "files": written,
        "items": log.case_count(),
        "of_items": filtered.cases_before,
        "filtered": filtered.narrowed(),
        "item_type": followed,
        "note": "Process map (BPMN 2.0, every step and connection), one row per item, and the routes with their counts, for the project's current filters."
    }))?;
    Ok(ExitCode::SUCCESS)
}

/// A CSV cell, quoted when it holds a comma, quote or line break.
fn cell(text: &str) -> String {
    if text.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_owned()
    }
}

fn time(micros: i64) -> String {
    chrono::DateTime::from_timestamp_micros(micros)
        .map(|at| at.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_default()
}
