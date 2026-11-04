//! One local service session per project folder. Browser navigation cannot
//! retarget a running agent; a project stays bound to its own port and identity.
use super::*;
use pm_project::workspace::{
    InvestigationPurpose, WorkspaceAsset, WorkspaceFolder, WorkspaceState, valid_workspace_id,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::Path;

/// A running project page and the switch that stops it.
type ProjectPage = (Arc<AtomicBool>, std::thread::JoinHandle<()>);

pub(super) struct WorkspaceHub {
    root: PathBuf,
    default_policy: Option<pm_mcp::ExposurePolicy>,
    sessions: Mutex<BTreeMap<String, Value>>,
    /// Each open project's page, with the switch that stops it, so a project
    /// can be renamed or deleted after it was opened in this browser.
    pages: Mutex<BTreeMap<String, ProjectPage>>,
    pub(super) shutdown: Arc<AtomicBool>,
    /// The project list's own address, where a page whose project is
    /// deleted sends the person.
    home: String,
}

impl WorkspaceHub {
    /// Stops every project page this hub started.
    pub(super) fn stop_pages(&self) {
        if let Ok(pages) = self.pages.lock() {
            for (stop, _) in pages.values() {
                stop.store(true, Ordering::Release);
            }
        }
    }
}

fn issue(message: impl ToString) -> String {
    message.to_string()
}

impl Server {
    pub fn bind_with_workspaces(
        port: u16,
        policy: Option<pm_mcp::ExposurePolicy>,
        root: &Path,
    ) -> Result<Self, String> {
        fs::create_dir_all(root).map_err(issue)?;
        let root = root.canonicalize().map_err(issue)?;
        let mut server = Self::bind_with_agent(port, policy.clone()).map_err(issue)?;
        server.workspace_hub_owner = true;
        let home = format!("http://{}/", server.address);
        server.workspace_hub = Some(Arc::new(WorkspaceHub {
            root,
            default_policy: policy,
            sessions: Mutex::new(BTreeMap::new()),
            pages: Mutex::new(BTreeMap::new()),
            shutdown: Arc::new(AtomicBool::new(false)),
            home,
        }));
        Ok(server)
    }

    pub(super) fn workspace_request(
        &self,
        stream: &mut TcpStream,
        request: Request,
    ) -> Result<(), ServerError> {
        let result = self.workspace_operation(stream, request);
        match result {
            Ok(value) => write_json(stream, 200, "OK", &value),
            Err(message) => write_json(stream, 409, "Conflict", &ErrorResponse::new(&message)),
        }
    }

    fn workspace_operation(
        &self,
        stream: &mut TcpStream,
        request: Request,
    ) -> Result<Value, String> {
        let hub = self
            .workspace_hub
            .as_ref()
            .ok_or("project folders are not enabled for this service")?;
        match (request.method.as_str(), request.target.as_str()) {
            ("GET", "/api/workspaces") => {
                let mut projects = Vec::new();
                let mut seen = std::collections::BTreeSet::new();
                // Every folder that holds projects, the main one first.
                for root in pm_project::folders::all_roots(&hub.root) {
                    let Ok(entries) = fs::read_dir(&root) else {
                        continue;
                    };
                    for entry in entries {
                        let entry = entry.map_err(issue)?;
                        let id = entry.file_name().to_string_lossy().into_owned();
                        if !valid_workspace_id(&id) || !entry.file_type().map_err(issue)?.is_dir() {
                            continue;
                        }
                        if let Ok(manifest) = WorkspaceFolder::inspect(&entry.path())
                            && manifest.id == id
                            && seen.insert(id.clone())
                        {
                            projects.push(
                                json!({"id":id,"name":manifest.name,"purpose":manifest.purpose}),
                            );
                        }
                    }
                }
                projects.sort_by_key(|v| v["name"].as_str().unwrap_or_default().to_lowercase());
                let current = self.workspace_folder.lock().map_err(issue)?;
                Ok(json!({"projects":projects,"current":current.as_ref().map(workspace_info)}))
            }
            ("POST", "/api/workspaces/create") | ("POST", "/api/workspaces/open") => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Command {
                    id: Option<String>,
                    name: Option<String>,
                    purpose: Option<InvestigationPurpose>,
                }
                let create = request.target.ends_with("/create");
                let command: Command = serde_json::from_slice(
                    &request
                        .json_body(stream)
                        .map_err(|(_, _, message)| message.to_owned())?,
                )
                .map_err(issue)?;
                let mut sessions = hub.sessions.lock().map_err(issue)?;
                if sessions.len() >= 64
                    && (create
                        || !command
                            .id
                            .as_ref()
                            .is_some_and(|id| sessions.contains_key(id)))
                {
                    return Err(
                        "64 projects are already open; restart the application to release sessions"
                            .into(),
                    );
                }
                let (id, folder) = if create {
                    let name = command.name.ok_or("a project name is required")?;
                    if name.trim().is_empty() || name.len() > 200 {
                        return Err("use a project name of 1–200 bytes".into());
                    }
                    let mut entropy = [0u8; 16];
                    OsRng.fill_bytes(&mut entropy);
                    let id = hex(&entropy);
                    let folder = WorkspaceFolder::create(
                        &hub.root.join(&id),
                        &id,
                        &name,
                        command.purpose.unwrap_or_default(),
                    )
                    .map_err(issue)?;
                    (id, folder)
                } else {
                    let id = command.id.ok_or("a project identifier is required")?;
                    if !valid_workspace_id(&id) {
                        return Err("invalid project identifier".into());
                    }
                    if let Some(session) = sessions.get(&id) {
                        return Ok(session.clone());
                    }
                    // The project's folder, in the main home or a remembered one.
                    let place = pm_project::folders::all_roots(&hub.root)
                        .into_iter()
                        .map(|root| root.join(&id))
                        .find(|path| path.join("workspace.json").is_file())
                        .unwrap_or_else(|| hub.root.join(&id));
                    let folder = match WorkspaceFolder::open(&place) {
                        Ok(folder) => folder,
                        // Another glasstarn already has it open (an earlier
                        // link, or the page an agent opened): go to that page
                        // rather than refuse.
                        Err(error) => {
                            if let Some(url) = running_session(&place) {
                                return Ok(json!({"url": url, "already_open": true}));
                            }
                            return Err(issue(error));
                        }
                    };
                    if folder.manifest.id != id {
                        return Err("project identity does not match its folder".into());
                    }
                    (id, folder)
                };
                let mut server =
                    Self::bind_with_agent(0, hub.default_policy.clone()).map_err(issue)?;
                server.workspace_hub = Some(Arc::clone(hub));
                server.restore_folder(&folder)?;
                server.restore_agent_access(&folder)?;
                let response = json!({"project":workspace_info(&folder),"url":server.launch_url(),"port":server.address.port()});
                // Lets a second glasstarn find this page instead of failing on
                // the project's lock.
                let _ = std::fs::write(
                    folder.root.join(".browser-session.json"),
                    json!({"url": format!("http://{}/", server.address), "pid": std::process::id(), "build": crate::build_id()}).to_string(),
                );
                *server.workspace_folder.lock().map_err(issue)? = Some(folder);
                let stop = Arc::new(AtomicBool::new(false));
                let page_stop = Arc::clone(&stop);
                let handle = std::thread::Builder::new()
                    .name(format!("project-{id}"))
                    .spawn(move || {
                        let _ = server.run(page_stop);
                    })
                    .map_err(issue)?;
                hub.pages
                    .lock()
                    .map_err(issue)?
                    .insert(id.clone(), (stop, handle));
                sessions.insert(id, response.clone());
                Ok(response)
            }
            ("POST", "/api/workspaces/rename") | ("POST", "/api/workspaces/delete") => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Command {
                    id: String,
                    name: Option<String>,
                }
                let command: Command = serde_json::from_slice(
                    &request
                        .json_body(stream)
                        .map_err(|(_, _, message)| message.to_owned())?,
                )
                .map_err(issue)?;
                if !valid_workspace_id(&command.id) {
                    return Err("invalid project identifier".into());
                }
                let mut sessions = hub.sessions.lock().map_err(issue)?;
                // Asked from the project's own page (its "All projects" list):
                // this page is the one to close. It lets go of the project now,
                // answers, then stops; the person is sent to the project list.
                let own_page =
                    std::thread::current().name() == Some(&format!("project-{}", command.id));
                let mut closing_own_page = false;
                // Opened earlier from this page: stop its page, which frees it.
                let running = hub.pages.lock().map_err(issue)?.remove(&command.id);
                if let Some((stop, handle)) = running {
                    stop.store(true, Ordering::Release);
                    if own_page && !request.target.ends_with("/rename") {
                        *self.workspace_folder.lock().map_err(issue)? = None;
                        closing_own_page = true;
                        drop(handle);
                    } else if own_page {
                        // Renaming keeps the page: put it back, still running,
                        // and rename through the project it already holds.
                        stop.store(false, Ordering::Release);
                        hub.pages
                            .lock()
                            .map_err(issue)?
                            .insert(command.id.clone(), (stop, handle));
                        let name = command.name.clone().unwrap_or_default();
                        let name = name.trim();
                        if name.is_empty() || name.len() > 200 {
                            return Err("use a project name of 1–200 bytes".into());
                        }
                        let mut held = self.workspace_folder.lock().map_err(issue)?;
                        let folder = held.as_mut().ok_or("project not found")?;
                        folder.rename(name).map_err(issue)?;
                        return Ok(json!({"project":workspace_info(folder)}));
                    } else {
                        let _ = handle.join();
                    }
                    if closing_own_page || !own_page {
                        sessions.remove(&command.id);
                    }
                }
                let place = pm_project::folders::all_roots(&hub.root)
                    .into_iter()
                    .map(|root| root.join(&command.id))
                    .find(|path| path.join("workspace.json").is_file())
                    .ok_or("project not found")?;
                // Opening it takes its lock: a project open elsewhere is left alone.
                let mut folder = WorkspaceFolder::open(&place).map_err(|_| {
                    "This project is open in another glasstarn window. Close it there first."
                        .to_owned()
                })?;
                if request.target.ends_with("/rename") {
                    let name = command.name.unwrap_or_default();
                    let name = name.trim();
                    if name.is_empty() || name.len() > 200 {
                        return Err("use a project name of 1–200 bytes".into());
                    }
                    folder.rename(name).map_err(issue)?;
                    return Ok(json!({"project":workspace_info(&folder)}));
                }
                let label = folder.manifest.name.clone();
                drop(folder);
                sessions.remove(&command.id);
                let trashed = pm_project::folders::move_to_trash(&place, &label).map_err(issue)?;
                Ok(
                    json!({"deleted":command.id,"moved_to":trashed,"list_url":closing_own_page.then(|| hub.home.clone())}),
                )
            }
            ("POST", "/api/workspace/save") => self.save_workspace(),
            ("GET", "/api/workspace/views") => {
                let folder = self.workspace_folder.lock().map_err(issue)?;
                let folder = folder.as_ref().ok_or("open a project first")?;
                Ok(json!({"views":folder.saved_views().map_err(issue)?}))
            }
            ("POST", "/api/workspace/views") => {
                #[derive(serde::Deserialize)]
                #[serde(deny_unknown_fields)]
                struct SavedView {
                    title: String,
                    exploration: Value,
                    filter: pm_core::Filter,
                    object_type: Option<String>,
                }
                let command: SavedView = serde_json::from_slice(
                    &request
                        .json_body(stream)
                        .map_err(|(_, _, message)| message.to_owned())?,
                )
                .map_err(issue)?;
                if command.title.trim().is_empty()
                    || command.title.len() > 640
                    || command.exploration.to_string().len() > 16 * 1024
                {
                    return Err("invalid saved view".into());
                }
                let exploration = if command.exploration.get("linked").is_some() {
                    #[derive(serde::Deserialize, serde::Serialize)]
                    #[serde(deny_unknown_fields)]
                    struct Linked {
                        linked: pm_query::linked_interval::LinkedInterval,
                    }
                    let query: Linked =
                        serde_json::from_value(command.exploration).map_err(issue)?;
                    if Some(&query.linked.root_type) != command.object_type.as_ref() {
                        return Err("linked root must match the followed object".into());
                    }
                    serde_json::to_value(query).map_err(issue)?
                } else {
                    serde_json::to_value(
                        serde_json::from_value::<pm_query::exploration::ExplorationRequest>(
                            command.exploration,
                        )
                        .map_err(issue)?,
                    )
                    .map_err(issue)?
                };
                self.save_workspace()?;
                let folder = self.workspace_folder.lock().map_err(issue)?;
                let folder = folder.as_ref().ok_or("open a project first")?;
                let state = folder.state().map_err(issue)?;
                let filter = state
                    .followed_view
                    .as_ref()
                    .map(|view| &view.filter)
                    .or_else(|| {
                        state
                            .case_project
                            .as_ref()
                            .map(|project| &project.view_filter)
                    });
                if state.source.is_none()
                    || filter != Some(&command.filter)
                    || state.followed_view.as_ref().map(|view| &view.object_type)
                        != command.object_type.as_ref()
                {
                    return Err("the project selection changed; refresh before saving".into());
                }
                let stamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(issue)?
                    .as_nanos();
                let record = json!({"version":1,"project":folder.manifest.id,"title":command.title,
                    "exploration":exploration,"filter":command.filter,"object_type":command.object_type,"state":state});
                let bytes = serde_json::to_vec(&record).map_err(issue)?;
                if bytes.len() > 128 * 1024 {
                    return Err("saved view exceeds the supported size".into());
                }
                folder
                    .write_artifact("exports", &format!("saved-view-{stamp}.json"), &bytes)
                    .map_err(issue)?;
                Ok(json!({"saved":true}))
            }
            ("POST", "/api/workspace/views/remove") => {
                #[derive(serde::Deserialize)]
                #[serde(deny_unknown_fields)]
                struct RemoveView {
                    id: String,
                }
                let command: RemoveView = serde_json::from_slice(
                    &request
                        .json_body(stream)
                        .map_err(|(_, _, message)| message.to_owned())?,
                )
                .map_err(issue)?;
                let folder = self.workspace_folder.lock().map_err(issue)?;
                let folder = folder.as_ref().ok_or("open a project first")?;
                folder.remove_saved_view(&command.id).map_err(issue)?;
                Ok(json!({"removed":true}))
            }
            ("GET", "/api/workspace/state") => self.workspace_state(),
            ("GET", "/api/workspace/provenance") => {
                let folder = self.workspace_folder.lock().map_err(issue)?;
                Ok(folder.as_ref().map_or(json!({}), |folder| {
                    super::provenance::provenance(&folder.root)
                }))
            }
            ("GET", "/api/workspace/investigation") => {
                let folder = self.workspace_folder.lock().map_err(issue)?;
                match folder.as_ref() {
                    Some(folder) => {
                        let mut report = investigation_report(folder)?;
                        // Which expected process is current, so every open page
                        // can pick up one another page or the agent set.
                        if let Some(model) = folder.state().ok().and_then(|state| state.case_model)
                        {
                            report["case_model_sha256"] = json!(model.sha256);
                        }
                        // A model the agent checked while this page was open:
                        // every open page is offered it until one has adopted
                        // it as the project's baseline; then it is cleared.
                        let pending = folder.root.join("pending-model.model.json");
                        if let Ok(text) = fs::read_to_string(&pending)
                            && text.len() <= 10 * 1024 * 1024
                        {
                            use sha2::Digest;
                            let sha = hex(&sha2::Sha256::digest(text.as_bytes()));
                            let adopted = folder
                                .state()
                                .ok()
                                .and_then(|state| state.case_model)
                                .is_some_and(|model| model.sha256 == sha);
                            if adopted {
                                let _ = fs::remove_file(&pending);
                            } else {
                                report["pending_model"] = json!({"name": "expected-process.model.json", "sha256": sha, "text": text});
                            }
                        }
                        Ok(report)
                    }
                    None => Ok(json!({"record":null})),
                }
            }
            _ => Err("unknown project operation".into()),
        }
    }

    pub(super) fn save_workspace(&self) -> Result<Value, String> {
        let followed_view = self
            .follow_context
            .lock()
            .map_err(issue)?
            .as_ref()
            .map(|context| pm_project::workspace::FollowedView {
                object_type: context.object_type.clone(),
                filter: context.case_filter.clone(),
                base_filter: context.base_filter.clone(),
            });
        let mut folder = self.workspace_folder.lock().map_err(issue)?;
        let Some(folder) = folder.as_mut() else {
            return Ok(json!({"saved":false}));
        };
        let live = self.live_workspace.lock().map_err(issue)?;
        let previous = folder.state().map_err(issue)?;
        // Save the connected source, never a followed view of it.
        let upload = self.ocel_store().lock().map_err(issue)?;
        let source = save_upload(folder, upload.as_ref(), previous.source.as_ref(), "sources")?;
        let case_model = self.case_model.lock().map_err(issue)?;
        let ocel_model = self.ocel_model.lock().map_err(issue)?;
        let case_asset = save_upload(
            folder,
            case_model.as_ref().filter(|_| live.case_model.is_some()),
            previous.case_model.as_ref(),
            "models",
        )?;
        let ocel_asset = save_upload(
            folder,
            ocel_model.as_ref().filter(|_| live.ocel_model.is_some()),
            previous.ocel_model.as_ref(),
            "models",
        )?;
        let mut project = live.case_project.as_ref().map(|p| p.project.clone());
        if let (Some(project), Some(source)) = (&mut project, &source) {
            project.source.path.clone_from(&source.relative_path);
            project.name.clone_from(&folder.manifest.name);
        }
        let state = WorkspaceState {
            followed_view,
            source,
            preparation_sources: previous.preparation_sources,
            case_project: project,
            ocel_filter: live.ocel_log.as_ref().map(|log| log.filter.clone()),
            case_model: case_asset,
            ocel_model: ocel_asset,
            observation_window: previous.observation_window,
            item_names: previous.item_names,
        };
        folder.save_state(&state).map_err(issue)?;
        Ok(json!({"saved":true,"project":workspace_info(folder)}))
    }

    pub(super) fn restore_folder(&self, folder: &WorkspaceFolder) -> Result<(), String> {
        if folder.manifest.state.is_null() {
            return Ok(());
        }
        let state = folder.state().map_err(issue)?;
        if (state.case_project.is_some() && state.ocel_filter.is_some())
            || (state.source.is_none()
                && (state.case_project.is_some() || state.ocel_filter.is_some()))
        {
            return Err("saved project has inconsistent dataset state".into());
        }
        self.stop_following();
        if let Some(saved) = state.followed_view {
            *self.follow_context.lock().map_err(issue)? = Some(follow::FollowContext {
                object_type: saved.object_type,
                base_filter: saved.base_filter,
                case_filter: saved.filter,
                journey: None,
            });
        }
        let mut live = pm_mcp::LiveWorkspace::default();
        if let Some(asset) = &state.source {
            let upload = restore_upload(folder, asset)?;
            if let Some(mut project) = state.case_project {
                project.validate().map_err(issue)?;
                if !project.check_fingerprint(&upload.source()).reproduces() {
                    return Err("saved analysis and source fingerprints disagree".into());
                }
                project.source.path = upload.path.display().to_string();
                live.case_project = Some(pm_mcp::LiveProject {
                    project,
                    source_name: asset.name.clone(),
                });
            }
            if let Some(filter) = state.ocel_filter {
                live.ocel_log = Some(pm_mcp::LiveOcelLog {
                    path: upload.path.clone(),
                    source_name: asset.name.clone(),
                    filter,
                });
            }
            *self.upload.lock().map_err(issue)? = Some(upload);
        }
        if let Some(asset) = &state.case_model {
            if asset.bytes > MAX_CASE_MODEL_BYTES {
                return Err("saved case model exceeds the size limit".into());
            }
            let model = restore_upload(folder, asset)?;
            pm_query::parse_formal_model_json(&fs::read(&model.path).map_err(issue)?)
                .map_err(issue)?;
            live.case_model = Some(pm_mcp::LiveProcessModel {
                path: model.path.clone(),
                source_name: asset.name.clone(),
            });
            *self.case_model.lock().map_err(issue)? = Some(model);
        }
        if let Some(asset) = &state.ocel_model {
            if asset.bytes > MAX_OCEL_MODEL_BYTES {
                return Err("saved object model exceeds the size limit".into());
            }
            let model = restore_upload(folder, asset)?;
            let parsed: pm_query::OcelModelReport =
                serde_json::from_slice(&fs::read(&model.path).map_err(issue)?).map_err(issue)?;
            let nodes = parsed.places.len().saturating_add(parsed.transitions.len());
            if nodes == 0 || nodes > 10_000 {
                return Err("saved object model has an invalid node count".into());
            }
            live.ocel_model = Some(pm_mcp::LiveOcelModel {
                path: model.path.clone(),
                source_name: asset.name.clone(),
            });
            *self.ocel_model.lock().map_err(issue)? = Some(model);
        }
        *self.live_workspace.lock().map_err(issue)? = live;
        Ok(())
    }

    fn workspace_state(&self) -> Result<Value, String> {
        let folder = self.workspace_folder.lock().map_err(issue)?;
        let Some(folder) = folder.as_ref() else {
            return Ok(json!({"project":null}));
        };
        // Bootstrap a restored followed view from its original source scope;
        // the live agent boundary remains the saved, narrowed cohort.
        let followed_view = self
            .follow_context
            .lock()
            .map_err(issue)?
            .as_ref()
            .map(|context| pm_project::workspace::FollowedView {
                object_type: context.object_type.clone(),
                filter: context.case_filter.clone(),
                base_filter: context.base_filter.clone(),
            });
        let upload = self.ocel_store().lock().map_err(issue)?;
        let live = self.live_workspace.lock().map_err(issue)?;
        let mut result = json!({"project":workspace_info(folder),"followed_view":followed_view});
        result["item_names"] = serde_json::to_value(
            folder
                .state()
                .map(|state| state.item_names)
                .unwrap_or_default(),
        )
        .map_err(issue)?;
        if let Some(upload) = upload.as_ref() {
            if let Some(ocel) = live.ocel_log.as_ref() {
                let mut analysis = pm_query::analyze_ocel(
                    &upload.path,
                    followed_view
                        .as_ref()
                        .map_or(&ocel.filter, |view| &view.base_filter),
                )
                .map_err(issue)?;
                analysis.source.clone_from(&upload.file_name);
                result["ocel"] = json!({"upload_id":upload.id,"analysis":analysis});
                result["ocel_filter"] = serde_json::to_value(&ocel.filter).map_err(issue)?;
            } else {
                let (schema, sample) =
                    pm_ingest::sample_rows(&upload.path, pm_ingest::PROFILE_ROWS).map_err(issue)?;
                let profile = pm_ingest::Profile::build(&schema, &sample);
                result["profile"] = serde_json::to_value(ProfileResponse {
                    upload_id: upload.id.clone(),
                    file_name: upload.file_name.clone(),
                    bytes: upload.bytes,
                    rows_sampled: profile.rows_sampled,
                    columns: profile.columns.iter().map(ColumnResponse::from).collect(),
                    proposed_mapping: pm_query::propose_mapping(schema.headers()),
                })
                .map_err(issue)?;
                if let Some(project) = live.case_project.as_ref() {
                    let mut portable = project.project.clone();
                    portable.source.path.clone_from(&upload.file_name);
                    result["case_project"] = serde_json::to_value(portable).map_err(issue)?;
                }
            }
        }
        if let Some(model) = self
            .case_model
            .lock()
            .map_err(issue)?
            .as_ref()
            .filter(|_| live.case_model.is_some())
        {
            let formal = pm_query::parse_formal_model_json(&fs::read(&model.path).map_err(issue)?)
                .map_err(issue)?;
            result["case_model"] = json!({"model_id":model.id,"file_name":model.file_name,"places":formal.net.place_count,"transitions":formal.net.transitions.len()});
        }
        if let Some(model) = self
            .ocel_model
            .lock()
            .map_err(issue)?
            .as_ref()
            .filter(|_| live.ocel_model.is_some())
        {
            result["ocel_model"] = json!({"model_id":model.id,"file_name":model.file_name});
        }
        Ok(result)
    }
}

