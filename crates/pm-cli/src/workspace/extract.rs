//! `glasstarn extract`: file a reviewed extract for a person to approve in
//! the app, or report the state of filed requests.
//!
//! The command line never approves or downloads. Approval is a person's click
//! in the browser, and the app then runs exactly the approved spec, so an
//! agent driving this command cannot start an extract on its own.
use super::{print_json, root_option};
use pm_project::extract;
use pm_project::workspace::{WorkspaceFolder, valid_workspace_id};
use serde_json::json;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "usage: glasstarn extract salesforce|jira|servicenow|hubspot <project-id|folder> --spec spec.json | glasstarn extract status <project-id|folder> | glasstarn extract discover <project-id|folder> --org <alias> --object <Name> [--object ...] [--pack <pack>] [--api-version 66] | glasstarn extract discover <project-id|folder> --system jira --site https://<site> --object <PROJECT> | glasstarn extract discover <project-id|folder> --system servicenow --site https://<instance> --object <table>";

pub(crate) fn command(args: &[String]) -> Result<ExitCode, String> {
    let (root, args) = root_option(args)?;
    let mut spec_path = None;
    let mut org = None;
    let mut objects = Vec::new();
    let mut pack = None;
    let mut api_version = 66u32;
    let mut system = "salesforce".to_owned();
    let mut site = None;
    let mut positionals = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--json" => {}
            "--spec" => {
                i += 1;
                let value = args.get(i).ok_or("--spec needs a value")?;
                if spec_path.replace(value.clone()).is_some() {
                    return Err("duplicate --spec".into());
                }
            }
            "--org" | "--object" | "--pack" | "--api-version" | "--system" | "--site" => {
                let flag = args[i].clone();
                i += 1;
                let value = args.get(i).ok_or(format!("{flag} needs a value"))?.clone();
                match flag.as_str() {
                    "--org" => org = Some(value),
                    "--object" => objects.push(value),
                    "--pack" => pack = Some(value),
                    "--system" => system = value,
                    "--site" => site = Some(value),
                    _ => {
                        api_version = value
                            .parse()
                            .map_err(|_| "--api-version must be a number")?
                    }
                }
            }
            "--approve" => {
                return Err("approval is given in the app: open the project, review the extract and choose Approve and run. The command line cannot approve or run an extract.".into());
            }
            value if value.starts_with('-') => return Err("unknown extract option".into()),
            _ => positionals.push(args[i].clone()),
        }
        i += 1;
    }
    let [action, project] = positionals.as_slice() else {
        return Err(USAGE.into());
    };
    // Reads and writes request files only, so this works while the project
    // is open in the app, which holds the project's write lock.
    let folder = project_path(&root, project)?;
    let manifest = WorkspaceFolder::inspect(&folder).map_err(|error| error.to_string())?;
    let info = json!({"id":manifest.id,"name":manifest.name,"path":folder});
    match action.as_str() {
        "status" => {
            if spec_path.is_some() {
                return Err(USAGE.into());
            }
            let requests = extract::requests(&folder)?
                .into_iter()
                .map(|mut request| {
                    // The queries were reviewed when filed; status stays short.
                    if let Some(request) = request.as_object_mut() {
                        request.remove("preview");
                    }
                    request
                })
                .collect::<Vec<_>>();
            print_json(&json!({"project":info,"requests":requests}))?;
            Ok(ExitCode::SUCCESS)
        }
        "discover" if system == "servicenow" => {
            let site = site.ok_or("ServiceNow discovery needs --site https://<instance>")?;
            let user = pm_project::extract_servicenow::user_from_environment()?;
            let mut found =
                pm_project::discover::servicenow(Path::new("curl"), &user, &site, &objects)?;
            found["project"] = info;
            let dir = folder.join("extraction");
            std::fs::create_dir_all(&dir).map_err(|_| "cannot save the discovery")?;
            let mut bytes = serde_json::to_vec_pretty(&found).map_err(|e| e.to_string())?;
            bytes.push(b'\n');
            std::fs::write(dir.join("discovery-servicenow.json"), bytes)
                .map_err(|_| "cannot save the discovery")?;
            found["next"] = json!(
                "Tell the person each table's states and name the company's own fields and every gap in business words; then write the ServiceNow extract spec (tables, fields, label fields, the audited fields to keep) for their approval."
            );
            print_json(&found)?;
            Ok(ExitCode::SUCCESS)
        }
        "discover" if system == "jira" => {
            let site = site.ok_or("Jira discovery needs --site https://<site>")?;
            let user = pm_project::extract_jira::user_from_environment()?;
            let mut found = pm_project::discover::jira(Path::new("curl"), &user, &site, &objects)?;
            found["project"] = info;
            let dir = folder.join("extraction");
            std::fs::create_dir_all(&dir).map_err(|_| "cannot save the discovery")?;
            let mut bytes = serde_json::to_vec_pretty(&found).map_err(|e| e.to_string())?;
            bytes.push(b'\n');
            std::fs::write(dir.join("discovery-jira.json"), bytes)
                .map_err(|_| "cannot save the discovery")?;
            found["next"] = json!(
                "Tell the person which issue types and statuses each project has and name the company's own fields; then write the Jira extract spec (fields by ID, the changed fields to keep) for their approval."
            );
            print_json(&found)?;
            Ok(ExitCode::SUCCESS)
        }
        "discover" => {
            let org = org.ok_or("extract discover needs --org <alias>")?;
            let needs = match &pack {
                Some(name) => needs(name, &objects)?,
                None => Vec::new(),
            };
            let mut found = pm_project::discover::salesforce(
                &pm_project::extract::salesforce_cli(),
                &org,
                api_version,
                &objects,
                &needs,
            )?;
            found["project"] = info;
            found["pack"] = json!(pack);
            let dir = folder.join("extraction");
            std::fs::create_dir_all(&dir).map_err(|_| "cannot save the discovery")?;
            let mut bytes = serde_json::to_vec_pretty(&found).map_err(|e| e.to_string())?;
            bytes.push(b'\n');
            std::fs::write(dir.join("discovery-salesforce.json"), bytes)
                .map_err(|_| "cannot save the discovery")?;
            found["next"] = json!(
                "Tell the person what the org holds and every gap in business words; field history that is off cannot be recovered, only switched on from now. Then write the extract spec for the objects the pack needs."
            );
            print_json(&found)?;
            Ok(ExitCode::SUCCESS)
        }
        system @ ("salesforce" | "jira" | "servicenow" | "hubspot") => {
            let spec_path = spec_path.ok_or("--spec is required")?;
            let mut bytes = Vec::new();
            File::open(spec_path)
                .map_err(|_| "cannot read extract spec")?
                .take(64 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| "cannot read extract spec")?;
            let reviewed = extract::review(&bytes)?;
            if reviewed.preview()["system"] != system {
                return Err(format!("this spec is not a {system} extract"));
            }
            let request = extract::request(&folder, &reviewed)?;
            print_json(&json!({"status":"awaiting_approval","project":info,
                "spec_sha256":reviewed.sha256,"request_status":request["status"],
                "preview":reviewed.preview(),
                "next":"Explain this extract to the person, then ask them to open this project in the app, review it and choose Approve and run. Nothing is downloaded before that. Check progress with glasstarn extract status <project>."}))?;
            Ok(ExitCode::SUCCESS)
        }
        _ => Err(USAGE.into()),
    }
}

