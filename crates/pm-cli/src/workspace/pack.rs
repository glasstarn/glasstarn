//! `workspace pack`: the consultant workflow around a process pack. The pack
//! lists what to export, the data check finds gaps and questions, the person's
//! answers are saved once in the project, and the build turns the pack and
//! those answers into a connected log. A refresh is a new inventory, check
//! and build; saved answers are reused and only new questions are asked.
use super::{folder_info, open_project, print_json, root_option, save_artifact_json};
use pm_ingest::pack::{
    Answer, BUILT_IN, CheckReport, Pack, SET_ASIDE, build_recipe_with, check, set_aside_question,
    validate_answer,
};
use pm_project::workspace::{WorkspaceAsset, WorkspaceFolder};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;

pub(super) fn command(args: &[String]) -> Result<ExitCode, String> {
    match args.first().map(String::as_str) {
        Some("list") => list(),
        Some("checklist") => checklist(&args[1..]),
        Some("check") => run_check(&args[1..]),
        Some("answer") => answer(&args[1..]),
        Some("build") => build(&args[1..]),
        Some("combine") => combine(&args[1..]),
        Some("handout") => handout(&args[1..]),
        Some("reshape") => reshape(&args[1..]),
        _ => Err(
            "workspace pack expects list, checklist <pack>, handout, combine, check, answer or build"
                .into(),
        ),
    }
}

/// A built-in pack by id, or a community pack file by path.
fn load(name: &str) -> Result<Pack, String> {
    if let Some((_, text)) = BUILT_IN.iter().find(|(id, _)| *id == name) {
        return Pack::parse(text).map_err(|e| e.to_string());
    }
    let path = PathBuf::from(name);
    if path.is_file() {
        let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        return Pack::parse(&text).map_err(|e| e.to_string());
    }
    Err(format!(
        "unknown pack '{name}'; built-in packs: {}",
        BUILT_IN.map(|(id, _)| id).join(", ")
    ))
}

fn list() -> Result<ExitCode, String> {
    let packs = BUILT_IN
        .iter()
        .map(|(_, text)| {
            let pack = Pack::parse(text).map_err(|e| e.to_string())?;
            Ok(json!({"id":pack.id,"name":pack.name,"system":pack.system,"process":pack.process,"status":pack.status,
                "variants":pack.variants.iter().map(|v| json!({"id":v.id,"name":v.name})).collect::<Vec<_>>(),
                "link_targets":pack.link_targets.iter().map(|t| json!({"id":t.id,"label":t.label})).collect::<Vec<_>>()}))
        })
        .collect::<Result<Vec<_>, String>>()?;
    // The whole catalog, never shortened: an agent needs every pack to pick
    // the right one, and the list is bounded by the packs that ship.
    println!("{}", json!({ "packs": packs }));
    Ok(ExitCode::SUCCESS)
}

fn checklist(args: &[String]) -> Result<ExitCode, String> {
    let name = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .ok_or("workspace pack checklist needs a pack")?;
    let mut pack = load(name)?;
    if let Some(at) = args.iter().position(|a| a == "--variant") {
        let variant = args.get(at + 1).ok_or("--variant needs a value")?;
        pack = pack.with_variant(variant).map_err(|e| e.to_string())?;
    }
    print_json(&pack.checklist())?;
    Ok(ExitCode::SUCCESS)
}