fn investigation_report(folder: &WorkspaceFolder) -> Result<Value, String> {
    let directory = folder.root.join("findings");
    if directory.canonicalize().map_err(issue)? != directory {
        return Err("findings directory cannot be a symbolic link".into());
    }
    let mut candidates = Vec::new();
    for entry in fs::read_dir(&directory).map_err(issue)? {
        let entry = entry.map_err(issue)?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let valid = name
            .strip_prefix("investigation-")
            .and_then(|v| v.strip_suffix(".json"))
            .is_some_and(|id| !id.is_empty() && id.bytes().all(|c| c.is_ascii_digit()));
        if valid && entry.file_type().map_err(issue)?.is_file() {
            candidates.push(name);
        }
    }
    candidates.sort();
    let Some(name) = candidates.last() else {
        return Ok(json!({"record":null}));
    };
    let path = directory.join(name);
    let meta = fs::symlink_metadata(&path).map_err(issue)?;
    if !meta.is_file() || meta.len() > 64 * 1024 * 1024 {
        return Err("investigation must be a regular file no larger than 64 MiB".into());
    }
    let record: Value = serde_json::from_slice(&fs::read(&path).map_err(issue)?).map_err(issue)?;
    if record["project"]["id"] != folder.manifest.id || record["version"] != 1 {
        return Err("investigation belongs to another project or unsupported version".into());
    }
    let state = data_scope(&serde_json::to_value(folder.state().map_err(issue)?).map_err(issue)?);
    // An answer goes stale only when the data behind it changes (a new source,
    // mapping or rules), not when someone browses with a filter, follows
    // another item type or adds an expected process.
    let stale = data_scope(&record["scope"]) != state;
    // Earlier questions, newest first: what was asked and answered, so one
    // project holds the whole conversation. Settings changes mark them as
    // belonging to earlier settings rather than hiding them.
    // A stale latest answer joins the earlier ones rather than disappearing.
    let history = candidates
        .iter()
        .rev()
        .skip(usize::from(!stale))
        .take(20)
        .filter_map(|earlier| {
            let path = directory.join(earlier);
            let meta = fs::symlink_metadata(&path).ok()?;
            if !meta.is_file() || meta.len() > 64 * 1024 * 1024 {
                return None;
            }
            let record: Value = serde_json::from_slice(&fs::read(&path).ok()?).ok()?;
            if record["project"]["id"] != folder.manifest.id || record["version"] != 1 {
                return None;
            }
            let note = &record["note"];
            Some(json!({
                "question": note["question"],
                "answer": note["answer"],
                "process": note["process"],
                "findings": note["findings"].as_array().map(|findings| findings.iter()
                    .map(|finding| json!({"kind":finding["kind"],"title":finding["title"],"statement":finding["statement"]}))
                    .collect::<Vec<_>>()).unwrap_or_default(),
                "recorded_at": record["recorded_at"],
                "current": data_scope(&record["scope"]) == state,
                "artifact": format!("findings/{earlier}")
            }))
        })
        .collect::<Vec<_>>();
    // Old interpretations remain historical; do not present them as current findings.
    Ok(
        json!({"record":if stale {Value::Null} else {record},"stale":stale,"artifact":format!("findings/{name}"),"history":history}),
    )
}