pub(super) fn project_path(root: &Path, value: &str) -> Result<PathBuf, String> {
    let given = PathBuf::from(value);
    if given.join("workspace.json").is_file() {
        Ok(given)
    } else if valid_workspace_id(value) {
        Ok(root.join(value))
    } else {
        Err("project must be a folder path or 32-character project id".into())
    }
}

/// What a pack needs from each object: tracked fields from its history
/// table and the fields it reads from the object itself.
fn needs(pack: &str, objects: &[String]) -> Result<Vec<pm_project::discover::HistoryNeed>, String> {
    let text = pm_ingest::pack::BUILT_IN
        .iter()
        .find(|(id, _)| *id == pack)
        .map(|(_, text)| (*text).to_owned())
        .or_else(|| std::fs::read_to_string(pack).ok())
        .ok_or(format!("unknown pack '{pack}'"))?;
    let pack = pm_ingest::pack::Pack::parse(&text).map_err(|e| e.to_string())?;
    Ok(objects
        .iter()
        .map(|object| {
            let history = pack
                .tables
                .iter()
                .find(|t| t.history_of.as_deref() == Some(object.as_str()));
            pm_project::discover::HistoryNeed {
                object: object.clone(),
                history_table: history.map(|t| t.name.clone()).unwrap_or_default(),
                fields: history.map(|t| t.needs_tracked.clone()).unwrap_or_default(),
                required: pack
                    .tables
                    .iter()
                    .find(|t| &t.name == object)
                    .map(|t| t.required.clone())
                    .unwrap_or_default(),
            }
        })
        .collect())
}