/// `workspace pack reshape <project> --pack <pack> --table <table> --file
/// <export.xlsx|csv> [--column <pack column>=<export column>]... [--time-zone
/// <America/New_York | ±HH:MM>] [--day-first yes]`: an export as a person made it (labels for column
/// names, local times, title rows) rewritten as the pack's file for that
/// table and added to the project. The newest copy of a table wins, so the
/// check that follows reads it. The agent proposes the column matches from
/// the check's `column_matches`; the person confirms them.
fn reshape(args: &[String]) -> Result<ExitCode, String> {
    let options = options(
        args,
        "reshape",
        &["table", "file", "column", "time-zone", "day-first"],
    )?;
    let table_name = one(&options, "table").ok_or("workspace pack reshape needs --table")?;
    let table = options
        .pack
        .tables
        .iter()
        .find(|t| t.name == table_name)
        .ok_or(format!(
            "pack {} has no table {table_name}",
            options.pack.id
        ))?;
    let source = PathBuf::from(one(&options, "file").ok_or("workspace pack reshape needs --file")?);
    let mut plan = pm_ingest::reshape::Plan {
        day_first: one(&options, "day-first").is_some_and(|v| v == "true" || v == "yes"),
        ..Default::default()
    };
    for given in options.flags.get("column").into_iter().flatten() {
        let (field, column) = given
            .split_once('=')
            .ok_or("--column expects <pack column>=<export column>")?;
        if !table
            .required
            .iter()
            .chain(&table.fields)
            .any(|f| f == field)
        {
            return Err(format!("{field} is not a column of {table_name}"));
        }
        plan.rename
            .insert(column.trim().to_owned(), field.to_owned());
    }
    if let Some(zone) = one(&options, "time-zone") {
        plan.zone = Some(
            match chrono::DateTime::parse_from_rfc3339(&format!("2000-01-01T00:00:00{zone}")) {
                Ok(fixed) => pm_ingest::reshape::Zone::Fixed(fixed.timezone()),
                Err(_) => pm_ingest::reshape::Zone::Named(pm_ingest::reshape::Named::load(zone)?),
            },
        );
    }
    let rows = read_export(&source)?;
    let rows = &pm_ingest::xlsx::table(&rows);
    let (header, body, rewritten) = pm_ingest::reshape::apply(rows, &plan);
    let still = table
        .required
        .iter()
        .filter(|f| !header.contains(f))
        .collect::<Vec<_>>();
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let out = dir.path().join(format!("{table_name}.csv"));
    let mut writer = csv::Writer::from_path(&out).map_err(|e| e.to_string())?;
    writer.write_record(&header).map_err(|e| e.to_string())?;
    for row in &body {
        writer.write_record(row).map_err(|e| e.to_string())?;
    }
    writer.flush().map_err(|e| e.to_string())?;
    drop(writer);
    let added = super::inventory_paths(&options.root, &options.project, &[out])?;
    let folder = open_project(&options.root, &options.project)?;
    pm_project::audit::append(
        &folder.root,
        "pack_reshape",
        json!({"pack": options.pack.id, "table": table_name, "export": source.file_name().map(|n| n.to_string_lossy().into_owned()),
            "renamed": plan.rename, "time_zone": one(&options, "time-zone"), "day_first": plan.day_first,
            "rows": body.len(), "asset": added["inputs"][0]["asset"]["sha256"]}),
    )?;
    print_json(&json!({
        "status": "saved",
        "table": table_name,
        "rows": body.len(),
        "columns": header,
        "times_rewritten": rewritten,
        "still_missing": still,
        "asset": added["inputs"][0]["asset"],
        "next": if still.is_empty() {
            "Run workspace pack check again; this copy of the table is the newest and is the one it reads."
        } else {
            "Some required columns are still missing; ask the person which export columns hold them, or ask the administrator for them."
        },
    }))?;
    Ok(ExitCode::SUCCESS)
}

/// The rows of an export: the first sheet of an Excel workbook, or a CSV.
pub(super) fn read_export(path: &std::path::Path) -> Result<Vec<Vec<String>>, String> {
    let lower = path.to_string_lossy().to_ascii_lowercase();
    let head = std::fs::read(path)
        .map(|b| b.into_iter().take(8).collect::<Vec<_>>())
        .unwrap_or_default();
    if pm_ingest::xlsx::is_workbook(&lower)
        || head.starts_with(b"PK\x03\x04")
        || head.starts_with(&[0xD0, 0xCF, 0x11, 0xE0])
    {
        return pm_ingest::xlsx::read(path).map_err(|e| e.to_string());
    }
    // Another encoding or separator is read as the same table in UTF-8 CSV.
    let converted = pm_ingest::text_export::as_utf8_csv(path)?;
    let bytes = match converted {
        Some((bytes, _)) => bytes,
        None => std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?,
    };
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_reader(bytes.as_slice());
    reader
        .records()
        .map(|r| {
            r.map(|r| r.iter().map(str::to_owned).collect())
                .map_err(|e| format!("cannot read {}: {e}", path.display()))
        })
        .collect()
}

/// `workspace pack handout <project> --pack <pack> [--from DATE] [--to DATE]
/// [--scope VALUE]... [--variant ID] [--purpose TEXT] [--note TEXT]...`: the export request the
/// person forwards to the system administrator, saved in the project as
/// `exports/<pack>-export-request.md`.
fn handout(args: &[String]) -> Result<ExitCode, String> {
    let options = options(
        args,
        "handout",
        &["from", "to", "scope", "variant", "purpose"],
    )?;
    let mut pack = options.pack.clone();
    if let Some(variant) = one(&options, "variant") {
        pack = pack.with_variant(variant).map_err(|e| e.to_string())?;
    }
    let day = |name: &str| -> Result<Option<String>, String> {
        one(&options, name)
            .map(|v| {
                chrono::NaiveDate::parse_from_str(v, "%Y-%m-%d")
                    .map(|d| d.to_string())
                    .map_err(|_| format!("--{name} expects a date such as 2026-01-31"))
            })
            .transpose()
    };
    let (from, to) = (day("from")?, day("to")?);
    if let (Some(from), Some(to)) = (&from, &to)
        && from > to
    {
        return Err("--from must be on or before --to".into());
    }
    let scope = options.flags.get("scope").cloned().unwrap_or_default();
    if scope.iter().any(|v| v.trim().is_empty() || v.len() > 40) {
        return Err("--scope expects values such as a company code".into());
    }
    let folder = open_project(&options.root, &options.project)?;
    let request = pm_ingest::handout::Request {
        purpose: one(&options, "purpose")
            .map(str::to_owned)
            .unwrap_or_else(|| folder.manifest.name.clone()),
        from,
        to,
        scope,
        today: chrono::Utc::now().date_naive().to_string(),
        notes: options.flags.get("note").cloned().unwrap_or_default(),
    };
    let text = pack.handout(&request);
    let name = format!("{}-export-request.md", pack.id.replace('+', "_and_"));
    folder
        .write_artifact("exports", &name, text.as_bytes())
        .map_err(|e| e.to_string())?;
    print_json(&json!({
        "status": "saved",
        "file": folder.root.join("exports").join(&name),
        "pack": pack.id,
        "tables": pack.tables.iter().map(|t| format!("{}.csv", t.name)).collect::<Vec<_>>(),
        "project": folder_info(&folder),
        "next": "Give the person this file to forward to the system administrator. When the files come back, inventory them and run workspace pack check.",
    }))?;
    Ok(ExitCode::SUCCESS)
}

