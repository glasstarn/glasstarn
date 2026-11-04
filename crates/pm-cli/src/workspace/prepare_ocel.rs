//! Lifecycle wrapper; all data construction is in pm-ingest.
use super::{folder_info, open_project, print_json, root_option, save_latest};
use pm_ingest::prepare_ocel::Recipe;
use serde_json::json;
use std::{collections::BTreeMap, fs, path::PathBuf, process::ExitCode};

pub(super) fn command(args: &[String]) -> Result<ExitCode, String> {
    let (root, args) = root_option(args)?;
    let mut project = None;
    let mut recipe_path = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--recipe" => {
                i += 1;
                let path = args.get(i).ok_or("--recipe needs a JSON file")?;
                if recipe_path.replace(PathBuf::from(path)).is_some() {
                    return Err("duplicate --recipe".into());
                }
            }
            "--json" => {}
            flag if flag.starts_with('-') => return Err("unknown prepare-ocel option".into()),
            value => {
                if project.replace(value.to_owned()).is_some() {
                    return Err("prepare-ocel accepts one project".into());
                }
            }
        }
        i += 1;
    }
    let folder = open_project(
        &root,
        project.as_deref().ok_or("prepare-ocel needs a project")?,
    )?;
    let path = recipe_path.ok_or("prepare-ocel needs --recipe")?;
    use std::io::Read;
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|_| "cannot open recipe")?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "cannot read recipe")?;
    if bytes.len() > 1024 * 1024 {
        return Err("connected recipe exceeds 1 MiB".into());
    }
    run(folder, bytes, None)
}

