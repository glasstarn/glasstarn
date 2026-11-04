//! Headless project-folder workflow used by agent skills and the browser.

use pm_core::OcelFilter;
use pm_ingest::Rules;
use pm_ingest::finding::Policy;
use pm_ingest::validate::Mapping;
use pm_ingest::{PreparationOutcome, PreparationRecipe};
use pm_project::Project;
use pm_project::workspace::{
    InvestigationPurpose, WorkspaceAsset, WorkspaceFolder, WorkspaceState, valid_workspace_id,
};
use pm_query::{Outcome, analyze_filtered};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

mod decide;
mod demo;
mod export;
pub(super) mod extract;
mod invented;
mod investigation;
mod pack;
mod prepare_ocel;

/// Set by `--full`: print whole answers instead of holding them to the agent
/// budget.
static FULL_OUTPUT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn print_everything() {
    FULL_OUTPUT.store(true, std::sync::atomic::Ordering::Relaxed);
}

pub(super) fn command(args: &[String]) -> Result<ExitCode, String> {
    let full = args.iter().any(|arg| arg == "--full");
    if full {
        print_everything();
    }
    let args = args
        .iter()
        .filter(|arg| *arg != "--full")
        .cloned()
        .collect::<Vec<_>>();
    let Some((action, args)) = args.split_first() else {
        return Err(
            "workspace needs create, decide, demo, export, list, inspect, inventory, prepare, prepare-ocel, import, analyze, query, record, window, name-type, move, or show"
                .into(),
        );
    };
    match action.as_str() {
        "create" => create(args),
        "demo" => demo::command(args),
        "decide" => decide::command(args),
        "export" => export::command(args),
        "list" => list(args),
        "inspect" => inspect(args),
        "inventory" => inventory(args),
        "prepare" => prepare(args),
        "prepare-ocel" => prepare_ocel::command(args),
        "pack" => pack::command(args),
        "import" => import(args),
        "analyze" => analyze(args),
        "show" => show(args),
        "query" => investigation::query(args),
        "record" => investigation::record(args),
        "window" => window(args),
        "name-type" => name_type(args),
        "move" => move_project(args),
        other => Err(format!("unknown workspace action '{other}'")),
    }
}

/// `workspace move <project> --to <folder>`: moves a project, with everything
/// saved in it, into another folder that holds projects, such as the
/// `glasstarn` folder beside the person's own files once they arrive. The new
/// folder is remembered so the projects page still lists it.
fn move_project(args: &[String]) -> Result<ExitCode, String> {
    let (root, args) = root_option(args)?;
    let mut project = None;
    let mut target = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--to" => {
                i += 1;
                target = Some(PathBuf::from(args.get(i).ok_or("--to needs a folder")?));
            }
            "--json" => {}
            value if project.is_none() => project = Some(value.to_owned()),
            other => return Err(format!("unknown workspace move option '{other}'")),
        }
        i += 1;
    }
    let project = project.ok_or("workspace move needs a project")?;
    let target = target.ok_or("workspace move needs --to <folder>")?;
    // Opening it takes its lock, so a project open in glasstarn is not moved
    // from under it.
    let folder = open_project(&root, &project)?;
    let id = folder.manifest.id.clone();
    let from = folder.root.clone();
    drop(folder);
    std::fs::create_dir_all(&target)
        .map_err(|e| format!("cannot create {}: {e}", target.display()))?;
    let to = target.join(&id);
    if to.exists() {
        return Err(format!("{} already holds this project", target.display()));
    }
    if from.canonicalize().ok() == to.canonicalize().ok() {
        return Err("the project is already there".into());
    }
    if std::fs::rename(&from, &to).is_err() {
        // Another disk: copy, then remove the original only once the copy is whole.
        copy_tree(&from, &to).map_err(|e| format!("cannot copy the project: {e}"))?;
        std::fs::remove_dir_all(&from)
            .map_err(|e| format!("copied, but cannot remove the old folder: {e}"))?;
    }
    pm_project::folders::remember_root(&target);
    let moved = WorkspaceFolder::open(&to).map_err(|error| error.to_string())?;
    print_json(&json!({"status":"moved","project":folder_info(&moved),"from":from}))?;
    Ok(ExitCode::SUCCESS)
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            copy_tree(&path, &to.join(entry.file_name()))?;
        } else {
            std::fs::copy(&path, to.join(entry.file_name()))?;
        }
    }
    Ok(())
}

fn inventory(args: &[String]) -> Result<ExitCode, String> {
    let (root, args) = root_option(args)?;
    let mut positional = args.iter().filter(|arg| arg.as_str() != "--json");
    let project = positional
        .next()
        .ok_or("workspace inventory needs a project and at least one CSV or Excel extract")?;
    let sources = positional.map(PathBuf::from).collect::<Vec<_>>();
    if sources.is_empty() {
        return Err("workspace inventory needs at least one CSV or Excel extract".into());
    }
    if sources
        .iter()
        .any(|source| source.to_string_lossy().starts_with('-'))
    {
        return Err("unknown workspace inventory option".into());
    }
    let result = inventory_paths(&root, project, &sources)?;
    print_json(&result)?;
    Ok(if result["status"] == "unreadable" {
        ExitCode::from(3)
    } else {
        ExitCode::SUCCESS
    })
}

/// A file that could not be read, described for the agent to advise on.
fn unreadable(source: &Path, error: &str) -> Result<ExitCode, String> {
    print_json(&json!({
        "status": "unreadable",
        "file": source,
        "error": error,
        "description": pm_ingest::text_export::describe(source),
        "next": "Tell the person what the file looks like and what would work instead, using the description and its suggestion; you may know a better way from the file's kind or its column names.",
    }))?;
    Ok(ExitCode::from(3))
}