/// Whether a file is named after a table: `EKKO.csv`, `ekko_2026.csv`, or a
/// table whose own name has underscores, such as `SBQQ__Quote__c.csv`.
fn named_after(file: &str, table: &str) -> bool {
    let stem = file
        .rsplit_once('.')
        .map_or(file, |(s, _)| s)
        .to_ascii_lowercase();
    let table = table.to_ascii_lowercase();
    let boundary = |c: Option<char>| c.is_none_or(|c| !c.is_ascii_alphanumeric());
    stem.match_indices(&table).any(|(at, _)| {
        boundary(stem[..at].chars().next_back())
            && boundary(stem[at + table.len()..].chars().next())
    }) && !(table.chars().all(|c| c.is_ascii_alphanumeric())
        && !stem
            .split(|c: char| !c.is_ascii_alphanumeric())
            .any(|token| token == table))
}

/// Whether a file's name is the table's name and nothing else but numbers
/// and separators.
fn exactly_named(file: &str, table: &str) -> bool {
    let stem = file
        .rsplit_once('.')
        .map_or(file, |(s, _)| s)
        .to_ascii_lowercase();
    let rest = stem.replacen(&table.to_ascii_lowercase(), "", 1);
    rest.len() < stem.len() && !rest.chars().any(char::is_alphabetic)
}

struct Options {
    root: PathBuf,
    project: String,
    pack: Pack,
    flags: BTreeMap<String, Vec<String>>,
}

/// `<project> --pack <pack>` plus repeatable `--name value` flags.
fn options(args: &[String], command: &str, allowed: &[&str]) -> Result<Options, String> {
    let (root, args) = root_option(args)?;
    let mut project = None;
    let mut flags = BTreeMap::<String, Vec<String>>::new();
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        if arg == "--json" {
        } else if let Some(name) = arg.strip_prefix("--") {
            if name != "pack" && !allowed.contains(&name) {
                return Err(format!("unknown workspace pack {command} option '{arg}'"));
            }
            i += 1;
            let value = args.get(i).ok_or(format!("{arg} needs a value"))?;
            flags
                .entry(name.to_owned())
                .or_default()
                .push(value.clone());
        } else if project.replace(arg.to_owned()).is_some() {
            return Err(format!("workspace pack {command} accepts one project"));
        }
        i += 1;
    }
    let pack = flags
        .get("pack")
        .and_then(|v| v.first())
        .ok_or(format!("workspace pack {command} needs --pack <pack>"))?;
    let project = project.ok_or(format!("workspace pack {command} needs a project"))?;
    // A combined pack lives in the project that combined it.
    let pack = if pack.contains('+') {
        let folder = open_project(&root, &project)?;
        let saved = read_artifact(&folder, &artifact_for(pack, "definition")).ok_or(format!(
            "pack {pack} has not been combined in this project; run workspace pack combine first"
        ))?;
        serde_json::from_value::<Pack>(saved["pack"].clone())
            .map_err(|e| format!("the combined pack is unreadable: {e}"))?
    } else {
        load(pack)?
    };
    Ok(Options {
        root,
        pack,
        project,
        flags,
    })
}

fn one<'a>(options: &'a Options, name: &str) -> Option<&'a str> {
    options
        .flags
        .get(name)
        .and_then(|v| v.last())
        .map(String::as_str)
}

fn artifact(pack: &Pack, kind: &str) -> String {
    artifact_for(&pack.id, kind)
}

/// Artifact names allow no `+`, so a combined pack's `a+b` becomes `a_and_b`.
fn artifact_for(id: &str, kind: &str) -> String {
    format!("pack-{}-{kind}.json", id.replace('+', "_and_"))
}