/// The parts of a project's settings that decide what the data is: without
/// view filters, the followed item type, expected-process models and names.
fn data_scope(state: &Value) -> Value {
    let mut scope = state.clone();
    if let Some(map) = scope.as_object_mut() {
        for key in [
            "case_model",
            "ocel_model",
            "followed_view",
            "item_names",
            "ocel_filter",
        ] {
            map.remove(key);
        }
        if let Some(project) = map.get_mut("case_project").and_then(Value::as_object_mut) {
            project.remove("view_filter");
        }
    }
    scope
}

fn workspace_info(folder: &WorkspaceFolder) -> Value {
    json!({"id":folder.manifest.id,"name":folder.manifest.name,"purpose":folder.manifest.purpose,"folder":folder.root})
}
fn save_upload(
    folder: &WorkspaceFolder,
    upload: Option<&TemporaryUpload>,
    previous: Option<&WorkspaceAsset>,
    category: &str,
) -> Result<Option<WorkspaceAsset>, String> {
    let Some(upload) = upload else {
        return Ok(None);
    };
    if let Some(previous) = previous.filter(|p| {
        p.sha256 == upload.sha256 && p.bytes == upload.bytes && p.name == upload.file_name
    }) {
        folder.resolve_asset(previous).map_err(issue)?;
        return Ok(Some(previous.clone()));
    }
    let suffix = if upload.file_name.to_lowercase().ends_with(".xes.gz") {
        ".xes.gz".to_owned()
    } else {
        format!(
            ".{}",
            Path::new(&upload.file_name)
                .extension()
                .and_then(|s| s.to_str())
                .ok_or("source has no supported extension")?
        )
    };
    folder
        .store_asset(&upload.path, &upload.file_name, &suffix, category)
        .map(Some)
        .map_err(issue)
}
fn restore_upload(
    folder: &WorkspaceFolder,
    asset: &WorkspaceAsset,
) -> Result<TemporaryUpload, String> {
    let path = folder.resolve_asset(asset).map_err(issue)?;
    let mut entropy = [0u8; 16];
    OsRng.fill_bytes(&mut entropy);
    Ok(TemporaryUpload {
        id: hex(&entropy),
        file_name: asset.name.clone(),
        path,
        file: None,
        bytes: asset.bytes,
        sha256: asset.sha256.clone(),
        delete_on_drop: false,
    })
}