/// An export that is not plain UTF-8 CSV, as a CSV of the same name in
/// `dir`, with a note of what was converted: an Excel workbook's first sheet,
/// or a text export in another encoding or with semicolons or tabs. `None`
/// when the file can be read as it is (or is not a table export at all).
fn as_csv(
    source: &Path,
    name: &str,
    dir: &Path,
) -> Result<Option<(PathBuf, String, String)>, String> {
    let lower = name.to_ascii_lowercase();
    // A workbook saved under another name is still a workbook.
    let head = std::fs::read(source)
        .map(|b| b.into_iter().take(8).collect::<Vec<_>>())
        .unwrap_or_default();
    let workbook_bytes =
        head.starts_with(b"PK\x03\x04") || head.starts_with(&[0xD0, 0xCF, 0x11, 0xE0]);
    if pm_ingest::xlsx::is_workbook(&lower)
        || (workbook_bytes && pm_ingest::xlsx::read_sheets(source).is_ok())
    {
        let (path, name) = excel_as_csv(source, name, dir)?;
        return Ok(Some((
            path,
            name,
            "first sheet of an Excel workbook".into(),
        )));
    }
    let Some(stem) = [".csv", ".txt", ".tsv"].iter().find_map(|suffix| {
        lower
            .ends_with(suffix)
            .then(|| &name[..name.len() - suffix.len()])
    }) else {
        return Ok(None);
    };
    let csv_name = format!("{stem}.csv");
    match pm_ingest::text_export::as_utf8_csv(source)? {
        Some((bytes, note)) => {
            let out = dir.join(&csv_name);
            std::fs::write(&out, bytes).map_err(|e| e.to_string())?;
            Ok(Some((out, csv_name, note)))
        }
        None if !lower.ends_with(".csv") => {
            let out = dir.join(&csv_name);
            std::fs::copy(source, &out).map_err(|e| e.to_string())?;
            Ok(Some((out, csv_name, String::new())))
        }
        None => Ok(None),
    }
}

/// For an event log handed over as a workbook: the sheet to read (the one
/// named, or else the one that looks most like an event log, with a case,
/// a step and a time, then the most rows) as a CSV in `dir`, and every
/// sheet's name, size and whether it looks like an event log.
/// A file the quick one-file commands can read: an Excel workbook becomes its
/// sheet that looks most like an event log, other spreadsheet and report
/// formats become a table, the way `workspace import` reads them. `None` when
/// the file is read as it is.
pub(crate) fn readable_file(source: &Path, dir: &Path) -> Result<Option<PathBuf>, String> {
    let name = source
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_owned();
    if pm_ingest::xlsx::is_workbook(&name.to_ascii_lowercase()) {
        return workbook_event_log(source, dir, None).map(|(path, _, _)| Some(path));
    }
    Ok(as_csv(source, &name, dir)?.map(|(path, _, _)| path))
}

fn workbook_event_log(
    source: &Path,
    dir: &Path,
    wanted: Option<&str>,
) -> Result<(PathBuf, String, Value), String> {
    let tables = workbook_tables(source, dir)?;
    let mut sheets = Vec::new();
    let mut best: Option<(usize, (usize, u64))> = None;
    for (index, (path, _, note)) in tables.iter().enumerate() {
        let sheet = note
            .strip_prefix("sheet \"")
            .and_then(|n| n.rsplit_once("\" of "))
            .map_or_else(|| "Sheet1".to_owned(), |(n, _)| n.to_owned());
        let profile = pm_query::inspect_log(path, false).map_err(|e| e.to_string())?;
        let rows = csv::Reader::from_path(path)
            .map_err(|e| e.to_string())?
            .records()
            .count() as u64;
        let filled = 3usize.saturating_sub(
            profile
                .unfillable_roles
                .iter()
                .filter(|r| matches!(r.as_str(), "case id" | "activity" | "timestamp"))
                .count(),
        );
        sheets.push(json!({"sheet": sheet, "rows": rows, "looks_like_event_log": filled == 3}));
        let score = (filled, rows);
        let named = wanted.is_some_and(|w| w == sheet || tables.len() == 1);
        if named || (wanted.is_none() && best.is_none_or(|(_, s)| score > s)) {
            best = Some((index, if named { (usize::MAX, rows) } else { score }));
        }
    }
    let Some((index, _)) = best else {
        return Err(match wanted {
            Some(w) => format!("the workbook has no sheet named {w}"),
            None => "the workbook has no sheet with data".into(),
        });
    };
    let (path, name, _) = tables[index].clone();
    let chosen = sheets[index]["sheet"].clone();
    Ok((path, name, json!({"chosen": chosen, "sheets": sheets})))
}

/// Each sheet of a workbook with data as a CSV in `dir`, from its header
/// row: one sheet keeps the workbook's name, several are named after their
/// sheets (an admin export often holds one table per sheet).
fn workbook_tables(source: &Path, dir: &Path) -> Result<Vec<(PathBuf, String, String)>, String> {
    let file = source
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("workbook.xlsx");
    let sheets = pm_ingest::xlsx::read_sheets(source)
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|(_, rows)| rows.iter().any(|r| r.iter().any(|c| !c.trim().is_empty())))
        .collect::<Vec<_>>();
    let several = sheets.len() > 1;
    let mut out = Vec::new();
    for (sheet, rows) in sheets {
        let rows = &pm_ingest::xlsx::table(&rows);
        let (header, body, _) = pm_ingest::reshape::apply(rows, &Default::default());
        let csv_name = if several {
            let clean = sheet
                .chars()
                .map(|c| {
                    if c.is_alphanumeric() || " ._-".contains(c) {
                        c
                    } else {
                        '_'
                    }
                })
                .collect::<String>();
            format!("{}.csv", clean.trim())
        } else {
            format!(
                "{}.csv",
                file.rsplit_once('.').map_or(file, |(stem, _)| stem)
            )
        };
        let path = dir.join(&csv_name);
        let mut writer = csv::Writer::from_path(&path).map_err(|e| e.to_string())?;
        writer.write_record(&header).map_err(|e| e.to_string())?;
        for row in &body {
            writer.write_record(row).map_err(|e| e.to_string())?;
        }
        writer.flush().map_err(|e| e.to_string())?;
        let note = if several {
            format!("sheet \"{sheet}\" of {file}")
        } else {
            "first sheet of an Excel workbook".to_owned()
        };
        out.push((path, csv_name, note));
    }
    if out.is_empty() {
        return Err(format!("{file} has no sheet with data"));
    }
    Ok(out)
}

/// An Excel workbook's first sheet as a CSV of the same name in `dir`,
/// starting at its header row (below any title rows).
fn excel_as_csv(source: &Path, name: &str, dir: &Path) -> Result<(PathBuf, String), String> {
    let rows = pack::read_export(source)?;
    let rows = &pm_ingest::xlsx::table(&rows);
    let (header, body, _) = pm_ingest::reshape::apply(rows, &Default::default());
    let csv_name = format!(
        "{}.csv",
        name.rsplit_once('.').map_or(name, |(stem, _)| stem)
    );
    let out = dir.join(&csv_name);
    let mut writer = csv::Writer::from_path(&out).map_err(|e| e.to_string())?;
    writer.write_record(&header).map_err(|e| e.to_string())?;
    for row in &body {
        writer.write_record(row).map_err(|e| e.to_string())?;
    }
    writer.flush().map_err(|e| e.to_string())?;
    Ok((out, csv_name))
}