/// Builds, checks and publishes a connected log from recipe bytes. `pack`
/// adds the process pack's account of the run to the printed result.
pub(super) fn run(
    mut folder: pm_project::workspace::WorkspaceFolder,
    bytes: Vec<u8>,
    pack: Option<serde_json::Value>,
) -> Result<ExitCode, String> {
    let recipe: Recipe =
        serde_json::from_slice(&bytes).map_err(|_| "invalid connected preparation recipe")?;
    let mut state = folder.state().map_err(|e| e.to_string())?;
    let mut inputs = BTreeMap::new();
    for reference in &recipe.inputs {
        if inputs.contains_key(&reference.id) {
            return Err("duplicate recipe input id".into());
        }
        let assets = state
            .preparation_sources
            .iter()
            .filter(|a| a.sha256 == reference.sha256)
            .collect::<Vec<_>>();
        let [asset] = assets.as_slice() else {
            return Err("input must identify exactly one inventoried source".into());
        };
        inputs.insert(
            reference.id.clone(),
            folder.resolve_asset(asset).map_err(|e| e.to_string())?,
        );
    }
    let outcome =
        pm_ingest::prepare_ocel::prepare_auto(&recipe, &inputs).map_err(|e| e.to_string())?;
    let stage = tempfile::Builder::new()
        .prefix(".ocel-preparation-")
        .tempdir_in(&folder.root)
        .map_err(|_| "cannot create preparation staging directory")?;
    fs::write(stage.path().join("recipe.json"), bytes).map_err(|_| "cannot save recipe")?;
    fs::write(
        stage.path().join("report.json"),
        serde_json::to_vec_pretty(&outcome.report).map_err(|e| e.to_string())?,
    )
    .map_err(|_| "cannot save report")?;
    // Lineage is one record per link; compressed, it is a small fraction of
    // its JSON size and still plain JSON once unpacked.
    write_json_gz(
        &stage.path().join("relationship-lineage.json.gz"),
        &outcome.relationship_lineage,
    )?;
    // Every row set aside, with its rule, source row and reason.
    if !outcome.set_aside.is_empty() {
        fs::write(
            stage.path().join("set-aside.json"),
            serde_json::to_vec_pretty(&outcome.set_aside).map_err(|e| e.to_string())?,
        )
        .map_err(|_| "cannot save the records set aside")?;
    }
    let set_aside = (
        outcome.report.set_aside_count,
        outcome.report.set_aside_by_reason.clone(),
    );
    drop(outcome.relationship_lineage);
    let completed = if let Some(log) = outcome.log {
        // The OCEL 2.0 SQLite form is far smaller than JSON and is read back
        // row by row, so analysis never holds the whole file in memory.
        let path = stage.path().join("prepared.sqlite");
        log.write_sqlite(&path).map_err(|e| e.to_string())?;
        // Release the construction graph before native ingestion builds its compact log.
        drop(log);
        // Run the existing strict OCEL ingestion, quality gate and Rust analysis.
        let analysis = pm_query::analyze_ocel(&path, &pm_core::OcelFilter::default())
            .map_err(|e| e.to_string())?;
        let asset = folder
            .store_asset(&path, "prepared-connected.sqlite", ".sqlite", "sources")
            .map_err(|e| e.to_string())?;
        fs::remove_file(&path).map_err(|_| "cannot remove staged log")?;
        Some((asset, analysis))
    } else {
        None
    };
    let preparation = folder.root.join("preparation");
    if preparation.canonicalize().map_err(|e| e.to_string())? != preparation {
        return Err("preparation directory cannot be a symbolic link".into());
    }
    let name = stage
        .path()
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or("invalid staging name")?
        .trim_start_matches('.');
    let run = preparation.join(name);
    fs::rename(stage.path(), &run).map_err(|_| "cannot publish preparation evidence")?;
    if set_aside.0 > 0 {
        pm_project::audit::append(
            &folder.root,
            "preparation_set_aside",
            json!({"rows": set_aside.0, "by_reason": set_aside.1, "evidence": name, "published": completed.is_some()}),
        )?;
    }
    let Some((asset, analysis)) = completed else {
        let next = if recipe.set_aside || pack.is_none() {
            "Resolve the reported keys, relationships or history with the user. The active analysis was not replaced."
        } else {
            "Some records do not fit (see preparation.issues). Ask the person, in business words with the counts and examples: leave them out, build with the rest and list them as a data-quality finding, or ask for a corrected export? Save the answer with workspace pack answer --question data:set-aside --choice set_aside|new_export, then build again. The active analysis was not replaced."
        };
        let mut value = json!({"status":"blocked","project":folder_info(&folder),"preparation":outcome.report,"evidence":run,"next":next});
        if let Some(pack) = pack {
            value["pack"] = pack;
        }
        print_json(&value)?;
        return Ok(ExitCode::from(3));
    };
    state.source = Some(asset.clone());
    state.case_project = None;
    state.case_model = None;
    state.ocel_filter = Some(Default::default());
    state.ocel_model = None;
    folder.save_state(&state).map_err(|e| e.to_string())?;
    let mut value = json!({"status":"complete","kind":"ocel","project":folder_info(&folder),"preparation":outcome.report,"evidence":run,"saved_source":asset.relative_path,"analysis":analysis});
    if set_aside.0 > 0 {
        value["next"] = json!(format!(
            "{} records that did not fit were left out, as the person approved (preparation.set_aside_by_reason; every row is in set-aside.json in the evidence folder). Report them as a data-quality finding with counts, in business words, alongside the analysis.",
            set_aside.0
        ));
    }
    if let Some(pack) = pack {
        value["pack"] = pack;
    }
    save_latest(&folder, &value)?;
    print_json(&value)?;
    Ok(ExitCode::SUCCESS)
}

fn write_json_gz(path: &std::path::Path, value: &impl serde::Serialize) -> Result<(), String> {
    use std::io::Write;
    let file = fs::File::create(path).map_err(|_| "cannot create preparation artifact")?;
    // Buffer before the compressor: it is slow on serde's many tiny writes.
    let mut writer = std::io::BufWriter::with_capacity(
        1 << 20,
        flate2::write::GzEncoder::new(file, flate2::Compression::default()),
    );
    serde_json::to_writer(&mut writer, value).map_err(|_| "cannot write preparation artifact")?;
    writer
        .into_inner()
        .map_err(|_| "cannot finish preparation artifact".to_owned())?
        .finish()
        .and_then(|mut file| file.flush())
        .map_err(|_| "cannot finish preparation artifact".to_owned())
}