/// The page of the glasstarn that has this project open, when it still answers.
fn running_session(folder: &std::path::Path) -> Option<String> {
    let text = std::fs::read_to_string(folder.join(".browser-session.json")).ok()?;
    let url = serde_json::from_str::<Value>(&text).ok()?["url"]
        .as_str()?
        .to_owned();
    let address = url.strip_prefix("http://")?.trim_end_matches('/');
    let address: std::net::SocketAddr = address.parse().ok()?;
    if !address.ip().is_loopback() {
        return None;
    }
    std::net::TcpStream::connect_timeout(&address, std::time::Duration::from_millis(300)).ok()?;
    Some(url)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn briefing_is_project_scoped_and_hides_old_interpretations() {
        let root = root();
        let folder = WorkspaceFolder::create(
            &root.join("briefing"),
            "11111111111111111111111111111111",
            "Briefing",
            InvestigationPurpose::Improvement,
        )
        .unwrap();
        assert!(investigation_report(&folder).unwrap()["record"].is_null());
        let state = serde_json::to_value(folder.state().unwrap()).unwrap();
        let mut record = json!({"version":1,"project":{"id":folder.manifest.id},"scope":state,"note":{"question":"Why?","findings":[]}});
        let path = folder
            .write_artifact(
                "findings",
                "investigation-100.json",
                &serde_json::to_vec(&record).unwrap(),
            )
            .unwrap();
        assert_eq!(investigation_report(&folder).unwrap()["record"], record);
        record["scope"]["source"] = json!({"different":"source"});
        fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
        let old = investigation_report(&folder).unwrap();
        assert_eq!(old["stale"], true);
        assert!(old["record"].is_null());
        record["project"]["id"] = json!("another-project");
        fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
        assert!(investigation_report(&folder).is_err());
        fs::remove_file(&path).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(root.join("outside.json"), &path).unwrap();
            assert!(investigation_report(&folder).unwrap()["record"].is_null());
        }
        drop(folder);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn earlier_questions_are_listed_newest_first_with_their_answers() {
        let root = root();
        let folder = WorkspaceFolder::create(
            &root.join("history"),
            "22222222222222222222222222222222",
            "History",
            InvestigationPurpose::Improvement,
        )
        .unwrap();
        let state = serde_json::to_value(folder.state().unwrap()).unwrap();
        for (id, question, answer) in [
            (100, "Health check", "Approval holds most of the time."),
            (
                101,
                "Who approves slowest?",
                "Two approvers hold 60% of the wait.",
            ),
            (102, "Why are invoices late?", "Approval waits 6 days."),
        ] {
            let record = json!({"version":1,"project":{"id":folder.manifest.id},"scope":state,
                "recorded_at":"2026-10-10T20:00:00Z",
                "note":{"question":question,"answer":answer,"findings":[{"kind":"observed","title":"A finding","statement":"s"}]}});
            folder
                .write_artifact(
                    "findings",
                    &format!("investigation-{id}.json"),
                    &serde_json::to_vec(&record).unwrap(),
                )
                .unwrap();
        }
        let report = investigation_report(&folder).unwrap();
        assert_eq!(
            report["record"]["note"]["question"],
            "Why are invoices late?"
        );
        let history = report["history"].as_array().unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0]["question"], "Who approves slowest?");
        assert_eq!(history[0]["answer"], "Two approvers hold 60% of the wait.");
        assert_eq!(history[0]["findings"][0]["title"], "A finding");
        assert_eq!(history[0]["current"], true);
        assert_eq!(history[1]["question"], "Health check");
        drop(folder);
        fs::remove_dir_all(root).unwrap();
    }
    fn root() -> PathBuf {
        let mut random = [0u8; 16];
        OsRng.fill_bytes(&mut random);
        let root = std::env::temp_dir().join(format!("pm-folders-{}", hex(&random)));
        fs::create_dir(&root).unwrap();
        root
    }
    fn http(
        server: &Server,
        path: &str,
        body: &str,
        token: &str,
        filename: Option<&str>,
    ) -> (u16, Value) {
        let mut stream = TcpStream::connect(server.address).unwrap();
        let file_header = filename
            .map(|name| format!("X-File-Name: {name}\r\n"))
            .unwrap_or_default();
        write!(stream,"POST {path} HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {token}\r\n{file_header}Content-Length: {}\r\n\r\n{body}",server.address,body.len()).unwrap();
        let (incoming, _) = server.listener.accept().unwrap();
        server.handle(incoming).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        let status = response.split_whitespace().nth(1).unwrap().parse().unwrap();
        let value = serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
        (status, value)
    }
    fn project(root: &Path, id: &str, policy: pm_mcp::ExposurePolicy) -> Server {
        let server = Server::bind_with_agent(0, Some(policy)).unwrap();
        *server.workspace_folder.lock().unwrap() = Some(
            WorkspaceFolder::create(&root.join(id), id, id, InvestigationPurpose::Audit).unwrap(),
        );
        server
    }
    fn import(server: &Server, name: &str, body: &str) -> Value {
        let (status, profile) = http(server, "/api/profile", body, server.token(), Some(name));
        assert_eq!(status, 200, "{profile}");
        let command = json!({"upload_id":profile["upload_id"],"mapping":profile["proposed_mapping"],"filter":{"min_events":2}});
        let (status, result) = http(
            server,
            "/api/analyze",
            &command.to_string(),
            server.token(),
            None,
        );
        assert_eq!(status, 200, "{result}");
        profile
    }
    #[test]
    fn saved_views_can_be_taken_off_the_list() {
        let root = root();
        let id = "44444444444444444444444444444444";
        let mut server = Server::bind_with_workspaces(0, None, &root).unwrap();
        server.workspace_hub_owner = false;
        *server.workspace_folder.lock().unwrap() = Some(
            WorkspaceFolder::create(
                &root.join(id),
                id,
                "Purchasing",
                InvestigationPurpose::Improvement,
            )
            .unwrap(),
        );
        import(
            &server,
            "sample.csv",
            "case_id,activity,timestamp\n1,A,2026-01-01T00:00:00Z\n1,B,2026-01-01T01:00:00Z\n",
        );
        let mut body = json!({"title":"A to B","exploration":{"segment":{"from":"A","to":"B","from_occurrence":"first","to_occurrence":"last"}},"filter":{"min_events":2},"object_type":null});
        for title in ["A to B", "Whole history"] {
            body["title"] = json!(title);
            let (status, result) = http(
                &server,
                "/api/workspace/views",
                &body.to_string(),
                server.token(),
                None,
            );
            assert_eq!(status, 200, "{result}");
        }
        let views = server
            .workspace_folder
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .saved_views()
            .unwrap();
        assert_eq!(views.len(), 2);
        let first = views[0]["id"].as_str().unwrap().to_owned();
        assert!(first.starts_with("saved-view-"));
        let (status, result) = http(
            &server,
            "/api/workspace/views/remove",
            &json!({"id":first}).to_string(),
            server.token(),
            None,
        );
        assert_eq!(status, 200, "{result}");
        let views = server
            .workspace_folder
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .saved_views()
            .unwrap();
        assert_eq!(views.len(), 1);
        assert_ne!(views[0]["id"], json!(first));
        // The removed view stays recoverable in the exports folder.
        assert!(
            root.join(id)
                .join("exports")
                .join(format!("removed-{first}.json"))
                .is_file()
        );
        for bad in ["../state", "saved-view-1/../x", "notes"] {
            let (status, _) = http(
                &server,
                "/api/workspace/views/remove",
                &json!({"id":bad}).to_string(),
                server.token(),
                None,
            );
            assert_ne!(status, 200, "{bad}");
        }
    }
    #[test]
    fn saved_views_keep_the_query_and_scope_and_reject_stale_filters() {
        let root = root();
        let id = "33333333333333333333333333333333";
        let mut server = Server::bind_with_workspaces(0, None, &root).unwrap();
        server.workspace_hub_owner = false;
        *server.workspace_folder.lock().unwrap() = Some(
            WorkspaceFolder::create(
                &root.join(id),
                id,
                "Purchasing",
                InvestigationPurpose::Improvement,
            )
            .unwrap(),
        );
        import(
            &server,
            "sample.csv",
            "case_id,activity,timestamp\n1,A,2026-01-01T00:00:00Z\n1,B,2026-01-01T01:00:00Z\n",
        );
        let mut body = json!({"title":"A to B","exploration":{"segment":{"from":"A","to":"B","from_occurrence":"first","to_occurrence":"last"}},"filter":{"min_events":2},"object_type":null});
        let (status, result) = http(
            &server,
            "/api/workspace/views",
            &body.to_string(),
            server.token(),
            None,
        );
        assert_eq!(status, 200, "{result}");
        let folder = server.workspace_folder.lock().unwrap();
        let views = folder.as_ref().unwrap().saved_views().unwrap();
        assert_eq!(views.len(), 1);
        assert_eq!(views[0]["same_source"], true);
        assert_eq!(views[0]["compatible"], true);
        assert_eq!(views[0]["exploration"]["segment"]["to"], "B");
        assert_eq!(
            views[0]["state"]["case_project"]["view_filter"]["min_events"],
            2
        );
        assert!(views[0].get("result").is_none());
        drop(folder);
        body["filter"] = json!({});
        assert_eq!(
            http(
                &server,
                "/api/workspace/views",
                &body.to_string(),
                server.token(),
                None
            )
            .0,
            409
        );
        body["filter"] = json!({"min_events":2});
        body["exploration"]["html"] = json!("<script>bad</script>");
        assert_eq!(
            http(
                &server,
                "/api/workspace/views",
                &body.to_string(),
                server.token(),
                None
            )
            .0,
            409
        );
        drop(server);
        let mut reopened = WorkspaceFolder::open(&root.join(id)).unwrap();
        assert_eq!(reopened.saved_views().unwrap().len(), 1);
        let mut state = reopened.state().unwrap();
        let saved_rules = state.case_project.as_ref().unwrap().rules.clone();
        state
            .case_project
            .as_mut()
            .unwrap()
            .rules
            .exclude_activities
            .insert("A".into());
        reopened.save_state(&state).unwrap();
        assert_eq!(reopened.saved_views().unwrap()[0]["same_source"], true);
        assert_eq!(reopened.saved_views().unwrap()[0]["compatible"], false);
        state.case_project.as_mut().unwrap().rules = saved_rules;
        state.source.as_mut().unwrap().sha256 = "changed".into();
        reopened.save_state(&state).unwrap();
        assert_eq!(reopened.saved_views().unwrap()[0]["same_source"], false);
        drop(reopened);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn two_projects_keep_sources_filters_baselines_and_agent_credentials_separate_after_reopen() {
        let root = root();
        let policy = pm_mcp::ExposurePolicy::safe();
        let first_id = "11111111111111111111111111111111";
        let second_id = "22222222222222222222222222222222";
        let first = project(&root, first_id, policy.clone());
        let second = project(&root, second_id, policy.clone());
        import(
            &first,
            "first.csv",
            "case_id,activity,timestamp\n1,A,2026-01-01T00:00:00Z\n1,B,2026-01-01T01:00:00Z\n2,A,2026-01-01T00:00:00Z\n",
        );
        import(
            &second,
            "second.csv",
            "case_id,activity,timestamp\n9,X,2026-01-01T00:00:00Z\n9,Y,2026-01-01T01:00:00Z\n",
        );
        let model = json!({"source":"approved", "net":{"place_count":3,"transitions":[{"id":0,"activity":"A","inputs":[0],"outputs":[1]},{"id":1,"activity":"B","inputs":[1],"outputs":[2]}],"initial_marking":[1,0,0],"final_marking":[0,0,1]}});
        let (status, value) = http(
            &first,
            "/api/model",
            &model.to_string(),
            first.token(),
            Some("approved.model.json"),
        );
        assert_eq!(status, 200, "{value}");
        first.save_workspace().unwrap();
        second.save_workspace().unwrap();
        let request = json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}).to_string();
        assert_eq!(
            http(
                &first,
                "/api/mcp",
                &request,
                &policy.project_delegation_token(first_id),
                None
            )
            .0,
            200
        );
        assert_eq!(
            http(
                &second,
                "/api/mcp",
                &request,
                &policy.project_delegation_token(first_id),
                None
            )
            .0,
            401
        );
        assert_eq!(
            http(
                &first,
                "/api/mcp",
                &request,
                &policy.delegation_token(),
                None
            )
            .0,
            401
        );
        drop(first);
        drop(second);
        for (id, name, has_model) in [
            (first_id, "first.csv", true),
            (second_id, "second.csv", false),
        ] {
            let restored = Server::bind_with_agent(0, Some(policy.clone())).unwrap();
            let folder = WorkspaceFolder::open(&root.join(id)).unwrap();
            restored.restore_folder(&folder).unwrap();
            *restored.workspace_folder.lock().unwrap() = Some(folder);
            let state = restored.workspace_state().unwrap();
            assert_eq!(state["profile"]["file_name"], name);
            assert_eq!(state["case_project"]["view_filter"]["min_events"], 2);
            assert_eq!(!state["case_model"].is_null(), has_model);
            restored.save_workspace().unwrap();
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ocel_project_restores_its_object_filter_and_baseline() {
        let root = root();
        let id = "33333333333333333333333333333333";
        let server = project(&root, id, pm_mcp::ExposurePolicy::safe());
        let (status, uploaded) = http(
            &server,
            "/api/ocel",
            include_str!("../../../testdata/ocel/order-management.jsonocel"),
            server.token(),
            Some("orders.jsonocel"),
        );
        assert_eq!(status, 200, "{uploaded}");
        let kind = uploaded["analysis"]["catalog"]["object_types"][0].clone();
        let filter = json!({"object_types":[kind]});
        let command = json!({"upload_id":uploaded["upload_id"],"filter":filter}).to_string();
        let (status, analyzed) = http(&server, "/api/ocel/analyze", &command, server.token(), None);
        assert_eq!(status, 200, "{analyzed}");
        let (status, model) = http(
            &server,
            "/api/ocel/discover",
            &command,
            server.token(),
            None,
        );
        assert_eq!(status, 200, "{model}");
        let (status, accepted) = http(
            &server,
            "/api/ocel/model",
            &model.to_string(),
            server.token(),
            Some("approved.ocelmodel.json"),
        );
        assert_eq!(status, 200, "{accepted}");
        server.save_workspace().unwrap();
        drop(server);
        let restored = Server::bind(0).unwrap();
        let folder = WorkspaceFolder::open(&root.join(id)).unwrap();
        restored.restore_folder(&folder).unwrap();
        *restored.workspace_folder.lock().unwrap() = Some(folder);
        let state = restored.workspace_state().unwrap();
        assert_eq!(state["ocel_filter"]["object_types"], filter["object_types"]);
        assert_eq!(
            state["ocel"]["analysis"]["scope"],
            analyzed["analysis"]["scope"]
        );
        assert_eq!(state["ocel_model"]["file_name"], "approved.ocelmodel.json");
        assert!(state["case_project"].is_null());
        drop(restored);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn hub_reuses_project_session_and_rejects_folder_traversal() {
        let root = root();
        let server = Server::bind_with_workspaces(0, None, &root).unwrap();
        let (status, created) = http(
            &server,
            "/api/workspaces/create",
            r#"{"name":"Quarterly review","purpose":"audit"}"#,
            server.token(),
            None,
        );
        assert_eq!(status, 200, "{created}");
        let body = json!({"id":created["project"]["id"]}).to_string();
        let (status, reopened) = http(&server, "/api/workspaces/open", &body, server.token(), None);
        assert_eq!(status, 200);
        assert_eq!(created["url"], reopened["url"]);
        assert_ne!(created["port"], server.address.port());
        assert_eq!(
            http(
                &server,
                "/api/workspaces/open",
                r#"{"id":"../escape"}"#,
                server.token(),
                None
            )
            .0,
            409
        );
        let folder = root.join(created["project"]["id"].as_str().unwrap());
        assert!(WorkspaceFolder::open(&folder).is_err());
        drop(server);
        for _ in 0..100 {
            if WorkspaceFolder::open(&folder).is_ok() {
                fs::remove_dir_all(root).unwrap();
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("project session did not release its lock at shutdown");
    }
}