/// Copies extracts into the project and profiles them. An Excel workbook's
/// first sheet becomes a CSV of the same name, starting at its header row.
pub(super) fn inventory_paths(
    root: &Path,
    project: &str,
    sources: &[PathBuf],
) -> Result<Value, String> {
    let mut folder = open_project(root, project)?;
    let mut state = folder.state().map_err(|error| error.to_string())?;
    let mut results = Vec::new();
    let converted = tempfile::tempdir().map_err(|error| error.to_string())?;
    // A zipped export is its files.
    let unzipped = tempfile::tempdir().map_err(|error| error.to_string())?;
    let mut expanded = Vec::new();
    for source in sources {
        if source
            .to_string_lossy()
            .to_ascii_lowercase()
            .ends_with(".zip")
        {
            for (member, data) in pm_ingest::xlsx::zip_files(source).map_err(|e| e.to_string())? {
                let base = member.rsplit('/').next().unwrap_or(&member).to_owned();
                let lower = base.to_ascii_lowercase();
                if [".csv", ".txt", ".tsv", ".xlsx"]
                    .iter()
                    .any(|s| lower.ends_with(s))
                {
                    let path = unzipped.path().join(&base);
                    std::fs::write(&path, data).map_err(|e| e.to_string())?;
                    expanded.push(path);
                }
            }
        } else {
            expanded.push(source.clone());
        }
    }
    // A file Glasstarn cannot read is described for the agent rather than
    // stopping the others.
    let mut unreadable = Vec::new();
    for source in &expanded {
        let name = source
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("extract file name is not valid UTF-8")?;
        let lower = name.to_ascii_lowercase();
        let read = || -> Result<Vec<(PathBuf, String, String, Value)>, String> {
            let inputs = if pm_ingest::xlsx::is_workbook(&lower) {
                workbook_tables(source, converted.path())?
            } else {
                vec![match as_csv(source, name, converted.path())? {
                    Some(found) => found,
                    None if lower.ends_with(".csv") => {
                        (source.clone(), name.to_owned(), String::new())
                    }
                    None => {
                        return Err("not a table export Glasstarn reads (CSV or other text table, Excel, LibreOffice, or a zip of them)".into());
                    }
                }]
            };
            inputs
                .into_iter()
                .map(|(path, name, note)| {
                    let profile = pm_ingest::inventory_csv(&path).map_err(|e| e.to_string())?;
                    Ok((
                        path,
                        name,
                        note,
                        serde_json::to_value(profile).map_err(|e| e.to_string())?,
                    ))
                })
                .collect()
        };
        let inputs = match read() {
            Ok(inputs) => inputs,
            Err(error) => {
                unreadable.push(json!({
                    "file": name,
                    "error": error,
                    "description": pm_ingest::text_export::describe(source),
                }));
                continue;
            }
        };
        for (path, name, note, profile) in inputs {
            let asset = folder
                .store_asset(&path, &name, ".csv", "sources")
                .map_err(|error| error.to_string())?;
            if !state
                .preparation_sources
                .iter()
                .any(|existing| existing.sha256 == asset.sha256)
            {
                state.preparation_sources.push(asset.clone());
            }
            let mut result = json!({"asset":asset,"inventory":profile});
            if !note.is_empty() {
                result["converted"] = json!(note);
            }
            results.push(result);
        }
    }
    state
        .preparation_sources
        .sort_by(|a, b| a.name.cmp(&b.name).then(a.sha256.cmp(&b.sha256)));
    folder
        .save_state(&state)
        .map_err(|error| error.to_string())?;
    let mut result = json!({
        "status":"complete","project":folder_info(&folder),"inputs":results,
        "next":"Create a reviewed version-1 preparation recipe using the returned SHA-256 identities, then run workspace prepare."
    });
    if !unreadable.is_empty() {
        result["unreadable"] = json!(unreadable);
        result["next"] = json!(format!(
            "{} file(s) could not be read (unreadable, with a description of each). Tell the person what each file looks like and what would work instead, using the description and its suggestion; you may know a better way from the file's kind or its column names. Then: {}",
            unreadable.len(),
            result["next"].as_str().unwrap_or_default()
        ));
        if results.is_empty() {
            result["status"] = json!("unreadable");
        }
    }
    save_artifact_json(&folder, "preparation", "inventory.json", &result)?;
    Ok(result)
}