fn read_artifact(folder: &WorkspaceFolder, name: &str) -> Option<Value> {
    let path = folder.root.join("preparation").join(name);
    let metadata = std::fs::symlink_metadata(&path).ok()?;
    if !metadata.is_file() || metadata.len() > 16 * 1024 * 1024 {
        return None;
    }
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

fn saved_answers(
    folder: &WorkspaceFolder,
    pack: &Pack,
) -> Result<BTreeMap<String, Answer>, String> {
    match read_artifact(folder, &artifact(pack, "answers")) {
        None => Ok(BTreeMap::new()),
        Some(value) => serde_json::from_value(value["answers"].clone())
            .map_err(|e| format!("saved answers are unreadable: {e}")),
    }
}

/// Matches each pack table to one inventoried file: a file named after the
/// table (`EKKO.csv`, `ekko_2026.csv`), preferring the copy from the most
/// recent inventory or completed extract when a refresh added a newer one.
/// `--file TABLE=<sha256>` decides by hand.
fn tables(
    folder: &WorkspaceFolder,
    pack: &Pack,
    overrides: &[String],
) -> Result<BTreeMap<String, (String, String)>, String> {
    let state = folder.state().map_err(|e| e.to_string())?;
    let latest = read_artifact(folder, "inventory.json")
        .map(|value| {
            value["inputs"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|input| input["asset"]["sha256"].as_str().map(str::to_owned))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    // When a file was added to the project: its stored copy's time.
    let added = |asset: &WorkspaceAsset| {
        std::fs::metadata(folder.root.join(&asset.relative_path))
            .and_then(|m| m.modified())
            .ok()
    };
    let mut chosen = BTreeMap::new();
    for given in overrides {
        let (table, sha) = given
            .split_once('=')
            .ok_or("--file expects TABLE=<sha256>")?;
        let asset = state
            .preparation_sources
            .iter()
            .find(|a| a.sha256 == sha)
            .ok_or(format!(
                "--file {table}: no inventoried file has that SHA-256"
            ))?;
        chosen.insert(table.to_owned(), (asset.sha256.clone(), asset.name.clone()));
    }
    for table in &pack.tables {
        if chosen.contains_key(&table.name) {
            continue;
        }
        let candidates = state
            .preparation_sources
            .iter()
            .filter(|asset| named_after(&asset.name, &table.name))
            .collect::<Vec<_>>();
        // Several copies: the newest wins, unless it came in the latest
        // inventory together with another copy, which stays a question.
        let candidates = match candidates.iter().max_by_key(|a| added(a)) {
            Some(newest) if candidates.len() > 1 => {
                if latest.contains(&newest.sha256) {
                    candidates
                        .iter()
                        .copied()
                        .filter(|a| latest.contains(&a.sha256))
                        .collect()
                } else {
                    vec![*newest]
                }
            }
            _ => candidates,
        };
        // Of several, one named exactly after the table (give or take numbers
        // and separators, as in "38 Purchase Header") beats one that only
        // starts with it ("Purchase Header Archive").
        let exact = candidates
            .iter()
            .copied()
            .filter(|a| exactly_named(&a.name, &table.name))
            .collect::<Vec<_>>();
        let candidates = if candidates.len() > 1 && exact.len() == 1 {
            exact
        } else {
            candidates
        };
        match candidates.as_slice() {
            [] => {}
            [asset] => {
                chosen.insert(
                    table.name.clone(),
                    (asset.sha256.clone(), asset.name.clone()),
                );
            }
            many => {
                return Err(format!(
                    "several files could be table {}: {}; choose one with --file {}=<sha256>",
                    table.name,
                    many.iter()
                        .map(|a| format!("{} ({})", a.name, &a.sha256[..12]))
                        .collect::<Vec<_>>()
                        .join(", "),
                    table.name
                ));
            }
        }
    }
    Ok(chosen)
}

/// For a table whose pack names an archive (orders a system moves out of the
/// open table once done): when the project has both files, one file of the
/// open rows and, for keys not among them, the latest archived version.
/// It is added to the project and used as the table's file.
fn merge_archives(
    root: &std::path::Path,
    project: &str,
    pack: &Pack,
    chosen: &mut BTreeMap<String, (String, String)>,
) -> Result<(), String> {
    let mut folder = open_project(root, project)?;
    for table in &pack.tables {
        let Some(archive) = &table.archive else {
            continue;
        };
        let Some((sha, name)) = chosen.get(&table.name).cloned() else {
            continue;
        };
        let state = folder.state().map_err(|e| e.to_string())?;
        let Some(archived) = state
            .preparation_sources
            .iter()
            .filter(|a| named_after(&a.name, &archive.table) && a.sha256 != sha)
            .max_by_key(|a| {
                std::fs::metadata(folder.root.join(&a.relative_path))
                    .and_then(|m| m.modified())
                    .ok()
            })
            .cloned()
        else {
            continue;
        };
        let open_asset = state
            .preparation_sources
            .iter()
            .find(|a| a.sha256 == sha)
            .ok_or("inventoried file disappeared")?
            .clone();
        let read = |asset: &WorkspaceAsset| -> Result<(Vec<String>, Vec<Vec<String>>), String> {
            let path = folder.resolve_asset(asset).map_err(|e| e.to_string())?;
            let mut reader = csv::Reader::from_path(&path).map_err(|e| e.to_string())?;
            let headers = reader
                .headers()
                .map_err(|e| e.to_string())?
                .iter()
                .map(|h| h.trim_start_matches('\u{feff}').to_owned())
                .collect::<Vec<_>>();
            let rows = reader
                .records()
                .map(|r| {
                    r.map(|r| r.iter().map(str::to_owned).collect())
                        .map_err(|e| e.to_string())
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok((headers, rows))
        };
        let (open_headers, open_rows) = read(&open_asset)?;
        let (archive_headers, archive_rows) = read(&archived)?;
        let at = |headers: &[String], column: &str| headers.iter().position(|h| h == column);
        let key_of = |headers: &[String], row: &[String]| -> Option<Vec<String>> {
            archive
                .key
                .iter()
                .map(|k| at(headers, k).map(|i| row.get(i).cloned().unwrap_or_default()))
                .collect()
        };
        let mut columns = open_headers.clone();
        for h in &archive_headers {
            if !columns.contains(h) {
                columns.push(h.clone());
            }
        }
        let mut seen = std::collections::BTreeSet::new();
        for row in &open_rows {
            if let Some(key) = key_of(&open_headers, row) {
                seen.insert(key);
            }
        }
        // The latest archived version of each key not still open.
        let version = at(&archive_headers, &archive.version);
        let mut latest = BTreeMap::<Vec<String>, (i64, usize)>::new();
        for (i, row) in archive_rows.iter().enumerate() {
            let Some(key) = key_of(&archive_headers, row) else {
                return Err(format!(
                    "{} lacks the columns {}",
                    archived.name,
                    archive.key.join(", ")
                ));
            };
            if seen.contains(&key) {
                continue;
            }
            let v = version
                .and_then(|at| row.get(at))
                .and_then(|v| v.trim().parse::<i64>().ok())
                .unwrap_or(0);
            let entry = latest.entry(key).or_insert((v, i));
            if v > entry.0 {
                *entry = (v, i);
            }
        }
        let mut rows = latest.values().map(|(_, i)| *i).collect::<Vec<_>>();
        rows.sort_unstable();
        let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
        let merged_name = format!("{} with archive.csv", table.name);
        let path = dir.path().join(&merged_name);
        let mut writer = csv::Writer::from_path(&path).map_err(|e| e.to_string())?;
        writer.write_record(&columns).map_err(|e| e.to_string())?;
        let place = |headers: &[String], row: &[String]| {
            columns
                .iter()
                .map(|c| {
                    at(headers, c)
                        .and_then(|i| row.get(i))
                        .cloned()
                        .unwrap_or_default()
                })
                .collect::<Vec<_>>()
        };
        for row in &open_rows {
            writer
                .write_record(place(&open_headers, row))
                .map_err(|e| e.to_string())?;
        }
        for i in rows {
            writer
                .write_record(place(&archive_headers, &archive_rows[i]))
                .map_err(|e| e.to_string())?;
        }
        writer.flush().map_err(|e| e.to_string())?;
        drop(writer);
        let asset = folder
            .store_asset(&path, &merged_name, ".csv", "sources")
            .map_err(|e| e.to_string())?;
        let mut state = folder.state().map_err(|e| e.to_string())?;
        if !state
            .preparation_sources
            .iter()
            .any(|a| a.sha256 == asset.sha256)
        {
            state.preparation_sources.push(asset.clone());
            folder.save_state(&state).map_err(|e| e.to_string())?;
        }
        chosen.insert(
            table.name.clone(),
            (asset.sha256.clone(), format!("{name} + {}", archived.name)),
        );
    }
    Ok(())
}

/// The agent's view of a check: gaps, a line per table and link, and the
/// questions still open. The whole report is saved in the project.
fn summary(report: &CheckReport, answers: &BTreeMap<String, Answer>) -> Value {
    let open = report
        .questions
        .iter()
        .filter(|q| !answers.contains_key(&q.id))
        .collect::<Vec<_>>();
    json!({
        "status": report.status,
        "gaps": report.gaps,
        "data_problems": report.data_problems,
        // Missing pieces the build goes ahead without, with what it loses.
        "left_out": report.left_out,
        // One line per table: rows, dates, scope and the company's own fields.
        "tables": report.tables.iter().map(|t| {
            let mut line = format!("{}: {} rows", t.table, t.rows);
            if let Some(first) = &t.first_date {
                line.push_str(&format!(", {first} to {}", t.last_date.as_deref().unwrap_or("?")));
            }
            if !t.scope.is_empty() {
                let scope = t.scope.iter().map(|(v, n)| format!("{v} ({n})")).collect::<Vec<_>>();
                line.push_str(&format!(", scope {}", scope.join(" ")));
            }
            for (field, column) in &t.fallbacks_used {
                line.push_str(&format!(", {field} not exported so {column} is used"));
            }
            if !t.custom_fields.is_empty() {
                let own = t.custom_fields.iter().map(|f| f.column.as_str()).collect::<Vec<_>>();
                line.push_str(&format!(", own fields {}", own.join(" ")));
            }
            line
        }).collect::<Vec<_>>(),
        // A table whose export names its columns differently: its columns
        // and the likely matches, to propose to the person.
        "column_matches": report.tables.iter().filter(|t| !t.missing_fields.is_empty()).map(|t| json!({
            "table": t.table, "missing": t.missing_fields, "suggested": t.suggested, "columns": t.columns,
        })).collect::<Vec<_>>(),
        "missing_tables": report.missing_tables,
        // Only the links that did not fully match; the rest are counted.
        "links_fully_matched": report.links.iter().filter(|l| l.missing == 0).count(),
        "links_with_gaps": report.links.iter().filter(|l| l.missing > 0).map(|l| format!("{}: {} of {} ({:.1}%)", l.label, l.matched, l.rows, l.share * 100.0)).collect::<Vec<_>>(),
        "open_questions": open,
        "answered": answers.keys().collect::<Vec<_>>(),
    })
}

fn run_check(args: &[String]) -> Result<ExitCode, String> {
    let options = options(args, "check", &["file", "variant"])?;
    let folder = open_project(&options.root, &options.project)?;
    let variant = one(&options, "variant").map(str::to_owned);
    let narrowed = match &variant {
        Some(id) => options.pack.with_variant(id).map_err(|e| e.to_string())?,
        None => options.pack.clone(),
    };
    // Answered settings can name fields (a company's own field), so the
    // check reads them filled in.
    let narrowed = narrowed
        .with_settings(&saved_answers(&folder, &narrowed)?)
        .map_err(|e| e.to_string())?;
    let pack = &narrowed;
    let mut chosen = tables(
        &folder,
        pack,
        options.flags.get("file").map_or(&[][..], Vec::as_slice),
    )?;
    // The merge adds a file to the project, which needs it opened anew.
    drop(folder);
    merge_archives(&options.root, &options.project, pack, &mut chosen)?;
    let folder = open_project(&options.root, &options.project)?;
    let state = folder.state().map_err(|e| e.to_string())?;
    let mut files = BTreeMap::new();
    for (table, (sha, _)) in &chosen {
        let asset = state
            .preparation_sources
            .iter()
            .find(|a| &a.sha256 == sha)
            .ok_or("inventoried file disappeared")?;
        files.insert(
            table.clone(),
            folder.resolve_asset(asset).map_err(|e| e.to_string())?,
        );
    }
    let report = check(pack, &files).map_err(|e| e.to_string())?;
    let answers = saved_answers(&folder, pack)?;
    let tables = chosen
        .iter()
        .map(|(table, (sha, name))| (table.clone(), json!({"sha256": sha, "file": name})))
        .collect::<serde_json::Map<_, _>>();
    save_artifact_json(
        &folder,
        "preparation",
        &artifact(pack, "check"),
        &json!({"pack": pack.id, "variant": variant, "pack_status": pack.status, "files": tables, "report": report}),
    )?;
    let mut value = summary(&report, &answers);
    value["project"] = folder_info(&folder);
    value["pack"] =
        json!({"id": pack.id, "variant": variant, "name": pack.name, "status": pack.status});
    value["next"] = json!(if report.status == "ready"
        && !report.data_problems.is_empty()
        && !answers.contains_key(SET_ASIDE)
    {
        "Some records do not fit (data_problems). Ask the data:set-aside question first, in business words with the counts: leave them out, build with the rest and list them as a data-quality finding (recommended when they are a small share), or ask for a corrected export. Then ask the other open questions one at a time, save each with workspace pack answer, and run workspace pack build."
    } else if report.status == "ready" {
        "Ask the open questions one at a time in business words, with the evidence and the recommended answer; save each with workspace pack answer, then run workspace pack build."
    } else if report.tables.iter().any(|t| !t.suggested.is_empty()) {
        "Some exports name their columns differently: propose the matches in column_matches to the person (and ask which columns hold any it could not match, and the time zone of local times), then run workspace pack reshape for each and check again. For tables or fields that are truly missing, tell the person what the administrator must add or re-export."
    } else {
        "Tell the person which tables or fields the administrator must add or re-export; then inventory the new files and check again."
    });
    if report.status == "ready" && !report.left_out.is_empty() {
        let next = value["next"].as_str().unwrap_or_default().to_owned();
        value["next"] = json!(format!(
            "Some tables or columns are missing but the build can do without them (left_out): tell the person in one line each what the analysis will not show, and that a fuller export can be added later. {next}"
        ));
    }
    print_json(&value)?;
    Ok(if report.status == "ready" {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(3)
    })
}

fn answer(args: &[String]) -> Result<ExitCode, String> {
    let options = options(
        args,
        "answer",
        &["question", "choice", "value", "confirmed-by"],
    )?;
    let folder = open_project(&options.root, &options.project)?;
    let pack = &options.pack;
    let id = one(&options, "question").ok_or("workspace pack answer needs --question <id>")?;
    let choice = one(&options, "choice").ok_or("workspace pack answer needs --choice <option>")?;
    let confirmed_by = one(&options, "confirmed-by")
        .map(str::trim)
        .filter(|v| !v.is_empty() && v.len() <= 120)
        .ok_or("workspace pack answer needs --confirmed-by with the name or role of the person who answered")?;
    let value = one(&options, "value");
    let saved = read_artifact(&folder, &artifact(pack, "check"))
        .ok_or("run workspace pack check first; answers are checked against its questions")?;
    let question = saved["report"]["questions"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|q| q["id"] == id)
        .ok_or(format!("the latest check asked no question '{id}'"))?;
    let question: pm_ingest::pack::Question =
        serde_json::from_value(question.clone()).map_err(|e| e.to_string())?;
    validate_answer(&question, choice, value).map_err(|e| e.to_string())?;
    if question.kind == "setting" && id.ends_with("offset") {
        let offset = value.unwrap_or_default().trim();
        let probe = format!("2000-01-01T00:00:00{offset}");
        if offset != "Z" && chrono::DateTime::parse_from_rfc3339(&probe).is_err() {
            return Err("the offset must look like +01:00, -05:00 or Z".into());
        }
    }
    let mut answers = saved_answers(&folder, pack)?;
    let answer = Answer {
        choice: choice.to_owned(),
        value: value.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty()),
        confirmed_by: confirmed_by.to_owned(),
        at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        question: question.text.clone(),
        carries: question.carries.clone(),
    };
    let replaced = answers.insert(id.to_owned(), answer.clone());
    save_artifact_json(
        &folder,
        "preparation",
        &artifact(pack, "answers"),
        &json!({"pack": pack.id, "answers": answers}),
    )?;
    pm_project::audit::append(
        &folder.root,
        "pack_answer",
        json!({"pack": pack.id, "question": id, "choice": answer.choice, "value": answer.value,
            "confirmed_by": answer.confirmed_by, "replaced": replaced.is_some()}),
    )?;
    let open = saved["report"]["questions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|q| q["id"].as_str().is_some_and(|q| !answers.contains_key(q)))
        .count();
    print_json(
        &json!({"status":"saved","question":id,"answer":answer,"replaced":replaced.is_some(),
        "open_questions":open,"project":folder_info(&folder)}),
    )?;
    Ok(ExitCode::SUCCESS)
}

/// Combines two packs for a company that uses both systems, joined where a
/// table of the first holds the number of an item of the second. The
/// combined pack is kept in the project; check, answer and build then take
/// `--pack <first>+<second>`.
fn combine(args: &[String]) -> Result<ExitCode, String> {
    let options = options(args, "combine", &["with", "link"])?;
    let folder = open_project(&options.root, &options.project)?;
    let other = load(one(&options, "with").ok_or("workspace pack combine needs --with <pack>")?)?;
    let link = one(&options, "link")
        .ok_or("workspace pack combine needs --link <table>:<link target>, for example Record:purchase_order")?;
    let (table, target) = link
        .split_once(':')
        .ok_or("--link expects <table>:<link target>")?;
    let combined = options
        .pack
        .combine(&other, table, target)
        .map_err(|e| e.to_string())?;
    save_artifact_json(
        &folder,
        "preparation",
        &artifact(&combined, "definition"),
        &json!({"pack": combined, "from": [options.pack.id, other.id], "link": {"table": table, "target": target}}),
    )?;
    pm_project::audit::append(
        &folder.root,
        "pack_combine",
        json!({"pack": combined.id, "from": [options.pack.id, other.id], "table": table, "target": target}),
    )?;
    print_json(&json!({
        "status": "combined",
        "pack": {"id": combined.id, "name": combined.name, "status": combined.status,
                 "case_object_type": combined.case_object_type},
        "checklist": combined.checklist(),
        "project": folder_info(&folder),
        "next": format!("Have both systems' tables exported, inventory them, then run workspace pack check with --pack {}. It asks which {table} field holds the number and reports how many rows find their item.", combined.id),
    }))?;
    Ok(ExitCode::SUCCESS)
}

fn build(args: &[String]) -> Result<ExitCode, String> {
    let options = options(args, "build", &[])?;
    let folder = open_project(&options.root, &options.project)?;
    let saved = read_artifact(&folder, &artifact(&options.pack, "check"))
        .ok_or("run workspace pack check first")?;
    // The build follows the variant the data was checked against.
    let variant = saved["variant"].as_str().map(str::to_owned);
    let narrowed = match &variant {
        Some(id) => options.pack.with_variant(id).map_err(|e| e.to_string())?,
        None => options.pack.clone(),
    };
    let pack = &narrowed;
    if saved["report"]["status"] != "ready" {
        return Err(format!(
            "the latest data check found export gaps: {}",
            saved["report"]["gaps"]
        ));
    }
    let hashes = saved["files"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(table, file)| Some((table.clone(), file["sha256"].as_str()?.to_owned())))
        .collect::<BTreeMap<_, _>>();
    let answers = saved_answers(&folder, pack)?;
    let problems = saved["report"]["data_problems"]
        .as_array()
        .map_or(0, Vec::len);
    match answers.get(SET_ASIDE).map(|a| a.choice.as_str()) {
        None if problems > 0 => {
            return Err(format!(
                "some records do not fit: {}. Ask the person the {SET_ASIDE} question (leave them out and list them as a finding, or ask for a corrected export), save it with workspace pack answer, then build again",
                saved["report"]["data_problems"]
            ));
        }
        Some("new_export") => {
            return Err("the person chose a corrected export for the records that do not fit; inventory the new files and check again".into());
        }
        _ => {}
    }
    // The build may still find records that do not fit; keep the question
    // answerable so the person can decide then.
    let mut saved = saved;
    let asked = saved["report"]["questions"]
        .as_array()
        .is_some_and(|q| q.iter().any(|q| q["id"] == SET_ASIDE));
    if !asked {
        if let Some(questions) = saved["report"]["questions"].as_array_mut() {
            questions
                .push(serde_json::to_value(set_aside_question(&[])).map_err(|e| e.to_string())?);
        }
        save_artifact_json(&folder, "preparation", &artifact(pack, "check"), &saved)?;
    }
    // Stand-in columns for fields the export lacks, and absent optional fields.
    let exported = saved["report"]["tables"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| {
            let field = |name: &str| t.get(name).cloned().unwrap_or_default();
            Some((
                t["table"].as_str()?.to_owned(),
                pm_ingest::pack::Exported {
                    fallbacks: serde_json::from_value(field("fallbacks_used")).unwrap_or_default(),
                    absent: [field("absent_fields"), field("missing_fields")]
                        .into_iter()
                        .flat_map(|v| serde_json::from_value::<Vec<String>>(v).unwrap_or_default())
                        .collect(),
                },
            ))
        })
        .collect::<BTreeMap<_, _>>();
    let built = build_recipe_with(pack, &hashes, &answers, &exported).map_err(|e| e.to_string())?;
    let open = saved["report"]["questions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|q| q["id"].as_str())
        .filter(|q| !answers.contains_key(*q) && *q != SET_ASIDE)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let mut bytes = serde_json::to_vec_pretty(&built.recipe).map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    folder
        .write_artifact("preparation", &artifact(pack, "recipe"), &bytes)
        .map_err(|e| e.to_string())?;
    pm_project::audit::append(
        &folder.root,
        "pack_build",
        json!({"pack": pack.id, "pack_status": pack.status, "changes": built.changes,
            "left_out": built.left_out, "unanswered": open}),
    )?;
    let account = json!({"id": pack.id, "variant": variant, "status": pack.status, "changes": built.changes,
        "left_out": built.left_out, "unanswered_questions": open,
        "note": "Unanswered questions about the company's own fields leave the standard recipe unchanged."});
    super::prepare_ocel::run(folder, bytes, Some(account))
}

#[cfg(test)]
mod tests {
    use super::{exactly_named, named_after};

    #[test]
    fn an_exact_name_beats_a_longer_one() {
        assert!(exactly_named("38 Purchase Header.csv", "Purchase Header"));
        assert!(exactly_named("EKKO_2026.csv", "EKKO"));
        assert!(!exactly_named(
            "5109 Purchase Header Archive.csv",
            "Purchase Header"
        ));
        assert!(!exactly_named("Purchase Line.csv", "Purchase Header"));
    }

    #[test]
    fn files_are_matched_to_tables_by_name() {
        assert!(named_after("EKKO.csv", "EKKO"));
        assert!(named_after("sap_ekko_2026.csv", "EKKO"));
        assert!(!named_after("EKKO.csv", "EKPO"));
        // Names with underscores, as Salesforce managed packages use.
        assert!(named_after("SBQQ__Quote__c.csv", "SBQQ__Quote__c"));
        assert!(named_after(
            "SBQQ__Quote__History.csv",
            "SBQQ__Quote__History"
        ));
        assert!(!named_after("SBQQ__Quote__History.csv", "SBQQ__Quote__c"));
        assert!(!named_after("SBQQ__QuoteLine__c.csv", "SBQQ__Quote__c"));
        // A whole word only: Order is not OrderItem.
        assert!(!named_after("OrderItem.csv", "Order"));
        assert!(named_after("Order.csv", "Order"));
    }
}