fn prepare(args: &[String]) -> Result<ExitCode, String> {
    let (root, args) = root_option(args)?;
    let mut project = None;
    let mut recipe_path = None;
    let mut policy = Policy::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--recipe" => {
                i += 1;
                recipe_path = Some(PathBuf::from(
                    args.get(i).ok_or("--recipe needs a JSON file")?,
                ));
            }
            "--answer" => {
                i += 1;
                let answer = args.get(i).ok_or("--answer needs <kind>=<option>")?;
                let (kind, choice) = answer
                    .split_once('=')
                    .ok_or("--answer expects <kind>=<option>")?;
                policy = policy.answer(super::parse_kind(kind)?, choice.to_owned());
            }
            "--json" => {}
            flag if flag.starts_with('-') => {
                return Err(format!("unknown workspace prepare option '{flag}'"));
            }
            value if project.replace(value.to_owned()).is_none() => {}
            _ => return Err("workspace prepare accepts one project".into()),
        }
        i += 1;
    }
    let mut folder = open_project(
        &root,
        project
            .as_deref()
            .ok_or("workspace prepare needs a project")?,
    )?;
    let recipe_path = recipe_path.ok_or("workspace prepare needs --recipe <file.json>")?;
    let metadata = std::fs::metadata(&recipe_path).map_err(|error| error.to_string())?;
    if metadata.len() > 1024 * 1024 {
        return Err("preparation recipe exceeds 1 MiB".into());
    }
    let recipe_text = std::fs::read_to_string(&recipe_path).map_err(|error| error.to_string())?;
    let recipe: PreparationRecipe = serde_json::from_str(&recipe_text)
        .map_err(|error| format!("invalid preparation recipe: {error}"))?;
    let mut state = folder.state().map_err(|error| error.to_string())?;
    let mut inputs = BTreeMap::new();
    for reference in &recipe.inputs {
        let matches = state
            .preparation_sources
            .iter()
            .filter(|asset| asset.sha256 == reference.sha256)
            .collect::<Vec<_>>();
        let [asset] = matches.as_slice() else {
            return Err(format!(
                "recipe input '{}' does not identify exactly one inventoried source",
                reference.id
            ));
        };
        inputs.insert(
            reference.id.clone(),
            folder
                .resolve_asset(asset)
                .map_err(|error| error.to_string())?,
        );
    }
    let temporary = folder
        .root
        .join("preparation")
        .join(format!(".prepared-events-{}.csv", std::process::id()));
    let outcome =
        pm_ingest::prepare_csv(&recipe, &inputs, &temporary).map_err(|error| error.to_string())?;
    let recipe_value = serde_json::to_value(&recipe).map_err(|error| error.to_string())?;
    save_artifact_json(&folder, "preparation", "recipe.json", &recipe_value)?;
    let report = match &outcome {
        PreparationOutcome::Complete(report) | PreparationOutcome::Blocked(report) => report,
    };
    let report_value = serde_json::to_value(report).map_err(|error| error.to_string())?;
    save_artifact_json(&folder, "preparation", "validation.json", &report_value)?;
    if let PreparationOutcome::Blocked(_) = outcome {
        let _ = std::fs::remove_file(&temporary);
        let result = json!({
            "status":"blocked","project":folder_info(&folder),"preparation":report_value,
            "next":"Review the reported reconciliation issues. Change the recipe only after the business meaning or missing data is confirmed."
        });
        print_json(&result)?;
        return Ok(ExitCode::from(3));
    }
    let prepared = folder
        .store_asset(&temporary, "prepared-events.csv", ".csv", "sources")
        .map_err(|error| error.to_string())?;
    let _ = std::fs::remove_file(&temporary);
    let stored = folder
        .resolve_asset(&prepared)
        .map_err(|error| error.to_string())?;
    let mut mapping = Mapping::new("case_id", "activity", "timestamp");
    if recipe
        .events
        .iter()
        .any(|rule| rule.end_timestamp.is_some())
    {
        mapping.end_timestamp = Some("end_timestamp".into());
    }
    if recipe.events.iter().any(|rule| rule.event_order.is_some()) {
        mapping.event_order = Some("event_order".into());
    }
    if recipe.events.iter().any(|rule| rule.resource.is_some()) {
        mapping.resource = Some("resource".into());
    }
    if recipe.events.iter().any(|rule| rule.lifecycle.is_some()) {
        mapping.lifecycle = Some("lifecycle".into());
    }
    if recipe.events.iter().any(|rule| rule.cost.is_some()) {
        mapping.cost = Some("cost".into());
    }
    let mut case_attributes = recipe
        .events
        .iter()
        .flat_map(|rule| {
            rule.case_attributes
                .iter()
                .map(|field| field.output.clone())
        })
        .chain(
            recipe
                .lookups
                .iter()
                .flat_map(|rule| rule.fields.iter().map(|field| field.output.clone())),
        )
        .collect::<Vec<_>>();
    case_attributes.sort();
    case_attributes.dedup();
    mapping.case_attributes = case_attributes;
    // Cases are labelled against the extract's window, never cut to it.
    let rules = Rules {
        observation_window: state.observation_window.clone(),
        ..Rules::new()
    };
    let outcome = analyze_filtered(
        &stored,
        Some(mapping),
        &policy,
        rules.clone(),
        Default::default(),
    )
    .map_err(|error| error.to_string())?;
    let resolved = match &outcome {
        Outcome::Complete(analysis) => analysis.mapping.clone(),
        Outcome::Blocked(blocked) => blocked.mapping.clone(),
    };
    let analysis_result = outcome_value(&folder, &outcome, &prepared, &stored, &policy)?;
    let mut project =
        Project::new(&stored, resolved, policy, rules).map_err(|error| error.to_string())?;
    project.name.clone_from(&folder.manifest.name);
    project.source.path.clone_from(&prepared.relative_path);
    state.source = Some(prepared);
    state.case_project = Some(project);
    state.ocel_filter = None;
    state.case_model = None;
    state.ocel_model = None;
    folder
        .save_state(&state)
        .map_err(|error| error.to_string())?;
    let result = json!({
        "status":analysis_result.0["status"],"kind":"prepared_case_log","project":folder_info(&folder),
        "preparation":report_value,"analysis":analysis_result.0,"recipe":folder.root.join("preparation/recipe.json"),
        "validation":folder.root.join("preparation/validation.json")
    });
    save_latest(&folder, &result)?;
    print_json(&result)?;
    Ok(analysis_result.1)
}

fn default_root() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .ok_or("cannot find your home folder; pass --workspace-dir")?;
    Ok(PathBuf::from(home).join(".glasstarn").join("projects"))
}

fn root_option(args: &[String]) -> Result<(PathBuf, Vec<String>), String> {
    let mut root = None;
    let mut rest = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--workspace-dir" {
            i += 1;
            root = Some(PathBuf::from(
                args.get(i).ok_or("--workspace-dir needs a directory")?,
            ));
        } else {
            rest.push(args[i].clone());
        }
        i += 1;
    }
    if let Some(root) = &root
        && root.is_dir()
    {
        pm_project::folders::remember_root(root);
    }
    Ok((root.map_or_else(default_root, Ok)?, rest))
}

fn create(args: &[String]) -> Result<ExitCode, String> {
    let (root, args) = root_option(args)?;
    let mut name = None;
    let mut purpose = InvestigationPurpose::Improvement;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--name" => {
                i += 1;
                name = Some(args.get(i).ok_or("--name needs a project name")?.clone());
            }
            "--purpose" => {
                i += 1;
                purpose = match args.get(i).map(String::as_str) {
                    Some("audit") => InvestigationPurpose::Audit,
                    Some("improvement") => InvestigationPurpose::Improvement,
                    _ => return Err("--purpose expects audit or improvement".into()),
                };
            }
            "--json" => {}
            other => return Err(format!("unknown workspace create option '{other}'")),
        }
        i += 1;
    }
    let folder = WorkspaceFolder::create_named(
        &root,
        name.as_deref().ok_or("workspace create requires --name")?,
        purpose,
    )
    .map_err(|error| error.to_string())?;
    // A folder made just now for this project is remembered too.
    if root != default_root()? {
        pm_project::folders::remember_root(&root);
    }
    print_json(&json!({"status":"created","project":folder_info(&folder)}))?;
    Ok(ExitCode::SUCCESS)
}

fn list(args: &[String]) -> Result<ExitCode, String> {
    let (root, args) = root_option(args)?;
    for arg in args {
        if arg != "--json" {
            return Err(format!("unknown workspace list option '{arg}'"));
        }
    }
    std::fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    // From glasstarn's home, every remembered folder is listed too, so a new
    // conversation finds projects saved in the person's own folders.
    let roots = if root == default_root()? {
        pm_project::folders::all_roots(&root)
    } else {
        vec![root.clone()]
    };
    let mut projects = Vec::new();
    for entry in roots
        .iter()
        .filter_map(|root| std::fs::read_dir(root).ok())
        .flatten()
    {
        let entry = entry.map_err(|error| error.to_string())?;
        let id = entry.file_name().to_string_lossy().into_owned();
        if valid_workspace_id(&id)
            && entry
                .file_type()
                .map_err(|error| error.to_string())?
                .is_dir()
            && let Ok(manifest) = WorkspaceFolder::inspect(&entry.path())
            && manifest.id == id
        {
            projects.push(json!({
                "id": manifest.id,
                "name": manifest.name,
                "purpose": manifest.purpose,
                "path": entry.path(),
                "has_data": !manifest.state.is_null(),
            }));
        }
    }
    projects.sort_by_key(|item| item["name"].as_str().unwrap_or_default().to_lowercase());
    print_json(&json!({"root":root,"projects":projects}))?;
    Ok(ExitCode::SUCCESS)
}

fn inspect(args: &[String]) -> Result<ExitCode, String> {
    let mut source = None;
    let mut sheet = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--json" => {}
            "--sheet" => sheet = Some(args.next().ok_or("--sheet needs a sheet name")?.clone()),
            option if option.starts_with('-') => {
                return Err(format!("unknown workspace inspect option '{option}'"));
            }
            path if source.replace(path.to_owned()).is_some() => {
                return Err("workspace inspect accepts one source".into());
            }
            _ => {}
        }
    }
    let source = source.ok_or("workspace inspect needs a source file")?;
    if is_ocel(&source) {
        let analysis = pm_query::analyze_ocel(&source, &OcelFilter::default())
            .map_err(|error| error.to_string())?;
        print_json(&json!({
            "kind":"ocel","source":source,"catalog":analysis.catalog,
            "scope":analysis.scope,"filter":analysis.filter
        }))?;
        Ok(ExitCode::SUCCESS)
    } else {
        let converted = tempfile::tempdir().map_err(|error| error.to_string())?;
        let name = Path::new(&source)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_owned();
        let opened = (|| -> Result<_, String> {
            let (read, workbook) = if pm_ingest::xlsx::is_workbook(&name.to_ascii_lowercase()) {
                let (path, _, sheets) =
                    workbook_event_log(Path::new(&source), converted.path(), sheet.as_deref())?;
                (path, Some(sheets))
            } else {
                let read = as_csv(Path::new(&source), &name, converted.path())?
                    .map_or_else(|| PathBuf::from(&source), |(path, _, _)| path);
                (read, None)
            };
            let profile = pm_query::inspect_log(&read, true).map_err(|error| error.to_string())?;
            Ok((workbook, profile))
        })();
        let (workbook, profile) = match opened {
            Ok(found) => found,
            Err(error) => return unreadable(Path::new(&source), &error),
        };
        let incomplete = !profile.unfillable_roles.is_empty();
        let mut value = json!({"kind":"case_log","source":source,"profile":profile});
        if let Some(workbook) = workbook {
            value["workbook"] = workbook;
        }
        print_json(&value)?;
        Ok(if incomplete {
            ExitCode::from(3)
        } else {
            ExitCode::SUCCESS
        })
    }
}

#[derive(Default)]
struct ImportOptions {
    project: Option<String>,
    /// The sheet of a workbook to read; by default the one that looks most
    /// like an event log.
    sheet: Option<String>,
    source: Option<String>,
    case: Option<String>,
    activity: Option<String>,
    timestamp: Option<String>,
    event_order: Option<String>,
    end_timestamp: Option<String>,
    resource: Option<String>,
    lifecycle: Option<String>,
    cost: Option<String>,
    case_attributes: Vec<String>,
    /// Columns not to carry when every other column is carried by default.
    leave_out: Vec<String>,
    policy: Policy,
    rules: Rules,
}

fn import(args: &[String]) -> Result<ExitCode, String> {
    let (root, args) = root_option(args)?;
    let mut options = ImportOptions {
        policy: Policy::new(),
        rules: Rules::new(),
        ..Default::default()
    };
    let mut i = 0;
    while i < args.len() {
        let option = args[i].as_str();
        let next = |index: &mut usize| -> Result<String, String> {
            *index += 1;
            args.get(*index)
                .cloned()
                .ok_or_else(|| format!("{option} needs a value"))
        };
        match option {
            "--case" => options.case = Some(next(&mut i)?),
            "--sheet" => options.sheet = Some(next(&mut i)?),
            "--activity" => options.activity = Some(next(&mut i)?),
            "--timestamp" => options.timestamp = Some(next(&mut i)?),
            "--event-order" => options.event_order = Some(next(&mut i)?),
            "--end-timestamp" => options.end_timestamp = Some(next(&mut i)?),
            "--resource" => options.resource = Some(next(&mut i)?),
            "--lifecycle" => options.lifecycle = Some(next(&mut i)?),
            "--cost" => options.cost = Some(next(&mut i)?),
            "--case-attribute" => options.case_attributes.push(next(&mut i)?),
            "--leave-out" => options.leave_out.push(next(&mut i)?),
            "--answer" => {
                let answer = next(&mut i)?;
                let (kind, choice) = answer
                    .split_once('=')
                    .ok_or("--answer expects <kind>=<option>")?;
                options.policy = options
                    .policy
                    .answer(super::parse_kind(kind)?, choice.to_owned());
            }
            "--rules" => {
                options.rules =
                    Rules::from_file(next(&mut i)?).map_err(|error| error.to_string())?;
            }
            "--json" => {}
            flag if flag.starts_with('-') => {
                return Err(format!("unknown workspace import option '{flag}'"));
            }
            positional if options.project.is_none() => options.project = Some(positional.into()),
            positional if options.source.is_none() => options.source = Some(positional.into()),
            _ => return Err("workspace import accepts one project and one source file".into()),
        }
        i += 1;
    }
    let mut folder = open_project(
        &root,
        options
            .project
            .as_deref()
            .ok_or("workspace import needs a project id or path")?,
    )?;
    let source = PathBuf::from(
        options
            .source
            .as_deref()
            .ok_or("workspace import needs a source file")?,
    );
    let source_name = source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("source file name is not valid UTF-8")?
        .to_owned();
    // An Excel event log is read as the CSV of its first sheet.
    let converted = tempfile::tempdir().map_err(|error| error.to_string())?;
    let opened = if pm_ingest::xlsx::is_workbook(&source_name.to_ascii_lowercase()) {
        workbook_event_log(&source, converted.path(), options.sheet.as_deref())
            .map(|(path, name, _)| (path, name))
    } else {
        as_csv(&source, &source_name, converted.path()).map(|found| match found {
            Some((path, name, _)) => (path, name),
            None => (source.clone(), source_name.clone()),
        })
    };
    let (source, source_name) = match opened {
        Ok(found) => found,
        Err(error) => return unreadable(&source, &error),
    };
    let suffix = source_suffix(&source_name)?;
    let asset = folder
        .store_asset(&source, &source_name, &suffix, "sources")
        .map_err(|error| error.to_string())?;
    let stored = folder
        .resolve_asset(&asset)
        .map_err(|error| error.to_string())?;

    let previous = folder.state().map_err(|error| error.to_string())?;
    let (state, result, code) = if is_ocel(&source_name) {
        let analysis = pm_query::analyze_ocel(&stored, &OcelFilter::default())
            .map_err(|error| error.to_string())?;
        let value = json!({
            "status":"complete","kind":"ocel","project":folder_info(&folder),
            "analysis":analysis,"saved_source":asset.relative_path
        });
        (
            WorkspaceState {
                source: Some(asset),
                preparation_sources: previous.preparation_sources,
                ocel_filter: Some(OcelFilter::default()),
                ..Default::default()
            },
            value,
            ExitCode::SUCCESS,
        )
    } else {
        let mut mapping = case_mapping(&options)?;
        // Every column besides the mapped ones is carried by default, so each
        // can be filtered and compared; listing --case-attribute narrows it,
        // --leave-out drops one.
        if let Some(mapping) = mapping.as_mut()
            && options.case_attributes.is_empty()
            && let Ok((schema, _)) = pm_ingest::sample_rows(&stored, 1)
        {
            let assigned: Vec<String> = mapping
                .mapped_columns()
                .into_iter()
                .map(str::to_owned)
                .collect();
            mapping.case_attributes = schema
                .headers()
                .iter()
                .filter(|column| !assigned.contains(column) && !options.leave_out.contains(column))
                .cloned()
                .collect();
        }
        let outcome = analyze_filtered(
            &stored,
            mapping,
            &options.policy,
            options.rules.clone(),
            Default::default(),
        )
        .map_err(|error| error.to_string())?;
        let resolved = match &outcome {
            Outcome::Complete(analysis) => analysis.mapping.clone(),
            Outcome::Blocked(blocked) => blocked.mapping.clone(),
        };
        let result = outcome_value(&folder, &outcome, &asset, &stored, &options.policy)?;
        let mut project = Project::new(&stored, resolved, options.policy, options.rules)
            .map_err(|error| error.to_string())?;
        project.name.clone_from(&folder.manifest.name);
        project.source.path.clone_from(&asset.relative_path);
        (
            WorkspaceState {
                source: Some(asset),
                preparation_sources: previous.preparation_sources,
                case_project: Some(project),
                ..Default::default()
            },
            result.0,
            result.1,
        )
    };
    folder
        .save_state(&state)
        .map_err(|error| error.to_string())?;
    let mut result = result;
    if folder.is_append_only() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos();
        let name = format!("analysis-{stamp}.json");
        save_artifact_json(&folder, "exports", &name, &result)?;
        result["saved_as"] = json!(format!(
            "exports/{name} (the project is open in a page, which shows the same figures)"
        ));
    } else {
        save_latest(&folder, &result)?;
    }
    print_json(&result)?;
    Ok(code)
}

fn analyze(args: &[String]) -> Result<ExitCode, String> {
    let (root, args) = root_option(args)?;
    let project = one_positional(&args, "workspace analyze")?;
    // A project open in a page can still be analysed again: the page owns
    // its settings, so this reads them and adds the result as new evidence.
    let folder = open_project_to_read(&root, &project)?;
    let state = folder.state().map_err(|error| error.to_string())?;
    let asset = state
        .source
        .as_ref()
        .ok_or("project has no imported data")?;
    let source = folder
        .resolve_asset(asset)
        .map_err(|error| error.to_string())?;
    let (result, code) = if let Some(project) = state.case_project {
        let outcome = analyze_filtered(
            &source,
            Some(project.mapping),
            &project.policy,
            project.rules,
            project.view_filter,
        )
        .map_err(|error| error.to_string())?;
        outcome_value(&folder, &outcome, asset, &source, &project.policy)?
    } else if let Some(filter) = state.ocel_filter {
        let analysis =
            pm_query::analyze_ocel(&source, &filter).map_err(|error| error.to_string())?;
        (
            json!({"status":"complete","kind":"ocel","project":folder_info(&folder),"analysis":analysis}),
            ExitCode::SUCCESS,
        )
    } else {
        return Err("project has data but no saved analysis settings".into());
    };
    let mut result = result;
    if folder.is_append_only() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos();
        let name = format!("analysis-{stamp}.json");
        save_artifact_json(&folder, "exports", &name, &result)?;
        result["saved_as"] = json!(format!(
            "exports/{name} (the project is open in a page, which shows the same figures)"
        ));
    } else {
        save_latest(&folder, &result)?;
    }
    print_json(&result)?;
    Ok(code)
}

fn show(args: &[String]) -> Result<ExitCode, String> {
    let (root, args) = root_option(args)?;
    let project = one_positional(&args, "workspace show")?;
    let folder = open_project_to_read(&root, &project)?;
    let state = folder.state().map_err(|error| error.to_string())?;
    let kind = if state.case_project.is_some() {
        "case_log"
    } else if state.ocel_filter.is_some() {
        "ocel"
    } else {
        "none"
    };
    print_json(&json!({
        "project":folder_info(&folder),"data_kind":kind,"source":state.source,
        "case_project":state.case_project,"ocel_filter":state.ocel_filter,
        "preparation_sources":state.preparation_sources,
        "case_model":state.case_model,"ocel_model":state.ocel_model,
        "expected_process":expected_process(&folder, state.case_model.as_ref()),
        "observation_window":state.observation_window,
        "item_names":state.item_names,
        "investigation_records":investigation::records(&folder)?,
        "saved_views":folder.saved_views().map_err(|error|error.to_string())?,
        "latest_analysis":folder.root.join("exports/latest-analysis.json")
    }))?;
    Ok(ExitCode::SUCCESS)
}

/// The project's saved expected process (an SOP), named as its author named
/// it and dated by when it was first saved, so an agent continuing the
/// project can say "you checked it against the SOP on 1 Nov".
fn expected_process(folder: &WorkspaceFolder, model: Option<&WorkspaceAsset>) -> Value {
    let Some(path) = model.and_then(|model| folder.resolve_asset(model).ok()) else {
        return Value::Null;
    };
    let name = std::fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|value| {
            value
                .get("source")
                .and_then(Value::as_str)
                .map(str::to_owned)
        });
    let set_on = std::fs::metadata(&path)
        .and_then(|meta| meta.modified())
        .ok()
        .map(|time| {
            chrono::DateTime::<chrono::Local>::from(time)
                .format("%Y-%m-%d")
                .to_string()
        });
    json!({"name":name,"set_on":set_on,
        "note":"Re-run the conformance query (no model needed) to show current results; the browser's Conformance page re-checks on opening."})
}

/// Sets the observation window and the steps that finish an item.
///
/// Extraction records the window; finishing steps come from the person who
/// knows the process, so "still open" is never guessed from the data.
fn window(args: &[String]) -> Result<ExitCode, String> {
    let (root, args) = root_option(args)?;
    let mut project = None;
    let mut start = None;
    let mut end = None;
    let mut end_steps = std::collections::BTreeSet::new();
    let mut clear = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--json" => {}
            "--clear" => clear = true,
            flag @ ("--start" | "--end" | "--end-step") => {
                i += 1;
                let value = args.get(i).ok_or_else(|| format!("{flag} needs a value"))?;
                match flag {
                    "--start" => start = Some(window_instant(value)?),
                    "--end" => end = Some(window_instant(value)?),
                    _ => {
                        if value.trim().is_empty() || value.len() > 256 {
                            return Err("--end-step needs a step name".into());
                        }
                        end_steps.insert(value.clone());
                    }
                }
            }
            value if value.starts_with('-') => {
                return Err(format!("unknown workspace window option '{value}'"));
            }
            value if project.is_none() => project = Some(value.to_owned()),
            _ => return Err("workspace window accepts one project".into()),
        }
        i += 1;
    }
    let mut folder = open_project(
        &root,
        project
            .as_deref()
            .ok_or("workspace window needs a project")?,
    )?;
    let mut state = folder.state().map_err(|error| error.to_string())?;
    let window = if clear {
        None
    } else {
        let mut window = state.observation_window.clone().unwrap_or_default();
        let had_window = state.observation_window.is_some();
        if let Some(start) = start {
            window.start_micros = start;
        }
        if let Some(end) = end {
            window.end_micros = end;
        }
        if !had_window && (start.is_none() || end.is_none()) {
            return Err("no extract window is recorded yet; give both --start and --end".into());
        }
        if window.start_micros >= window.end_micros {
            return Err("start must precede end (end is exclusive)".into());
        }
        if !end_steps.is_empty() {
            window.end_activities = end_steps;
        }
        Some(window)
    };
    state.observation_window.clone_from(&window);
    if let Some(project) = &mut state.case_project {
        project.rules.observation_window.clone_from(&window);
    }
    folder
        .save_state(&state)
        .map_err(|error| error.to_string())?;
    print_json(
        &json!({"status":"saved","project":folder_info(&folder),"observation_window":window,
        "next":"Run workspace analyze to label cases against this window."}),
    )?;
    Ok(ExitCode::SUCCESS)
}

/// Gives an item type of a connected log a business name, such as
/// "opportunity" → "Deals". Display only; analysis keeps the source names.
fn name_type(args: &[String]) -> Result<ExitCode, String> {
    let (root, args) = root_option(args)?;
    let (mut project, mut item_type, mut name, mut clear) = (None, None, None, false);
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--json" => {}
            "--clear" => clear = true,
            flag @ ("--type" | "--name") => {
                i += 1;
                let value = args
                    .get(i)
                    .ok_or_else(|| format!("{flag} needs a value"))?
                    .clone();
                if value.trim().is_empty()
                    || value.len() > 120
                    || value.chars().any(char::is_control)
                {
                    return Err(format!("{flag} needs a plain name of up to 120 characters"));
                }
                if flag == "--type" {
                    item_type = Some(value)
                } else {
                    name = Some(value)
                }
            }
            value if value.starts_with('-') => {
                return Err(format!("unknown workspace name-type option '{value}'"));
            }
            value if project.is_none() => project = Some(value.to_owned()),
            _ => return Err("workspace name-type accepts one project".into()),
        }
        i += 1;
    }
    let mut folder = open_project(
        &root,
        project
            .as_deref()
            .ok_or("workspace name-type needs a project")?,
    )?;
    let item_type = item_type.ok_or("workspace name-type needs --type <item type>")?;
    let mut state = folder.state().map_err(|error| error.to_string())?;
    match (clear, name) {
        (true, _) => {
            state.item_names.remove(&item_type);
        }
        (false, Some(name)) => {
            state.item_names.insert(item_type, name);
        }
        (false, None) => return Err("give --name <plural business name> or --clear".into()),
    }
    folder
        .save_state(&state)
        .map_err(|error| error.to_string())?;
    print_json(
        &json!({"status":"saved","project":folder_info(&folder),"item_names":state.item_names}),
    )?;
    Ok(ExitCode::SUCCESS)
}

fn window_instant(value: &str) -> Result<i64, String> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|at| at.timestamp_micros())
        .map_err(|_| "window bounds must be RFC3339 timestamps, e.g. 2026-01-01T00:00:00Z".into())
}

fn case_mapping(options: &ImportOptions) -> Result<Option<Mapping>, String> {
    match (&options.case, &options.activity, &options.timestamp) {
        (None, None, None) => Ok(None),
        (Some(case), Some(activity), Some(timestamp)) => {
            let mut mapping = Mapping::new(case, activity, timestamp);
            mapping.event_order.clone_from(&options.event_order);
            mapping.end_timestamp.clone_from(&options.end_timestamp);
            mapping.resource.clone_from(&options.resource);
            mapping.lifecycle.clone_from(&options.lifecycle);
            mapping.cost.clone_from(&options.cost);
            mapping.case_attributes.clone_from(&options.case_attributes);
            Ok(Some(mapping))
        }
        _ => Err("specify all three of --case, --activity and --timestamp, or none".into()),
    }
}

fn outcome_value(
    folder: &WorkspaceFolder,
    outcome: &Outcome,
    asset: &WorkspaceAsset,
    source: &Path,
    policy: &Policy,
) -> Result<(Value, ExitCode), String> {
    match outcome {
        Outcome::Complete(analysis) => Ok((
            json!({"status":"complete","kind":"case_log","project":folder_info(folder),"analysis":analysis,"saved_source":asset.relative_path}),
            ExitCode::SUCCESS,
        )),
        Outcome::Blocked(blocked) => {
            let quality = pm_query::assess_quality(source, Some(blocked.mapping.clone()), policy)
                .map_err(|error| error.to_string())?;
            Ok((
                json!({
                    "status":"blocked","kind":"case_log","project":folder_info(folder),
                    "quality":quality,"mapping":blocked.mapping,"saved_source":asset.relative_path,
                    "next":"Review each decision and rerun workspace import with --answer <kind>=<option>."
                }),
                ExitCode::from(3),
            ))
        }
    }
}

/// Replaces each embedded case analysis with its headline. The whole
/// analysis is saved in the project as `exports/latest-analysis.json`.
fn headlines_for_analyses(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if let Some(analysis) = map.get("analysis")
                && let Ok(analysis) = serde_json::from_value::<pm_query::Analysis>(analysis.clone())
            {
                map.remove("analysis");
                let mut headline = json!(pm_query::headline(&analysis, None, HEADLINE_ITEMS));
                drop_empty_filter_parts(&mut headline);
                map.insert("headline".to_owned(), headline);
                map.insert(
                    "full_analysis".to_owned(),
                    json!(
                        "saved in the project as exports/latest-analysis.json; or rerun with --full"
                    ),
                );
            }
            map.values_mut().for_each(headlines_for_analyses);
        }
        Value::Array(items) => items.iter_mut().for_each(headlines_for_analyses),
        _ => {}
    }
}

/// Leaves only the conditions a headline finding's filter sets: an unset
/// condition (empty, null or false) says nothing and costs the agent bytes.
/// The saved analysis keeps every filter whole.
pub(crate) fn drop_empty_filter_parts(headline: &mut Value) {
    let Some(findings) = headline.get_mut("findings").and_then(Value::as_array_mut) else {
        return;
    };
    for finding in findings {
        if let Some(Value::Object(filter)) = finding.get_mut("filter") {
            filter.retain(|_, part| match part {
                Value::Null | Value::Bool(false) => false,
                Value::Array(items) => !items.is_empty(),
                Value::Object(items) => !items.is_empty(),
                _ => true,
            });
        }
    }
}

/// Routes and hand-overs in a headline.
const HEADLINE_ITEMS: usize = 2;

fn save_latest(folder: &WorkspaceFolder, value: &Value) -> Result<(), String> {
    save_artifact_json(folder, "exports", "latest-analysis.json", value)
}

fn save_artifact_json(
    folder: &WorkspaceFolder,
    category: &str,
    name: &str,
    value: &Value,
) -> Result<(), String> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
    bytes.push(b'\n');
    folder
        .write_artifact(category, name, &bytes)
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn folder_info(folder: &WorkspaceFolder) -> Value {
    json!({
        "id":folder.manifest.id,"name":folder.manifest.name,
        "purpose":folder.manifest.purpose,"path":folder.root
    })
}

/// Opens a project for reading, even while its page has it open for writing.
fn open_project_to_read(root: &Path, value: &str) -> Result<WorkspaceFolder, String> {
    open_project(root, value).or_else(|original| {
        let given = PathBuf::from(value);
        let path = if given.join("workspace.json").is_file() {
            given
        } else if valid_workspace_id(value) {
            pm_project::folders::all_roots(root)
                .into_iter()
                .map(|root| root.join(value))
                .find(|path| path.join("workspace.json").is_file())
                .ok_or(original)?
        } else {
            return Err(original);
        };
        WorkspaceFolder::open_for_investigation(&path).map_err(|error| error.to_string())
    })
}

fn open_project(root: &Path, value: &str) -> Result<WorkspaceFolder, String> {
    let given = PathBuf::from(value);
    let path = if given.join("workspace.json").is_file() {
        given
    } else if valid_workspace_id(value) {
        // Not in this folder: a remembered folder may hold it.
        pm_project::folders::all_roots(root)
            .into_iter()
            .map(|root| root.join(value))
            .find(|path| path.join("workspace.json").is_file())
            .unwrap_or_else(|| root.join(value))
    } else {
        return Err("project must be a folder path or 32-character project id".into());
    };
    WorkspaceFolder::open(&path).map_err(|error| error.to_string())
}

fn source_suffix(name: &str) -> Result<String, String> {
    let lower = name.to_ascii_lowercase();
    for suffix in [
        ".xes.gz",
        ".jsonocel",
        ".xmlocel",
        ".sqlite3",
        ".parquet",
        ".sqlite",
        ".csv",
        ".xes",
        ".xml",
    ] {
        if lower.ends_with(suffix) {
            return Ok(suffix.into());
        }
    }
    Err("unsupported source; use CSV, Parquet, XES, XES.gz, or OCEL JSON/XML/SQLite".into())
}

fn is_ocel(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [".jsonocel", ".xmlocel", ".sqlite", ".sqlite3", ".xml"]
        .iter()
        .any(|suffix| lower.ends_with(suffix))
}

fn one_positional(args: &[String], command: &str) -> Result<String, String> {
    let positional = args
        .iter()
        .filter(|arg| arg.as_str() != "--json")
        .collect::<Vec<_>>();
    match positional.as_slice() {
        [value] if !value.starts_with('-') => Ok((*value).clone()),
        _ => Err(format!("{command} needs exactly one project id or path")),
    }
}

/// Prints an answer for an agent: compact JSON, held to the agent budget
/// unless `--full` was given. Saved artifacts keep the whole answer.
fn print_json(value: &Value) -> Result<(), String> {
    let value = if FULL_OUTPUT.load(std::sync::atomic::Ordering::Relaxed) {
        value.clone()
    } else {
        let mut value = value.clone();
        headlines_for_analyses(&mut value);
        // A query's saved scope (sources, hashes, settings) is provenance kept
        // in its artifact; the agent does not need it echoed every time.
        if let Some(response) = value.get_mut("response").and_then(Value::as_object_mut)
            && response.remove("scope").is_some()
        {
            response.insert("scope".to_owned(), json!("saved with the artifact"));
        }
        pm_query::budget::fit(
            value,
            pm_query::budget::AGENT_BUDGET,
            "run the same command again with --full, or read the saved artifact it names",
        )
    };
    println!(
        "{}",
        serde_json::to_string(&value).map_err(|error| error.to_string())?
    );
    Ok(())
}
