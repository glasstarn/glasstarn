//! Loopback-only HTTP service for the local browser application.
//!
//! This deliberately uses the standard library rather than a general-purpose
//! web framework. The current surface is tiny, and a smaller dependency and
//! parser surface is easier to audit. Requests are bounded before parsing.

mod agent_setup;
mod extracts;
mod follow;
mod provenance;
mod workspaces;

use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const MAX_REQUEST_HEAD: usize = 16 * 1024;
const MAX_UPLOAD_BYTES: u64 = 100 * 1024 * 1024 * 1024;
const MAX_CASE_MODEL_BYTES: u64 = 10 * 1024 * 1024;
const MAX_OCEL_MODEL_BYTES: u64 = 100 * 1024 * 1024;
const MAX_JSON_BODY: u64 = 1024 * 1024;
const READ_TIMEOUT: Duration = Duration::from_secs(2);
const UPLOAD_READ_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("could not bind the local service to 127.0.0.1:{port}: {source}")]
    Bind {
        port: u16,
        #[source]
        source: std::io::Error,
    },
    #[error("local service failed: {0}")]
    Io(#[from] std::io::Error),
}

/// When glasstarn was last used in this process (seconds since 1970), by any
/// of its pages.
static LAST_ACTIVITY: AtomicU64 = AtomicU64::new(0);

fn now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn note_activity() {
    LAST_ACTIVITY.store(now_seconds(), Ordering::Relaxed);
}

/// A newer glasstarn version, when one is known, for the page to mention.
static UPDATE_AVAILABLE: Mutex<Option<String>> = Mutex::new(None);

pub fn set_update_available(version: Option<String>) {
    if let Ok(mut known) = UPDATE_AVAILABLE.lock() {
        *known = version;
    }
}

/// Which installed build this process runs: the size and modification time
/// of its program file when it started. A reinstall changes both, so a later
/// `glasstarn open` can tell a page left running by an older build and
/// restart it instead of showing the old screens.
pub fn build_id() -> &'static str {
    static BUILD: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    BUILD.get_or_init(|| {
        std::env::current_exe()
            .and_then(std::fs::metadata)
            .map(|meta| {
                let modified = meta
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |elapsed| elapsed.as_secs());
                format!("{}-{modified}", meta.len())
            })
            .unwrap_or_default()
    })
}

/// When an open page last checked for new answers: how `glasstarn open`
/// tells a page still showing from one whose tab was closed.
static PAGE_SEEN: AtomicU64 = AtomicU64::new(0);

/// Seconds since an open page last checked in, if one ever has.
pub fn page_seen_seconds_ago() -> Option<u64> {
    let seen = PAGE_SEEN.load(Ordering::Relaxed);
    (seen != 0).then(|| now_seconds().saturating_sub(seen))
}

/// How long since anyone used a glasstarn page in this process. Counts from
/// the start when nothing has been used yet.
pub fn idle_seconds() -> u64 {
    let last = LAST_ACTIVITY.load(Ordering::Relaxed);
    if last == 0 {
        note_activity();
        return 0;
    }
    now_seconds().saturating_sub(last)
}

/// A bound local service with a fresh bearer token.
pub struct Server {
    workspace_hub_owner: bool,
    workspace_hub: Option<Arc<workspaces::WorkspaceHub>>,
    workspace_folder: Arc<Mutex<Option<pm_project::workspace::WorkspaceFolder>>>,
    /// The Salesforce CLI an approved extract runs.
    salesforce_cli: PathBuf,
    listener: TcpListener,
    address: SocketAddr,
    token: String,
    upload: Mutex<Option<TemporaryUpload>>,
    /// The connected source while one of its item types is followed; the
    /// followed case log is then in `upload`.
    ocel_upload: Mutex<Option<TemporaryUpload>>,
    follow_context: Mutex<Option<follow::FollowContext>>,
    case_model: Mutex<Option<TemporaryUpload>>,
    ocel_model: Mutex<Option<TemporaryUpload>>,
    mcp_policy: Mutex<Option<pm_mcp::ExposurePolicy>>,
    agent_contact: Mutex<Option<serde_json::Value>>,
    live_workspace: Mutex<pm_mcp::LiveWorkspace>,
    /// The active view's log, kept between map requests so exploring the map
    /// does not re-read the source on every step. Keyed by upload and view.
    map_log: Mutex<Option<(String, Arc<pm_core::EventLog>)>>,
}

impl Server {
    /// Binds only to IPv4 loopback. Port zero asks the operating system for a
    /// free ephemeral port.
    pub fn bind(port: u16) -> Result<Self, ServerError> {
        Self::bind_with_agent(port, None)
    }

    /// Binds the browser service and optionally enables an independently
    /// authenticated MCP delegation endpoint under the reviewed policy.
    pub fn bind_with_agent(
        port: u16,
        mcp_policy: Option<pm_mcp::ExposurePolicy>,
    ) -> Result<Self, ServerError> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))
            .map_err(|source| ServerError::Bind { port, source })?;
        let address = listener.local_addr()?;
        let mut entropy = [0u8; 32];
        OsRng.fill_bytes(&mut entropy);
        let token = hex(&entropy);
        Ok(Self {
            workspace_hub_owner: false,
            workspace_hub: None,
            workspace_folder: Arc::new(Mutex::new(None)),
            salesforce_cli: pm_project::extract::salesforce_cli(),
            listener,
            address,
            token,
            upload: Mutex::new(None),
            ocel_upload: Mutex::new(None),
            follow_context: Mutex::new(None),
            case_model: Mutex::new(None),
            ocel_model: Mutex::new(None),
            mcp_policy: Mutex::new(mcp_policy),
            agent_contact: Mutex::new(None),
            live_workspace: Mutex::new(pm_mcp::LiveWorkspace::default()),
            map_log: Mutex::new(None),
        })
    }

    pub fn address(&self) -> SocketAddr {
        self.address
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    /// URL handed to the browser. The token stays in the fragment, which is
    /// not transmitted in the initial HTTP request or Referer header.
    pub fn launch_url(&self) -> String {
        format!("http://{}/#token={}", self.address, self.token)
    }

    /// Serves until the shared shutdown flag is set.
    pub fn run(self, shutdown: Arc<AtomicBool>) -> Result<(), ServerError> {
        self.listener.set_nonblocking(true)?;
        while !shutdown.load(Ordering::Acquire) {
            match self.listener.accept() {
                // A malformed, timed-out, or abruptly closed client is a bad
                // request, not a reason to terminate the whole local service.
                Ok((stream, _peer)) => {
                    let _ = self.handle(stream);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }

    fn handle(&self, mut stream: TcpStream) -> Result<(), ServerError> {
        // Accepted sockets can inherit the nonblocking listener mode on macOS.
        // Request bodies may arrive after headers; use the bounded blocking
        // reads below rather than rejecting a valid split TCP request.
        stream.set_nonblocking(false)?;
        stream.set_read_timeout(Some(READ_TIMEOUT))?;
        let Some(request) = Request::read(&mut stream)? else {
            return write_response(
                &mut stream,
                400,
                "Bad Request",
                "text/plain",
                b"bad request",
            );
        };

        if request.host.as_deref() != Some(&self.address.to_string()) {
            return write_response(&mut stream, 403, "Forbidden", "text/plain", b"forbidden");
        }
        if let Some(origin) = request.origin.as_deref()
            && origin != format!("http://{}", self.address)
        {
            return write_response(&mut stream, 403, "Forbidden", "text/plain", b"forbidden");
        }
        let path = request.target.split('?').next().unwrap_or(&request.target);
        // The open page checks for new answers every few seconds; that is not
        // someone using it, so it does not keep glasstarn running.
        if !matches!(path, "/api/workspace/investigation" | "/api/health") {
            note_activity();
        }
        // ...but it does tell `glasstarn open` that a page is still showing.
        if path == "/api/workspace/investigation" {
            PAGE_SEEN.store(now_seconds(), Ordering::Relaxed);
        }
        let authorized = if path == "/api/mcp" {
            self.mcp_policy
                .lock()
                .expect("agent policy lock")
                .as_ref()
                .is_some_and(|policy| {
                    let folder = self.workspace_folder.lock().expect("workspace lock");
                    let token = folder.as_ref().map_or_else(
                        || policy.delegation_token(),
                        |folder| policy.project_delegation_token(&folder.manifest.id),
                    );
                    request.authorized(&token)
                })
        } else {
            // glasstarn's own page needs no key: the browser says the request
            // comes from this very address (Origin, or Sec-Fetch-Site on a
            // same-origin read), and the Host and Origin checks above refuse
            // every other website. Anything that is not a browser on this
            // page, such as another program, still needs the key.
            let own_page = request.origin.as_deref() == Some(&format!("http://{}", self.address))
                || request.fetch_site.as_deref() == Some("same-origin");
            own_page || request.authorized(&self.token)
        };
        if path.starts_with("/api/") && !authorized {
            return write_json(
                &mut stream,
                401,
                "Unauthorized",
                &ErrorResponse::new("unauthorized"),
            );
        }

        if self.workspace_hub_owner
            && request.method == "POST"
            && !path.starts_with("/api/workspaces/")
            && path != "/api/workspace/save"
        {
            return write_json(
                &mut stream,
                409,
                "Conflict",
                &ErrorResponse::new(
                    "create or open a project before loading data or connecting an agent",
                ),
            );
        }
        match (request.method.as_str(), path) {
            ("GET", "/") => {
                write_response(&mut stream, 200, "OK", "text/html; charset=utf-8", INDEX)
            }
            ("GET", "/assets/app.css") => {
                write_response(&mut stream, 200, "OK", "text/css; charset=utf-8", STYLES)
            }
            ("GET", script) if script_asset(script).is_some() => write_response(
                &mut stream,
                200,
                "OK",
                "text/javascript; charset=utf-8",
                script_asset(script).unwrap_or_default(),
            ),
            ("GET", font) if font_asset(font).is_some() => write_response(
                &mut stream,
                200,
                "OK",
                "font/woff2",
                font_asset(font).unwrap_or_default(),
            ),
            ("GET", "/favicon.svg") => {
                write_response(&mut stream, 200, "OK", "image/svg+xml", FAVICON)
            }
            ("GET", "/api/health") => write_json(
                &mut stream,
                200,
                "OK",
                &serde_json::json!({"status":"ok", "project_folders":self.workspace_hub.is_some(), "update_available": UPDATE_AVAILABLE.lock().ok().and_then(|known| known.clone()), "page_seen_seconds_ago": page_seen_seconds_ago()}),
            ),
            (
                "GET",
                "/api/workspaces"
                | "/api/workspace/state"
                | "/api/workspace/investigation"
                | "/api/workspace/views"
                | "/api/workspace/provenance",
            )
            | (
                "POST",
                "/api/workspaces/create"
                | "/api/workspaces/open"
                | "/api/workspaces/rename"
                | "/api/workspaces/delete"
                | "/api/workspace/save"
                | "/api/workspace/views"
                | "/api/workspace/views/remove",
            ) => self.workspace_request(&mut stream, request),
            ("GET", "/api/workspace/extracts")
            | ("POST", "/api/workspace/extracts/approve" | "/api/workspace/extracts/decline") => {
                self.extract_request(&mut stream, request)
            }
            ("GET", "/api/agent") | ("POST", "/api/agent/enable" | "/api/agent/disable") => {
                self.agent_request(&mut stream, request)
            }
            ("POST", "/api/mcp") => self.delegate_mcp(&mut stream, request),
            ("POST", "/api/profile") => self.profile_upload(&mut stream, request),
            ("POST", "/api/ocel") => self.ocel_upload(&mut stream, request),
            ("POST", "/api/ocel/analyze") => self.ocel_analyze(&mut stream, request),
            ("POST", "/api/ocel/follow") => self.follow_request(&mut stream, request),
            ("POST", "/api/ocel/journey") => self.journey_request(&mut stream, request),
            ("POST", "/api/ocel/scope") => self.connected_scope_request(&mut stream, request),
            ("POST", "/api/ocel/object") => self.ocel_object(&mut stream, request),
            ("POST", "/api/ocel/time-between") => self.ocel_time_between(&mut stream, request),
            ("POST", "/api/ocel/synchronization") => {
                self.ocel_synchronization(&mut stream, request)
            }
            ("POST", "/api/ocel/bottlenecks") => self.ocel_bottlenecks(&mut stream, request),
            ("POST", "/api/ocel/project") => self.ocel_projection(&mut stream, request),
            ("POST", "/api/ocel/discover") => self.ocel_discovery(&mut stream, request),
            ("POST", "/api/ocel/model") => self.ocel_model_upload(&mut stream, request),
            ("POST", "/api/ocel/conformance") => self.ocel_conformance(&mut stream, request),
            ("POST", "/api/analyze") => self.analyze_upload(&mut stream, request),
            ("POST", "/api/ocel/explore") => self.explore_linked(&mut stream, request),
            ("POST", "/api/explore") => self.explore(&mut stream, request),
            ("POST", "/api/compare/options") => self.comparison_options(&mut stream, request),
            ("POST", "/api/filter/options") => self.attribute_choices(&mut stream, request),
            ("POST", "/api/compare/cohorts") => self.compare_cohorts(&mut stream, request),
            ("POST", "/api/values") => self.values(&mut stream, request),
            ("POST", "/api/map") => self.map_view(&mut stream, request, false),
            ("POST", "/api/map/bpmn") => self.map_view(&mut stream, request, true),
            ("POST", "/api/items") => self.items(&mut stream, request),
            ("POST", "/api/compare/periods") => self.compare_periods(&mut stream, request),
            ("POST", "/api/discover") => self.discover_upload(&mut stream, request),
            ("POST", "/api/model") => self.case_model_upload(&mut stream, request),
            ("POST", "/api/conformance") => self.case_conformance(&mut stream, request),
            ("POST", "/api/case") => self.case_detail(&mut stream, request),
            ("POST", "/api/project/open") => self.open_project(&mut stream, request),
            ("POST", "/api/project/save") => self.save_project(&mut stream, request),
            ("POST", "/api/project/reopen") => self.reopen_project(&mut stream, request),
            ("GET", _) => write_response(&mut stream, 404, "Not Found", "text/plain", b"not found"),
            ("POST", path) if path.starts_with("/api/") => {
                write_response(&mut stream, 404, "Not Found", "text/plain", b"not found")
            }
            _ => write_response(
                &mut stream,
                405,
                "Method Not Allowed",
                "text/plain",
                b"method not allowed",
            ),
        }
    }

    fn delegate_mcp(&self, stream: &mut TcpStream, request: Request) -> Result<(), ServerError> {
        let policy = self.mcp_policy.lock().expect("agent policy lock").clone();
        let Some(policy) = policy else {
            return write_json(
                stream,
                404,
                "Not Found",
                &ErrorResponse::new("live agent access is not enabled"),
            );
        };
        let body = match request.json_body(stream) {
            Ok(body) => body,
            Err(error) => return write_request_error(stream, error, "MCP delegation"),
        };
        let message: serde_json::Value = match serde_json::from_slice(&body) {
            Ok(message) => message,
            Err(_) => {
                return write_json(
                    stream,
                    400,
                    "Bad Request",
                    &ErrorResponse::new("the delegated MCP request is not valid JSON"),
                );
            }
        };
        let workspace = match self.live_workspace.lock() {
            Ok(workspace) => workspace.clone(),
            Err(_) => {
                return write_json(
                    stream,
                    500,
                    "Internal Server Error",
                    &ErrorResponse::new("the live analysis workspace is unavailable"),
                );
            }
        };
        let server = match pm_mcp::Server::with_live_workspace(workspace, policy.clone()) {
            Ok(server) => server,
            Err(message) => {
                return write_json(
                    stream,
                    500,
                    "Internal Server Error",
                    &ErrorResponse::new(&message),
                );
            }
        };
        let response = server.handle(message.clone());
        if response
            .as_ref()
            .is_none_or(|value| value.get("error").is_none())
        {
            self.record_agent_contact(&message);
        }
        match response {
            Some(response) => write_json(stream, 200, "OK", &response),
            None => write_response(stream, 204, "No Content", "application/json", b""),
        }
    }

    fn set_live_case_project(&self, project: pm_project::Project, source_name: String) -> bool {
        // A followed item type is a view of the connected project, not a new
        // case project; the agent and saving keep seeing the connected log.
        if self.following() {
            // Until the linked cohort is resolved, expose an empty boundary
            // rather than the previous/global population to concurrent agents.
            let root = self.follow_context.lock().ok().and_then(|mut context| {
                context.as_mut().map(|context| {
                    context.case_filter = project.view_filter.clone();
                    context.object_type.clone()
                })
            });
            return match (root, self.live_workspace.lock()) {
                (Some(root_type), Ok(mut workspace)) => {
                    workspace.ocel_log.as_mut().is_some_and(|log| {
                        log.filter.cohort = Some(pm_core::OcelCohort {
                            root_type,
                            ..Default::default()
                        });
                        true
                    })
                }
                _ => false,
            };
        }
        match self.live_workspace.lock() {
            Ok(mut workspace) => {
                workspace.ocel_log = None;
                workspace.case_project = Some(pm_mcp::LiveProject {
                    project,
                    source_name,
                });
                true
            }
            Err(_) => false,
        }
    }

    fn set_live_ocel_filter(&self, filter: pm_core::OcelFilter) -> bool {
        let changed = match self.live_workspace.lock() {
            Ok(mut workspace) => match workspace.ocel_log.as_mut() {
                Some(log) => {
                    log.filter = filter.clone();
                    true
                }
                None => false,
            },
            Err(_) => false,
        };
        // A native selection replaces the base scope. Later case filters and
        // type changes must not silently restore the previous native scope.
        if changed
            && let Ok(mut context) = self.follow_context.lock()
            && let Some(context) = context.as_mut()
        {
            context.base_filter = filter;
            context.case_filter = Default::default();
            context.journey = None;
        }
        changed
    }

    fn profile_upload(&self, stream: &mut TcpStream, request: Request) -> Result<(), ServerError> {
        self.stop_following();
        let Some(length) = request.content_length else {
            return write_json(
                stream,
                411,
                "Length Required",
                &ErrorResponse::new("upload length is required"),
            );
        };
        if length == 0 {
            return write_json(
                stream,
                400,
                "Bad Request",
                &ErrorResponse::new("the selected file is empty"),
            );
        }
        if length > MAX_UPLOAD_BYTES {
            return write_json(
                stream,
                413,
                "Content Too Large",
                &ErrorResponse::new("the selected file exceeds the 100 GiB upload limit"),
            );
        }
        let Some(encoded_name) = request.file_name.as_deref() else {
            return write_json(
                stream,
                400,
                "Bad Request",
                &ErrorResponse::new("the file name is required"),
            );
        };
        let Some(file_name) = decode_file_name(encoded_name) else {
            return write_json(
                stream,
                400,
                "Bad Request",
                &ErrorResponse::new("the file name is not valid"),
            );
        };
        let Some(suffix) = supported_suffix(&file_name) else {
            return write_json(
                stream,
                415,
                "Unsupported Media Type",
                &ErrorResponse::new("choose a CSV, Parquet, XES, or XES.gz file"),
            );
        };

        let mut temporary = match TemporaryUpload::create(suffix) {
            Ok(upload) => upload,
            Err(error) => {
                return write_json(
                    stream,
                    500,
                    "Internal Server Error",
                    &ErrorResponse::new(&format!("could not prepare the upload: {error}")),
                );
            }
        };
        temporary.file_name.clone_from(&file_name);
        stream.set_read_timeout(Some(UPLOAD_READ_TIMEOUT))?;
        if let Err(error) = temporary.write_request(stream, length, &request.body_prefix) {
            return write_json(
                stream,
                400,
                "Bad Request",
                &ErrorResponse::new(&format!("could not receive the upload: {error}")),
            );
        }

        let (schema, sample) =
            match pm_ingest::sample_rows(&temporary.path, pm_ingest::PROFILE_ROWS) {
                Ok(profile) => profile,
                Err(error) => {
                    return write_json(
                        stream,
                        422,
                        "Unprocessable Content",
                        &ErrorResponse::new(&format!("could not read the event log: {error}")),
                    );
                }
            };
        let profile = pm_ingest::Profile::build(&schema, &sample);
        let upload_id = temporary.id.clone();
        let response = ProfileResponse {
            upload_id,
            file_name,
            bytes: length,
            rows_sampled: profile.rows_sampled,
            columns: profile.columns.iter().map(ColumnResponse::from).collect(),
            proposed_mapping: pm_query::propose_mapping(schema.headers()),
        };
        match self.upload.lock() {
            Ok(mut stored) => *stored = Some(temporary),
            Err(_) => {
                return write_json(
                    stream,
                    500,
                    "Internal Server Error",
                    &ErrorResponse::new("the upload store is unavailable"),
                );
            }
        }
        if let Ok(mut model) = self.case_model.lock() {
            *model = None;
        }
        match self.live_workspace.lock() {
            Ok(mut workspace) => *workspace = pm_mcp::LiveWorkspace::default(),
            Err(_) => {
                return write_json(
                    stream,
                    500,
                    "Internal Server Error",
                    &ErrorResponse::new("the live analysis workspace is unavailable"),
                );
            }
        }
        write_json(stream, 200, "OK", &response)
    }

    fn ocel_upload(&self, stream: &mut TcpStream, request: Request) -> Result<(), ServerError> {
        self.stop_following();
        let Some(length) = request.content_length else {
            return write_json(
                stream,
                411,
                "Length Required",
                &ErrorResponse::new("upload length is required"),
            );
        };
        if length == 0 {
            return write_json(
                stream,
                400,
                "Bad Request",
                &ErrorResponse::new("the selected file is empty"),
            );
        }
        if length > MAX_UPLOAD_BYTES {
            return write_json(
                stream,
                413,
                "Content Too Large",
                &ErrorResponse::new("the selected file exceeds the 100 GiB upload limit"),
            );
        }
        let Some(encoded_name) = request.file_name.as_deref() else {
            return write_json(
                stream,
                400,
                "Bad Request",
                &ErrorResponse::new("the file name is required"),
            );
        };
        let Some(file_name) = decode_file_name(encoded_name) else {
            return write_json(
                stream,
                400,
                "Bad Request",
                &ErrorResponse::new("the file name is not valid"),
            );
        };
        if !pm_ingest::is_ocel(&file_name) {
            return write_json(
                stream,
                415,
                "Unsupported Media Type",
                &ErrorResponse::new(
                    "choose an OCEL 2.0 .jsonocel, .xmlocel, .xml, .sqlite, or .sqlite3 file",
                ),
            );
        }
        let suffix = if file_name.to_ascii_lowercase().ends_with(".jsonocel") {
            ".jsonocel"
        } else if file_name.to_ascii_lowercase().ends_with(".xmlocel") {
            ".xmlocel"
        } else if file_name.to_ascii_lowercase().ends_with(".sqlite3") {
            ".sqlite3"
        } else if file_name.to_ascii_lowercase().ends_with(".sqlite") {
            ".sqlite"
        } else {
            ".xml"
        };
        let mut temporary = match TemporaryUpload::create(suffix) {
            Ok(upload) => upload,
            Err(error) => {
                return write_json(
                    stream,
                    500,
                    "Internal Server Error",
                    &ErrorResponse::new(&format!("could not prepare the upload: {error}")),
                );
            }
        };
        temporary.file_name.clone_from(&file_name);
        stream.set_read_timeout(Some(UPLOAD_READ_TIMEOUT))?;
        if let Err(error) = temporary.write_request(stream, length, &request.body_prefix) {
            return write_json(
                stream,
                400,
                "Bad Request",
                &ErrorResponse::new(&format!("could not receive the upload: {error}")),
            );
        }
        let mut analysis =
            match pm_query::analyze_ocel(&temporary.path, &pm_core::OcelFilter::default()) {
                Ok(analysis) => analysis,
                Err(error) => {
                    return write_json(
                        stream,
                        422,
                        "Unprocessable Content",
                        &ErrorResponse::new(&format!(
                            "could not read the object-centric log: {error}"
                        )),
                    );
                }
            };
        analysis.source.clone_from(&file_name);
        let upload_id = temporary.id.clone();
        let live_log = pm_mcp::LiveOcelLog {
            path: temporary.path.clone(),
            source_name: file_name.clone(),
            filter: pm_core::OcelFilter::default(),
        };
        match self.upload.lock() {
            Ok(mut stored) => *stored = Some(temporary),
            Err(_) => {
                return write_json(
                    stream,
                    500,
                    "Internal Server Error",
                    &ErrorResponse::new("the upload store is unavailable"),
                );
            }
        }
        match self.live_workspace.lock() {
            Ok(mut workspace) => {
                workspace.case_model = None;
                workspace.case_project = None;
                workspace.ocel_log = Some(live_log);
                workspace.ocel_model = None;
            }
            Err(_) => {
                return write_json(
                    stream,
                    500,
                    "Internal Server Error",
                    &ErrorResponse::new("the live analysis workspace is unavailable"),
                );
            }
        }
        write_json(
            stream,
            200,
            "OK",
            &OcelAnalysisResponse {
                upload_id,
                analysis,
            },
        )
    }

    fn ocel_analyze(&self, stream: &mut TcpStream, request: Request) -> Result<(), ServerError> {
        let body = match request.json_body(stream) {
            Ok(body) => body,
            Err(error) => return write_request_error(stream, error, "object-centric analysis"),
        };
        let command: OcelAnalysisCommand = match serde_json::from_slice(&body) {
            Ok(command) => command,
            Err(_) => {
                return write_json(
                    stream,
                    400,
                    "Bad Request",
                    &ErrorResponse::new("the object-centric analysis request is not valid JSON"),
                );
            }
        };
        let stored = match self.ocel_store().lock() {
            Ok(stored) => stored,
            Err(_) => {
                return write_json(
                    stream,
                    500,
                    "Internal Server Error",
                    &ErrorResponse::new("the upload store is unavailable"),
                );
            }
        };
        let Some(upload) = stored
            .as_ref()
            .filter(|upload| upload.id == command.upload_id)
        else {
            return write_json(
                stream,
                404,
                "Not Found",
                &ErrorResponse::new("the uploaded file is no longer available; choose it again"),
            );
        };
        let mut analysis = match pm_query::analyze_ocel(&upload.path, &command.filter) {
            Ok(analysis) => analysis,
            Err(error) => {
                return write_json(
                    stream,
                    422,
                    "Unprocessable Content",
                    &ErrorResponse::new(&format!(
                        "could not analyze the object-centric log: {error}"
                    )),
                );
            }
        };
        analysis.source.clone_from(&upload.file_name);
        let upload_id = upload.id.clone();
        drop(stored);
        if !self.set_live_ocel_filter(command.filter) {
            return write_json(
                stream,
                500,
                "Internal Server Error",
                &ErrorResponse::new("the live object-centric workspace is unavailable"),
            );
        }
        write_json(
            stream,
            200,
            "OK",
            &OcelAnalysisResponse {
                upload_id,
                analysis,
            },
        )
    }

    fn ocel_object(&self, stream: &mut TcpStream, request: Request) -> Result<(), ServerError> {
        let body = match request.json_body(stream) {
            Ok(body) => body,
            Err(error) => return write_request_error(stream, error, "object lifecycle"),
        };
        let command: OcelObjectCommand = match serde_json::from_slice(&body) {
            Ok(command) => command,
            Err(_) => {
                return write_json(
                    stream,
                    400,
                    "Bad Request",
                    &ErrorResponse::new("the object lifecycle request is not valid JSON"),
                );
            }
        };
        let stored = match self.ocel_store().lock() {
            Ok(stored) => stored,
            Err(_) => {
                return write_json(
                    stream,
                    500,
                    "Internal Server Error",
                    &ErrorResponse::new("the upload store is unavailable"),
                );
            }
        };
        let Some(upload) = stored
            .as_ref()
            .filter(|upload| upload.id == command.upload_id)
        else {
            return write_json(
                stream,
                404,
                "Not Found",
                &ErrorResponse::new("the uploaded file is no longer available; choose it again"),
            );
        };
        let lifecycle =
            match pm_query::inspect_ocel_object(&upload.path, &command.filter, &command.object) {
                Ok(lifecycle) => lifecycle,
                Err(error) => {
                    return write_json(
                        stream,
                        422,
                        "Unprocessable Content",
                        &ErrorResponse::new(&format!(
                            "could not inspect the object lifecycle: {error}"
                        )),
                    );
                }
            };
        write_json(
            stream,
            200,
            "OK",
            &OcelObjectResponse {
                upload_id: upload.id.clone(),
                lifecycle,
            },
        )
    }

    fn ocel_time_between(
        &self,
        stream: &mut TcpStream,
        request: Request,
    ) -> Result<(), ServerError> {
        let body = match request.json_body(stream) {
            Ok(body) => body,
            Err(error) => return write_request_error(stream, error, "object-centric timing"),
        };
        let command: OcelTimeBetweenCommand = match serde_json::from_slice(&body) {
            Ok(command) => command,
            Err(_) => {
                return write_json(
                    stream,
                    400,
                    "Bad Request",
                    &ErrorResponse::new("the object-centric timing request is not valid JSON"),
                );
            }
        };
        let stored = match self.ocel_store().lock() {
            Ok(stored) => stored,
            Err(_) => {
                return write_json(
                    stream,
                    500,
                    "Internal Server Error",
                    &ErrorResponse::new("the upload store is unavailable"),
                );
            }
        };
        let Some(upload) = stored
            .as_ref()
            .filter(|upload| upload.id == command.upload_id)
        else {
            return write_json(
                stream,
                404,
                "Not Found",
                &ErrorResponse::new("the uploaded file is no longer available; choose it again"),
            );
        };
        let report = match pm_query::measure_ocel_time_between(
            &upload.path,
            &command.filter,
            &command.from_activity,
            &command.to_activity,
        ) {
            Ok(report) => report,
            Err(error) => {
                return write_json(
                    stream,
                    422,
                    "Unprocessable Content",
                    &ErrorResponse::new(&format!(
                        "could not measure object-centric elapsed time: {error}"
                    )),
                );
            }
        };
        write_json(stream, 200, "OK", &report)
    }

    fn ocel_synchronization(
        &self,
        stream: &mut TcpStream,
        request: Request,
    ) -> Result<(), ServerError> {
        let body = match request.json_body(stream) {
            Ok(body) => body,
            Err(error) => return write_request_error(stream, error, "object synchronization"),
        };
        let command: OcelSynchronizationCommand = match serde_json::from_slice(&body) {
            Ok(command) => command,
            Err(_) => {
                return write_json(
                    stream,
                    400,
                    "Bad Request",
                    &ErrorResponse::new("the object synchronization request is not valid JSON"),
                );
            }
        };
        let stored = match self.ocel_store().lock() {
            Ok(stored) => stored,
            Err(_) => {
                return write_json(
                    stream,
                    500,
                    "Internal Server Error",
                    &ErrorResponse::new("the upload store is unavailable"),
                );
            }
        };
        let Some(upload) = stored
            .as_ref()
            .filter(|upload| upload.id == command.upload_id)
        else {
            return write_json(
                stream,
                404,
                "Not Found",
                &ErrorResponse::new("the uploaded file is no longer available; choose it again"),
            );
        };
        let report = match pm_query::analyze_ocel_synchronization(&upload.path, &command.filter) {
            Ok(report) => report,
            Err(error) => {
                return write_json(
                    stream,
                    422,
                    "Unprocessable Content",
                    &ErrorResponse::new(&format!(
                        "could not analyze object synchronization: {error}"
                    )),
                );
            }
        };
        write_json(stream, 200, "OK", &report)
    }

    fn ocel_bottlenecks(
        &self,
        stream: &mut TcpStream,
        request: Request,
    ) -> Result<(), ServerError> {
        let body = match request.json_body(stream) {
            Ok(body) => body,
            Err(error) => return write_request_error(stream, error, "object bottlenecks"),
        };
        let command: OcelBottleneckCommand = match serde_json::from_slice(&body) {
            Ok(command) => command,
            Err(_) => {
                return write_json(
                    stream,
                    400,
                    "Bad Request",
                    &ErrorResponse::new("the object bottleneck request is not valid JSON"),
                );
            }
        };
        let stored = match self.ocel_store().lock() {
            Ok(stored) => stored,
            Err(_) => {
                return write_json(
                    stream,
                    500,
                    "Internal Server Error",
                    &ErrorResponse::new("the upload store is unavailable"),
                );
            }
        };
        let Some(upload) = stored
            .as_ref()
            .filter(|upload| upload.id == command.upload_id)
        else {
            return write_json(
                stream,
                404,
                "Not Found",
                &ErrorResponse::new("the uploaded file is no longer available; choose it again"),
            );
        };
        let report = match pm_query::rank_ocel_bottlenecks(&upload.path, &command.filter) {
            Ok(report) => report,
            Err(error) => {
                return write_json(
                    stream,
                    422,
                    "Unprocessable Content",
                    &ErrorResponse::new(&format!(
                        "could not rank object-centric bottlenecks: {error}"
                    )),
                );
            }
        };
        write_json(stream, 200, "OK", &report)
    }

    fn ocel_projection(&self, stream: &mut TcpStream, request: Request) -> Result<(), ServerError> {
        let body = match request.json_body(stream) {
            Ok(body) => body,
            Err(error) => return write_request_error(stream, error, "object projection"),
        };
        let command: OcelProjectionCommand = match serde_json::from_slice(&body) {
            Ok(command) => command,
            Err(_) => {
                return write_json(
                    stream,
                    400,
                    "Bad Request",
                    &ErrorResponse::new("the object projection request is not valid JSON"),
                );
            }
        };
        if !(1..=20).contains(&command.case_limit)
            || !(1..=200).contains(&command.event_limit_per_case)
        {
            return write_json(
                stream,
                400,
                "Bad Request",
                &ErrorResponse::new(
                    "projection limits must be 1–20 cases and 1–200 events per case",
                ),
            );
        }
        let stored = match self.ocel_store().lock() {
            Ok(stored) => stored,
            Err(_) => {
                return write_json(
                    stream,
                    500,
                    "Internal Server Error",
                    &ErrorResponse::new("the upload store is unavailable"),
                );
            }
        };
        let Some(upload) = stored
            .as_ref()
            .filter(|upload| upload.id == command.upload_id)
        else {
            return write_json(
                stream,
                404,
                "Not Found",
                &ErrorResponse::new("the uploaded file is no longer available; choose it again"),
            );
        };
        let report = match pm_query::project_ocel_cases(
            &upload.path,
            &command.filter,
            &command.case_object_type,
            command.case_limit,
            command.event_limit_per_case,
        ) {
            Ok(report) => report,
            Err(error) => {
                return write_json(
                    stream,
                    422,
                    "Unprocessable Content",
                    &ErrorResponse::new(&format!(
                        "could not project object-centric cases: {error}"
                    )),
                );
            }
        };
        write_json(stream, 200, "OK", &report)
    }

    fn ocel_discovery(&self, stream: &mut TcpStream, request: Request) -> Result<(), ServerError> {
        let body = match request.json_body(stream) {
            Ok(body) => body,
            Err(error) => return write_request_error(stream, error, "object model discovery"),
        };
        let command: OcelDiscoveryCommand = match serde_json::from_slice(&body) {
            Ok(command) => command,
            Err(_) => {
                return write_json(
                    stream,
                    400,
                    "Bad Request",
                    &ErrorResponse::new("the object model discovery request is not valid JSON"),
                );
            }
        };
        if command.noise_threshold_milli > 1_000 || command.variable_arc_threshold_milli > 1_000 {
            return write_json(
                stream,
                400,
                "Bad Request",
                &ErrorResponse::new("discovery thresholds must be between 0 and 1000 thousandths"),
            );
        }
        let stored = match self.ocel_store().lock() {
            Ok(stored) => stored,
            Err(_) => {
                return write_json(
                    stream,
                    500,
                    "Internal Server Error",
                    &ErrorResponse::new("the upload store is unavailable"),
                );
            }
        };
        let Some(upload) = stored
            .as_ref()
            .filter(|upload| upload.id == command.upload_id)
        else {
            return write_json(
                stream,
                404,
                "Not Found",
                &ErrorResponse::new("the uploaded file is no longer available; choose it again"),
            );
        };
        let mut report = match pm_query::discover_ocel_model(
            &upload.path,
            &command.filter,
            pm_core::OcelDiscoveryConfig {
                noise_threshold_milli: command.noise_threshold_milli,
                variable_arc_threshold_milli: command.variable_arc_threshold_milli,
            },
        ) {
            Ok(report) => report,
            Err(error) => {
                return write_json(
                    stream,
                    422,
                    "Unprocessable Content",
                    &ErrorResponse::new(&format!(
                        "could not discover the object-centric model: {error}"
                    )),
                );
            }
        };
        report.source.clone_from(&upload.file_name);
        write_json(stream, 200, "OK", &report)
    }

    fn ocel_model_upload(
        &self,
        stream: &mut TcpStream,
        request: Request,
    ) -> Result<(), ServerError> {
        let Some(length) = request.content_length else {
            return write_json(
                stream,
                411,
                "Length Required",
                &ErrorResponse::new("the model file length is required"),
            );
        };
        if length == 0 {
            return write_json(
                stream,
                400,
                "Bad Request",
                &ErrorResponse::new("the selected model file is empty"),
            );
        }
        if length > MAX_OCEL_MODEL_BYTES {
            return write_json(
                stream,
                413,
                "Content Too Large",
                &ErrorResponse::new("the object-centric model exceeds the 100 MiB safety limit"),
            );
        }
        let Some(encoded_name) = request.file_name.as_deref() else {
            return write_json(
                stream,
                400,
                "Bad Request",
                &ErrorResponse::new("the model file name is required"),
            );
        };
        let Some(file_name) = decode_file_name(encoded_name) else {
            return write_json(
                stream,
                400,
                "Bad Request",
                &ErrorResponse::new("the model file name is not valid"),
            );
        };
        if !file_name.to_ascii_lowercase().ends_with(".ocelmodel.json") {
            return write_json(
                stream,
                415,
                "Unsupported Media Type",
                &ErrorResponse::new("choose a portable .ocelmodel.json baseline"),
            );
        }
        let mut temporary = match TemporaryUpload::create(".ocelmodel.json") {
            Ok(upload) => upload,
            Err(error) => {
                return write_json(
                    stream,
                    500,
                    "Internal Server Error",
                    &ErrorResponse::new(&format!("could not prepare the model upload: {error}")),
                );
            }
        };
        temporary.file_name.clone_from(&file_name);
        stream.set_read_timeout(Some(UPLOAD_READ_TIMEOUT))?;
        if let Err(error) = temporary.write_request(stream, length, &request.body_prefix) {
            return write_json(
                stream,
                400,
                "Bad Request",
                &ErrorResponse::new(&format!("could not receive the model upload: {error}")),
            );
        }
        let bytes = match fs::read(&temporary.path) {
            Ok(bytes) => bytes,
            Err(error) => {
                return write_json(
                    stream,
                    500,
                    "Internal Server Error",
                    &ErrorResponse::new(&format!("could not read the uploaded model: {error}")),
                );
            }
        };
        let model: pm_query::OcelModelReport = match serde_json::from_slice(&bytes) {
            Ok(model) => model,
            Err(error) => {
                return write_json(
                    stream,
                    422,
                    "Unprocessable Content",
                    &ErrorResponse::new(&format!(
                        "the selected file is not a portable object-centric model: {error}"
                    )),
                );
            }
        };
        let model_nodes = model.places.len().saturating_add(model.transitions.len());
        if model_nodes == 0 || model_nodes > 10_000 {
            return write_json(
                stream,
                422,
                "Unprocessable Content",
                &ErrorResponse::new(
                    "the model must contain between 1 and 10,000 places and transitions",
                ),
            );
        }
        let response = OcelModelUploadResponse {
            model_id: temporary.id.clone(),
            file_name,
            places: model.places.len(),
            transitions: model.transitions.len(),
            object_types: model.object_types.len(),
        };
        let live_model = pm_mcp::LiveOcelModel {
            path: temporary.path.clone(),
            source_name: response.file_name.clone(),
        };
        match self.ocel_model.lock() {
            Ok(mut stored) => *stored = Some(temporary),
            Err(_) => {
                return write_json(
                    stream,
                    500,
                    "Internal Server Error",
                    &ErrorResponse::new("the model upload store is unavailable"),
                );
            }
        }
        match self.live_workspace.lock() {
            Ok(mut workspace) => workspace.ocel_model = Some(live_model),
            Err(_) => {
                return write_json(
                    stream,
                    500,
                    "Internal Server Error",
                    &ErrorResponse::new("the live analysis workspace is unavailable"),
                );
            }
        }
        write_json(stream, 200, "OK", &response)
    }

    fn ocel_conformance(
        &self,
        stream: &mut TcpStream,
        request: Request,
    ) -> Result<(), ServerError> {
        let body = match request.json_body(stream) {
            Ok(body) => body,
            Err(error) => return write_request_error(stream, error, "object conformance"),
        };
        let command: OcelConformanceCommand = match serde_json::from_slice(&body) {
            Ok(command) => command,
            Err(_) => {
                return write_json(
                    stream,
                    400,
                    "Bad Request",
                    &ErrorResponse::new("the object conformance request is not valid JSON"),
                );
            }
        };
        if !(1..=200).contains(&command.detail_limit) {
            return write_json(
                stream,
                400,
                "Bad Request",
                &ErrorResponse::new("the conformance detail limit must be between 1 and 200"),
            );
        }
        let (log_path, log_name) = {
            let stored = match self.ocel_store().lock() {
                Ok(stored) => stored,
                Err(_) => {
                    return write_json(
                        stream,
                        500,
                        "Internal Server Error",
                        &ErrorResponse::new("the upload store is unavailable"),
                    );
                }
            };
            let Some(upload) = stored
                .as_ref()
                .filter(|upload| upload.id == command.upload_id)
            else {
                return write_json(
                    stream,
                    404,
                    "Not Found",
                    &ErrorResponse::new("the uploaded log is no longer available; choose it again"),
                );
            };
            (upload.path.clone(), upload.file_name.clone())
        };
        let (model_path, model_name) = {
            let stored = match self.ocel_model.lock() {
                Ok(stored) => stored,
                Err(_) => {
                    return write_json(
                        stream,
                        500,
                        "Internal Server Error",
                        &ErrorResponse::new("the model upload store is unavailable"),
                    );
                }
            };
            let Some(model) = stored.as_ref().filter(|model| model.id == command.model_id) else {
                return write_json(
                    stream,
                    404,
                    "Not Found",
                    &ErrorResponse::new(
                        "the uploaded baseline is no longer available; choose it again",
                    ),
                );
            };
            (model.path.clone(), model.file_name.clone())
        };
        match command.algorithm {
            OcelConformanceAlgorithm::Token => {
                let mut report = match pm_query::check_ocel_conformance(
                    &log_path,
                    &model_path,
                    &command.filter,
                    command.detail_limit,
                ) {
                    Ok(report) => report,
                    Err(error) => {
                        return write_json(
                            stream,
                            422,
                            "Unprocessable Content",
                            &ErrorResponse::new(&format!(
                                "could not check object-centric conformance: {error}"
                            )),
                        );
                    }
                };
                sanitize_ocel_model_sources(
                    &mut report.source,
                    &mut report.model_source,
                    &mut report.model_training_source,
                    log_name,
                    model_name,
                );
                write_json(stream, 200, "OK", &report)
            }
            OcelConformanceAlgorithm::Alignment => {
                let mut report = match pm_query::align_ocel_conformance(
                    &log_path,
                    &model_path,
                    &command.filter,
                    command.detail_limit,
                ) {
                    Ok(report) => report,
                    Err(error) => {
                        return write_json(
                            stream,
                            422,
                            "Unprocessable Content",
                            &ErrorResponse::new(&format!(
                                "could not align object-centric conformance: {error}"
                            )),
                        );
                    }
                };
                sanitize_ocel_model_sources(
                    &mut report.source,
                    &mut report.model_source,
                    &mut report.model_training_source,
                    log_name,
                    model_name,
                );
                write_json(stream, 200, "OK", &report)
            }
        }
    }

    fn analyze_upload(&self, stream: &mut TcpStream, request: Request) -> Result<(), ServerError> {
        let body = match request.json_body(stream) {
            Ok(body) => body,
            Err((status, reason, message)) => {
                return write_json(stream, status, reason, &ErrorResponse::new(message));
            }
        };
        let command: AnalyzeRequest = match serde_json::from_slice(&body) {
            Ok(command) => command,
            Err(_) => {
                return write_json(
                    stream,
                    400,
                    "Bad Request",
                    &ErrorResponse::new("the analysis request is not valid JSON"),
                );
            }
        };
        let stored = match self.upload.lock() {
            Ok(stored) => stored,
            Err(_) => {
                return write_json(
                    stream,
                    500,
                    "Internal Server Error",
                    &ErrorResponse::new("the upload store is unavailable"),
                );
            }
        };
        let Some(upload) = stored
            .as_ref()
            .filter(|upload| upload.id == command.upload_id)
        else {
            return write_json(
                stream,
                404,
                "Not Found",
                &ErrorResponse::new("the uploaded file is no longer available; choose it again"),
            );
        };
        let live_project = project_from_upload(
            upload,
            command.mapping.clone(),
            command.policy.clone(),
            command.rules.clone(),
            command.filter.clone(),
            upload.file_name.clone(),
        );
        match pm_query::analyze_filtered(
            &upload.path,
            Some(command.mapping.clone()),
            &command.policy,
            command.rules,
            command.filter,
        ) {
            Ok(pm_query::Outcome::Complete(mut analysis)) => {
                analysis.source.clone_from(&upload.file_name);
                if !self.set_live_case_project(live_project, upload.file_name.clone()) {
                    return write_json(
                        stream,
                        500,
                        "Internal Server Error",
                        &ErrorResponse::new("the live analysis workspace is unavailable"),
                    );
                }
                write_json(stream, 200, "OK", &AnalysisResponse::Complete { analysis })
            }
            Ok(pm_query::Outcome::Blocked(_)) => {
                if !self.set_live_case_project(live_project, upload.file_name.clone()) {
                    return write_json(
                        stream,
                        500,
                        "Internal Server Error",
                        &ErrorResponse::new("the live analysis workspace is unavailable"),
                    );
                }
                match pm_query::assess_quality(&upload.path, Some(command.mapping), &command.policy)
                {
                    Ok(quality) => write_json(
                        stream,
                        200,
                        "OK",
                        &AnalysisResponse::Blocked {
                            quality: Box::new(quality),
                        },
                    ),
                    Err(error) => write_json(
                        stream,
                        422,
                        "Unprocessable Content",
                        &ErrorResponse::new(&format!("could not validate the event log: {error}")),
                    ),
                }
            }
            Err(error) => write_json(
                stream,
                422,
                "Unprocessable Content",
                &ErrorResponse::new(&format!("could not analyze the event log: {error}")),
            ),
        }
    }

    fn attribute_choices(
        &self,
        stream: &mut TcpStream,
        request: Request,
    ) -> Result<(), ServerError> {
        let command: AttributeChoicesRequest = match parse_json_request(stream, &request) {
            Ok(command) => command,
            Err(error) => return write_request_error(stream, error, "field choices"),
        };
        let log = match self.view_log(
            command.upload_id,
            command.mapping,
            command.policy,
            command.rules,
            command.filter,
        ) {
            Ok(log) => log,
            Err(error) => return write_comparison_load_error(stream, error),
        };
        match pm_query::attribute_choices(&log, &command.attribute, &command.search, 100) {
            Ok(choices) => write_json(stream, 200, "OK", &choices),
            Err(message) => write_json(
                stream,
                422,
                "Unprocessable Content",
                &ErrorResponse::new(&message),
            ),
        }
    }

    fn explore_linked(&self, stream: &mut TcpStream, request: Request) -> Result<(), ServerError> {
        let command: LinkedExploreCommand = match parse_json_request(stream, &request) {
            Ok(command) => command,
            Err(error) => return write_request_error(stream, error, "linked interval"),
        };
        let roots = match self.view_log(
            command.case_view.upload_id,
            command.case_view.mapping,
            command.case_view.policy,
            command.case_view.rules,
            command.case_view.filter,
        ) {
            Ok(log) => log,
            Err(error) => return write_comparison_load_error(stream, error),
        };
        let stored = self
            .ocel_store()
            .lock()
            .map_err(|_| ServerError::Io(std::io::Error::other("upload store unavailable")))?;
        let Some(upload) = stored
            .as_ref()
            .filter(|upload| upload.id == command.ocel_upload_id)
        else {
            return write_json(
                stream,
                404,
                "Not Found",
                &ErrorResponse::new("connected data is no longer available"),
            );
        };
        match pm_query::linked_interval::explore_linked(
            &upload.path,
            &command.filter,
            &roots,
            &command.interval,
        ) {
            Ok(report) => write_json(stream, 200, "OK", &report),
            Err(error) => write_json(
                stream,
                422,
                "Unprocessable Content",
                &ErrorResponse::new(&error),
            ),
        }
    }

    fn explore(&self, stream: &mut TcpStream, request: Request) -> Result<(), ServerError> {
        let command: ExploreCommand = match parse_json_request(stream, &request) {
            Ok(command) => command,
            Err(error) => return write_request_error(stream, error, "analysis"),
        };
        let log = match self.view_log(
            command.view.upload_id,
            command.view.mapping,
            command.view.policy,
            command.view.rules,
            command.view.filter,
        ) {
            Ok(log) => log,
            Err(error) => return write_comparison_load_error(stream, error),
        };
        match pm_query::exploration::explore(&log, &command.exploration) {
            Ok(report) => write_json(stream, 200, "OK", &report),
            Err(error) => write_json(
                stream,
                422,
                "Unprocessable Content",
                &ErrorResponse::new(&error),
            ),
        }
    }

    fn comparison_options(
        &self,
        stream: &mut TcpStream,
        request: Request,
    ) -> Result<(), ServerError> {
        let command: ComparisonOptionsRequest = match parse_json_request(stream, &request) {
            Ok(command) => command,
            Err(error) => return write_request_error(stream, error, "comparison options"),
        };
        let log = match self.view_log(
            command.upload_id,
            command.mapping,
            command.policy,
            command.rules,
            command.filter,
        ) {
            Ok(log) => log,
            Err(error) => return write_comparison_load_error(stream, error),
        };
        let attributes = log
            .attribute_names
            .iter()
            .map(|name| ComparisonAttribute {
                name: name.clone(),
                values: log
                    .attribute_options(name)
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
            })
            .collect();
        let first_events = log
            .traces
            .iter()
            .filter_map(|trace| trace.events.first().map(|event| event.at));
        let mut started_at_min = None;
        let mut started_at_max = None;
        for timestamp in first_events {
            started_at_min =
                Some(started_at_min.map_or(timestamp, |value: i64| value.min(timestamp)));
            started_at_max =
                Some(started_at_max.map_or(timestamp, |value: i64| value.max(timestamp)));
        }
        write_json(
            stream,
            200,
            "OK",
            &ComparisonOptionsResponse {
                attributes,
                started_at_min,
                started_at_max,
            },
        )
    }

    fn compare_cohorts(&self, stream: &mut TcpStream, request: Request) -> Result<(), ServerError> {
        let command: CompareCohortsRequest = match parse_json_request(stream, &request) {
            Ok(command) => command,
            Err(error) => return write_request_error(stream, error, "cohort comparison"),
        };
        let log = match self.load_comparison_log(
            &command.upload_id,
            command.mapping,
            &command.policy,
            command.rules,
            command.filter,
        ) {
            Ok(log) => log,
            Err(error) => return write_comparison_load_error(stream, error),
        };
        match pm_query::compare_cohorts(&log, &command.attribute, &command.left, &command.right) {
            Ok(comparison) => write_json(stream, 200, "OK", &comparison),
            Err(error) => write_json(
                stream,
                422,
                "Unprocessable Content",
                &ErrorResponse::new(&format!("could not compare cohorts: {error}")),
            ),
        }
    }

    /// Totals, averages and medians of a number field, overall or by group.
    fn values(&self, stream: &mut TcpStream, request: Request) -> Result<(), ServerError> {
        let command: ValuesRequest = match parse_json_request(stream, &request) {
            Ok(command) => command,
            Err(error) => return write_request_error(stream, error, "value summary"),
        };
        let finishing = command
            .rules
            .observation_window
            .as_ref()
            .map(|window| window.end_activities.clone())
            .unwrap_or_default();
        let log = match self.load_comparison_log(
            &command.upload_id,
            command.mapping,
            &command.policy,
            command.rules,
            command.filter,
        ) {
            Ok(log) => log,
            Err(error) => return write_comparison_load_error(stream, error),
        };
        match pm_query::summarize_values(
            &log,
            &command.field,
            &command.grouping,
            &finishing,
            command.limit.clamp(1, 200),
        ) {
            Ok(summary) => write_json(stream, 200, "OK", &summary),
            Err(error) => write_json(
                stream,
                422,
                "Unprocessable Content",
                &ErrorResponse::new(&error.to_string()),
            ),
        }
    }

    /// One page of items, sorted and searched by the engine.
    fn items(&self, stream: &mut TcpStream, request: Request) -> Result<(), ServerError> {
        let command: ItemsRequest = match parse_json_request(stream, &request) {
            Ok(command) => command,
            Err(error) => return write_request_error(stream, error, "item list"),
        };
        let log = match self.view_log(
            command.upload_id,
            command.mapping,
            command.policy,
            command.rules,
            command.filter,
        ) {
            Ok(log) => log,
            Err(error) => return write_comparison_load_error(stream, error),
        };
        let page = pm_query::item_page(
            &log,
            command.order,
            &command.search,
            command.offset,
            command.limit.clamp(1, 200),
        );
        write_json(stream, 200, "OK", &page)
    }

    /// The filtered log for a view, kept for the next request on the same view.
    fn view_log(
        &self,
        upload_id: String,
        mapping: pm_ingest::Mapping,
        policy: pm_ingest::Policy,
        rules: pm_ingest::Rules,
        filter: pm_query::Filter,
    ) -> Result<Arc<pm_core::EventLog>, ComparisonLoadError> {
        let key = serde_json::to_string(&(&upload_id, &mapping, &policy, &rules, &filter))
            .unwrap_or_default();
        let cached = self.map_log.lock().ok().and_then(|cache| {
            cache
                .as_ref()
                .filter(|(k, _)| *k == key)
                .map(|(_, log)| Arc::clone(log))
        });
        if let Some(log) = cached {
            return Ok(log);
        }
        let log = Arc::new(self.load_comparison_log(&upload_id, mapping, &policy, rules, filter)?);
        if let Ok(mut cache) = self.map_log.lock() {
            *cache = Some((key, Arc::clone(&log)));
        }
        Ok(log)
    }

    /// The process map, as JSON for the browser or, with `bpmn`, as a BPMN 2.0
    /// file of exactly the map shown, for modelling tools.
    fn map_view(
        &self,
        stream: &mut TcpStream,
        request: Request,
        bpmn: bool,
    ) -> Result<(), ServerError> {
        let command: MapViewRequest = match parse_json_request(stream, &request) {
            Ok(command) => command,
            Err(error) => return write_request_error(stream, error, "process map"),
        };
        let key = serde_json::to_string(&(
            &command.upload_id,
            &command.mapping,
            &command.policy,
            &command.rules,
            &command.filter,
        ))
        .unwrap_or_default();
        let cached = self.map_log.lock().ok().and_then(|cache| {
            cache
                .as_ref()
                .filter(|(k, _)| *k == key)
                .map(|(_, log)| Arc::clone(log))
        });
        let log = match cached {
            Some(log) => log,
            None => match self.load_comparison_log(
                &command.upload_id,
                command.mapping,
                &command.policy,
                command.rules,
                command.filter,
            ) {
                Ok(log) => {
                    let log = Arc::new(log);
                    if let Ok(mut cache) = self.map_log.lock() {
                        *cache = Some((key, Arc::clone(&log)));
                    }
                    log
                }
                Err(error) => return write_comparison_load_error(stream, error),
            },
        };
        match pm_query::map_view(&log, &command.selection) {
            Ok(view) if bpmn => {
                let name = command
                    .name
                    .as_deref()
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or("Process map");
                write_response(
                    stream,
                    200,
                    "OK",
                    "application/xml; charset=utf-8",
                    pm_query::map_view_bpmn(&view, name).as_bytes(),
                )
            }
            Ok(view) => write_json(stream, 200, "OK", &view),
            Err(error) => write_json(
                stream,
                422,
                "Unprocessable Content",
                &ErrorResponse::new(&format!("could not draw the process map: {error}")),
            ),
        }
    }

    fn compare_periods(&self, stream: &mut TcpStream, request: Request) -> Result<(), ServerError> {
        let command: ComparePeriodsRequest = match parse_json_request(stream, &request) {
            Ok(command) => command,
            Err(error) => return write_request_error(stream, error, "period comparison"),
        };
        let log = match self.load_comparison_log(
            &command.upload_id,
            command.mapping,
            &command.policy,
            command.rules,
            command.filter,
        ) {
            Ok(log) => log,
            Err(error) => return write_comparison_load_error(stream, error),
        };
        match pm_query::compare_periods(
            &log,
            command.baseline_start,
            command.baseline_end,
            command.comparison_start,
            command.comparison_end,
        ) {
            Ok(comparison) => write_json(stream, 200, "OK", &comparison),
            Err(error) => write_json(
                stream,
                422,
                "Unprocessable Content",
                &ErrorResponse::new(&format!("could not compare periods: {error}")),
            ),
        }
    }

    fn load_comparison_log(
        &self,
        upload_id: &str,
        mapping: pm_ingest::Mapping,
        policy: &pm_ingest::Policy,
        rules: pm_ingest::Rules,
        filter: pm_query::Filter,
    ) -> Result<pm_core::EventLog, ComparisonLoadError> {
        let path = {
            let stored = self
                .upload
                .lock()
                .map_err(|_| ComparisonLoadError::StoreUnavailable)?;
            stored
                .as_ref()
                .filter(|upload| upload.id == upload_id)
                .map(|upload| upload.path.clone())
                .ok_or(ComparisonLoadError::UploadMissing)?
        };
        match pm_query::load_log(path, Some(mapping), policy, rules) {
            Ok(Ok(loaded)) if filter.is_empty() => Ok(loaded.log),
            Ok(Ok(loaded)) => Ok(filter.apply(&loaded.log).log.into_owned()),
            Ok(Err(_)) => Err(ComparisonLoadError::QualityBlocked),
            Err(error) => Err(ComparisonLoadError::Load(error.to_string())),
        }
    }

    fn discover_upload(&self, stream: &mut TcpStream, request: Request) -> Result<(), ServerError> {
        let body = match request.json_body(stream) {
            Ok(body) => body,
            Err((status, reason, message)) => {
                return write_json(stream, status, reason, &ErrorResponse::new(message));
            }
        };
        let command: DiscoverRequest = match serde_json::from_slice(&body) {
            Ok(command) => command,
            Err(_) => {
                return write_json(
                    stream,
                    400,
                    "Bad Request",
                    &ErrorResponse::new("the discovery request is not valid JSON"),
                );
            }
        };
        let stored = match self.upload.lock() {
            Ok(stored) => stored,
            Err(_) => {
                return write_json(
                    stream,
                    500,
                    "Internal Server Error",
                    &ErrorResponse::new("the upload store is unavailable"),
                );
            }
        };
        let Some(upload) = stored
            .as_ref()
            .filter(|upload| upload.id == command.upload_id)
        else {
            return write_json(
                stream,
                404,
                "Not Found",
                &ErrorResponse::new("the uploaded file is no longer available; choose it again"),
            );
        };
        let loaded = match pm_query::load_log(
            &upload.path,
            Some(command.mapping.clone()),
            &command.policy,
            command.rules,
        ) {
            Ok(Ok(loaded)) => loaded,
            Ok(Err(_)) => {
                return match pm_query::assess_quality(
                    &upload.path,
                    Some(command.mapping),
                    &command.policy,
                ) {
                    Ok(quality) => write_json(
                        stream,
                        200,
                        "OK",
                        &DiscoveryResponse::Blocked {
                            quality: Box::new(quality),
                        },
                    ),
                    Err(error) => write_json(
                        stream,
                        422,
                        "Unprocessable Content",
                        &ErrorResponse::new(&format!("could not validate the event log: {error}")),
                    ),
                };
            }
            Err(error) => {
                return write_json(
                    stream,
                    422,
                    "Unprocessable Content",
                    &ErrorResponse::new(&format!("could not load the event log: {error}")),
                );
            }
        };
        match pm_query::discover_formal(
            &upload.file_name,
            &loaded.log,
            &command.filter,
            pm_core::InductiveMinerConfig {
                noise_threshold_milli: command.noise_threshold_milli,
            },
        ) {
            Ok(discovery) => write_json(
                stream,
                200,
                "OK",
                &DiscoveryResponse::Complete {
                    discovery: Box::new(discovery),
                },
            ),
            Err(error) => write_json(
                stream,
                422,
                "Unprocessable Content",
                &ErrorResponse::new(&format!("could not discover a formal model: {error}")),
            ),
        }
    }

    fn case_model_upload(
        &self,
        stream: &mut TcpStream,
        request: Request,
    ) -> Result<(), ServerError> {
        let Some(length) = request.content_length else {
            return write_json(
                stream,
                411,
                "Length Required",
                &ErrorResponse::new("the model file length is required"),
            );
        };
        if length == 0 {
            return write_json(
                stream,
                400,
                "Bad Request",
                &ErrorResponse::new("the selected model file is empty"),
            );
        }
        if length > MAX_CASE_MODEL_BYTES {
            return write_json(
                stream,
                413,
                "Content Too Large",
                &ErrorResponse::new("the case-centric model exceeds the 10 MiB safety limit"),
            );
        }
        let Some(encoded_name) = request.file_name.as_deref() else {
            return write_json(
                stream,
                400,
                "Bad Request",
                &ErrorResponse::new("the model file name is required"),
            );
        };
        let Some(file_name) = decode_file_name(encoded_name) else {
            return write_json(
                stream,
                400,
                "Bad Request",
                &ErrorResponse::new("the model file name is not valid"),
            );
        };
        let lower_name = file_name.to_ascii_lowercase();
        if !lower_name.ends_with(".model.json") || lower_name.ends_with(".ocelmodel.json") {
            return write_json(
                stream,
                415,
                "Unsupported Media Type",
                &ErrorResponse::new("choose a portable case-centric .model.json baseline"),
            );
        }
        let mut temporary = match TemporaryUpload::create(".model.json") {
            Ok(upload) => upload,
            Err(error) => {
                return write_json(
                    stream,
                    500,
                    "Internal Server Error",
                    &ErrorResponse::new(&format!("could not prepare the model upload: {error}")),
                );
            }
        };
        temporary.file_name.clone_from(&file_name);
        stream.set_read_timeout(Some(UPLOAD_READ_TIMEOUT))?;
        if let Err(error) = temporary.write_request(stream, length, &request.body_prefix) {
            return write_json(
                stream,
                400,
                "Bad Request",
                &ErrorResponse::new(&format!("could not receive the model upload: {error}")),
            );
        }
        let bytes = match fs::read(&temporary.path) {
            Ok(bytes) => bytes,
            Err(error) => {
                return write_json(
                    stream,
                    500,
                    "Internal Server Error",
                    &ErrorResponse::new(&format!("could not read the uploaded model: {error}")),
                );
            }
        };
        let model = match pm_query::parse_formal_model_json(&bytes) {
            Ok(model) => model,
            Err(error) => {
                return write_json(
                    stream,
                    422,
                    "Unprocessable Content",
                    &ErrorResponse::new(&format!(
                        "the selected file is not a portable executable process model: {error}"
                    )),
                );
            }
        };
        if let Err(error) = pm_query::validate_formal_model(&model.net) {
            return write_json(
                stream,
                422,
                "Unprocessable Content",
                &ErrorResponse::new(&format!(
                    "the selected process model is not executable: {error}"
                )),
            );
        }
        let model_items = model
            .net
            .place_count
            .saturating_add(model.net.transitions.len());
        if !(1..=10_000).contains(&model_items) {
            return write_json(
                stream,
                422,
                "Unprocessable Content",
                &ErrorResponse::new(
                    "the model must contain between 1 and 10,000 places and transitions",
                ),
            );
        }
        let training_source = std::path::Path::new(&model.source)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let response = CaseModelUploadResponse {
            model_id: temporary.id.clone(),
            file_name,
            places: model.net.place_count,
            transitions: model.net.transitions.len(),
            training_source,
        };
        let live_model = pm_mcp::LiveProcessModel {
            path: temporary.path.clone(),
            source_name: response.file_name.clone(),
        };
        match self.case_model.lock() {
            Ok(mut stored) => *stored = Some(temporary),
            Err(_) => {
                return write_json(
                    stream,
                    500,
                    "Internal Server Error",
                    &ErrorResponse::new("the model upload store is unavailable"),
                );
            }
        }
        match self.live_workspace.lock() {
            Ok(mut workspace) => workspace.case_model = Some(live_model),
            Err(_) => {
                return write_json(
                    stream,
                    500,
                    "Internal Server Error",
                    &ErrorResponse::new("the live analysis workspace is unavailable"),
                );
            }
        }
        write_json(stream, 200, "OK", &response)
    }

    fn case_conformance(
        &self,
        stream: &mut TcpStream,
        request: Request,
    ) -> Result<(), ServerError> {
        let body = match request.json_body(stream) {
            Ok(body) => body,
            Err(error) => return write_request_error(stream, error, "process conformance"),
        };
        let command: CaseConformanceCommand = match serde_json::from_slice(&body) {
            Ok(command) => command,
            Err(_) => {
                return write_json(
                    stream,
                    400,
                    "Bad Request",
                    &ErrorResponse::new("the process conformance request is not valid JSON"),
                );
            }
        };
        if !(1..=200).contains(&command.detail_limit) {
            return write_json(
                stream,
                400,
                "Bad Request",
                &ErrorResponse::new("the conformance detail limit must be between 1 and 200"),
            );
        }
        let (log_path, log_name) = {
            let stored = match self.upload.lock() {
                Ok(stored) => stored,
                Err(_) => {
                    return write_json(
                        stream,
                        500,
                        "Internal Server Error",
                        &ErrorResponse::new("the upload store is unavailable"),
                    );
                }
            };
            let Some(upload) = stored
                .as_ref()
                .filter(|upload| upload.id == command.upload_id)
            else {
                return write_json(
                    stream,
                    404,
                    "Not Found",
                    &ErrorResponse::new("the uploaded log is no longer available; choose it again"),
                );
            };
            (upload.path.clone(), upload.file_name.clone())
        };
        let (model_path, model_name) = {
            let stored = match self.case_model.lock() {
                Ok(stored) => stored,
                Err(_) => {
                    return write_json(
                        stream,
                        500,
                        "Internal Server Error",
                        &ErrorResponse::new("the model upload store is unavailable"),
                    );
                }
            };
            let Some(model) = stored.as_ref().filter(|model| model.id == command.model_id) else {
                return write_json(
                    stream,
                    404,
                    "Not Found",
                    &ErrorResponse::new(
                        "the uploaded baseline is no longer available; choose it again",
                    ),
                );
            };
            (model.path.clone(), model.file_name.clone())
        };
        let loaded = match pm_query::load_log(
            &log_path,
            Some(command.mapping),
            &command.policy,
            command.rules,
        ) {
            Ok(Ok(loaded)) => loaded,
            Ok(Err(_)) => {
                return write_json(
                    stream,
                    422,
                    "Unprocessable Content",
                    &ErrorResponse::new(
                        "resolve the event log's data-quality decisions before checking conformance",
                    ),
                );
            }
            Err(error) => {
                return write_json(
                    stream,
                    422,
                    "Unprocessable Content",
                    &ErrorResponse::new(&format!("could not load the event log: {error}")),
                );
            }
        };
        let model_bytes = match fs::read(&model_path) {
            Ok(bytes) => bytes,
            Err(error) => {
                return write_json(
                    stream,
                    500,
                    "Internal Server Error",
                    &ErrorResponse::new(&format!("could not read the selected model: {error}")),
                );
            }
        };
        let model = match pm_query::parse_formal_model_json(&model_bytes) {
            Ok(model) => model,
            Err(error) => {
                return write_json(
                    stream,
                    422,
                    "Unprocessable Content",
                    &ErrorResponse::new(&format!("the selected model cannot be used: {error}")),
                );
            }
        };
        let training_source = std::path::Path::new(&model.source)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let expected_process = model.source.clone();
        let expected_steps = pm_query::expected_steps(&model_bytes, &model.net);
        let model_saved_at_ms = fs::metadata(&model_path)
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|elapsed| elapsed.as_millis() as u64);
        // Token replay guesses around optional steps and can flag compliant
        // cases, so a model with them is always checked exactly.
        let algorithm = if pm_query::has_silent_steps(&model.net) {
            CaseConformanceAlgorithm::Alignment
        } else {
            command.algorithm
        };
        match algorithm {
            CaseConformanceAlgorithm::Token => {
                let mut report = match pm_query::check_conformance(
                    &log_name,
                    &model_name,
                    &loaded.log,
                    &command.filter,
                    &model.net,
                    command.detail_limit,
                ) {
                    Ok(report) => report,
                    Err(error) => {
                        return write_json(
                            stream,
                            422,
                            "Unprocessable Content",
                            &ErrorResponse::new(&format!(
                                "could not check process conformance: {error}"
                            )),
                        );
                    }
                };
                report.source = log_name;
                report.model_source = model_name;
                write_json(
                    stream,
                    200,
                    "OK",
                    &CaseConformanceResponse {
                        report,
                        model_training_source: training_source,
                        expected_process,
                        expected_steps,
                        model_saved_at_ms,
                    },
                )
            }
            CaseConformanceAlgorithm::Alignment => {
                let mut report = match pm_query::align_conformance(
                    &log_name,
                    &model_name,
                    &loaded.log,
                    &command.filter,
                    &model.net,
                    command.detail_limit,
                ) {
                    Ok(report) => report,
                    Err(error) => {
                        return write_json(
                            stream,
                            422,
                            "Unprocessable Content",
                            &ErrorResponse::new(&format!(
                                "could not align process conformance: {error}"
                            )),
                        );
                    }
                };
                report.source = log_name;
                report.model_source = model_name;
                write_json(
                    stream,
                    200,
                    "OK",
                    &CaseConformanceResponse {
                        report,
                        model_training_source: training_source,
                        expected_process,
                        expected_steps,
                        model_saved_at_ms,
                    },
                )
            }
        }
    }

    fn case_detail(&self, stream: &mut TcpStream, request: Request) -> Result<(), ServerError> {
        let body = match request.json_body(stream) {
            Ok(body) => body,
            Err((status, reason, message)) => {
                return write_json(stream, status, reason, &ErrorResponse::new(message));
            }
        };
        let command: CaseRequest = match serde_json::from_slice(&body) {
            Ok(command) => command,
            Err(_) => {
                return write_json(
                    stream,
                    400,
                    "Bad Request",
                    &ErrorResponse::new("the case request is not valid JSON"),
                );
            }
        };
        let stored = match self.upload.lock() {
            Ok(stored) => stored,
            Err(_) => {
                return write_json(
                    stream,
                    500,
                    "Internal Server Error",
                    &ErrorResponse::new("the upload store is unavailable"),
                );
            }
        };
        let Some(upload) = stored
            .as_ref()
            .filter(|upload| upload.id == command.upload_id)
        else {
            return write_json(
                stream,
                404,
                "Not Found",
                &ErrorResponse::new("the uploaded file is no longer available; choose it again"),
            );
        };
        match pm_query::load_log(
            &upload.path,
            Some(command.mapping),
            &command.policy,
            command.rules,
        ) {
            Ok(Ok(loaded)) => match pm_query::inspect_case_read_as(
                &loaded.log,
                command.case_id.trim(),
                loaded.timestamp_reading,
            ) {
                Some(case) => write_json(stream, 200, "OK", &case),
                None => write_json(
                    stream,
                    404,
                    "Not Found",
                    &ErrorResponse::new("no case has that exact identifier"),
                ),
            },
            Ok(Err(_)) => write_json(
                stream,
                409,
                "Conflict",
                &ErrorResponse::new("data-quality decisions must be answered before opening cases"),
            ),
            Err(error) => write_json(
                stream,
                422,
                "Unprocessable Content",
                &ErrorResponse::new(&format!("could not open the case: {error}")),
            ),
        }
    }

    fn open_project(&self, stream: &mut TcpStream, request: Request) -> Result<(), ServerError> {
        let body = match request.json_body(stream) {
            Ok(body) => body,
            Err((status, reason, message)) => {
                return write_json(stream, status, reason, &ErrorResponse::new(message));
            }
        };
        let decoded_label = request.file_name.as_deref().and_then(decode_file_name);
        let label = decoded_label.as_deref().unwrap_or("selected project");
        let text = match std::str::from_utf8(&body) {
            Ok(text) => text,
            Err(_) => {
                return write_json(
                    stream,
                    400,
                    "Bad Request",
                    &ErrorResponse::new("the selected project is not UTF-8 JSON"),
                );
            }
        };
        match pm_project::Project::from_json(text, label) {
            Ok(project) => {
                let engine_changed = project.engine_changed();
                write_json(
                    stream,
                    200,
                    "OK",
                    &OpenProjectResponse {
                        project,
                        engine_changed,
                    },
                )
            }
            Err(error) => write_json(
                stream,
                422,
                "Unprocessable Content",
                &ErrorResponse::new(&error.to_string()),
            ),
        }
    }

    fn save_project(&self, stream: &mut TcpStream, request: Request) -> Result<(), ServerError> {
        let body = match request.json_body(stream) {
            Ok(body) => body,
            Err((status, reason, message)) => {
                return write_json(stream, status, reason, &ErrorResponse::new(message));
            }
        };
        let command: SaveProjectRequest = match serde_json::from_slice(&body) {
            Ok(command) => command,
            Err(_) => {
                return write_json(
                    stream,
                    400,
                    "Bad Request",
                    &ErrorResponse::new("the save-project request is not valid JSON"),
                );
            }
        };
        let stored = match self.upload.lock() {
            Ok(stored) => stored,
            Err(_) => {
                return write_json(
                    stream,
                    500,
                    "Internal Server Error",
                    &ErrorResponse::new("the upload store is unavailable"),
                );
            }
        };
        let Some(upload) = stored
            .as_ref()
            .filter(|upload| upload.id == command.upload_id)
        else {
            return write_json(
                stream,
                404,
                "Not Found",
                &ErrorResponse::new("the uploaded file is no longer available; choose it again"),
            );
        };
        let mut project = project_from_upload(
            upload,
            command.mapping,
            command.policy,
            command.rules,
            command.filter,
            command.name.trim().to_owned(),
        );
        let live_project = project.clone();
        project.source.path.clone_from(&upload.file_name);
        if !self.set_live_case_project(live_project, upload.file_name.clone()) {
            return write_json(
                stream,
                500,
                "Internal Server Error",
                &ErrorResponse::new("the live analysis workspace is unavailable"),
            );
        }
        write_json(stream, 200, "OK", &project)
    }

    fn reopen_project(&self, stream: &mut TcpStream, request: Request) -> Result<(), ServerError> {
        let body = match request.json_body(stream) {
            Ok(body) => body,
            Err((status, reason, message)) => {
                return write_json(stream, status, reason, &ErrorResponse::new(message));
            }
        };
        let command: ReopenProjectRequest = match serde_json::from_slice(&body) {
            Ok(command) => command,
            Err(_) => {
                return write_json(
                    stream,
                    400,
                    "Bad Request",
                    &ErrorResponse::new("the reopen-project request is not valid JSON"),
                );
            }
        };
        if let Err(error) = command.project.validate() {
            return write_json(
                stream,
                422,
                "Unprocessable Content",
                &ErrorResponse::new(&error.to_string()),
            );
        }
        let stored = match self.upload.lock() {
            Ok(stored) => stored,
            Err(_) => {
                return write_json(
                    stream,
                    500,
                    "Internal Server Error",
                    &ErrorResponse::new("the upload store is unavailable"),
                );
            }
        };
        let Some(upload) = stored
            .as_ref()
            .filter(|upload| upload.id == command.upload_id)
        else {
            return write_json(
                stream,
                404,
                "Not Found",
                &ErrorResponse::new("the uploaded file is no longer available; choose it again"),
            );
        };
        let source_state = command.project.check_source(&upload.path);
        if !source_state.reproduces() {
            return write_json(
                stream,
                409,
                "Conflict",
                &ErrorResponse::new(&source_state.explain()),
            );
        }
        let mut live_project = command.project.clone();
        live_project.source.path = upload.path.display().to_string();
        match pm_query::analyze_filtered(
            &upload.path,
            Some(command.project.mapping.clone()),
            &command.project.policy,
            command.project.rules.clone(),
            command.project.view_filter.clone(),
        ) {
            Ok(pm_query::Outcome::Complete(mut analysis)) => {
                analysis.source.clone_from(&upload.file_name);
                if !self.set_live_case_project(live_project, upload.file_name.clone()) {
                    return write_json(
                        stream,
                        500,
                        "Internal Server Error",
                        &ErrorResponse::new("the live analysis workspace is unavailable"),
                    );
                }
                write_json(stream, 200, "OK", &AnalysisResponse::Complete { analysis })
            }
            Ok(pm_query::Outcome::Blocked(_)) => {
                if !self.set_live_case_project(live_project, upload.file_name.clone()) {
                    return write_json(
                        stream,
                        500,
                        "Internal Server Error",
                        &ErrorResponse::new("the live analysis workspace is unavailable"),
                    );
                }
                match pm_query::assess_quality(
                    &upload.path,
                    Some(command.project.mapping),
                    &command.project.policy,
                ) {
                    Ok(quality) => write_json(
                        stream,
                        200,
                        "OK",
                        &AnalysisResponse::Blocked {
                            quality: Box::new(quality),
                        },
                    ),
                    Err(error) => write_json(
                        stream,
                        422,
                        "Unprocessable Content",
                        &ErrorResponse::new(&format!("could not validate the event log: {error}")),
                    ),
                }
            }
            Err(error) => write_json(
                stream,
                422,
                "Unprocessable Content",
                &ErrorResponse::new(&format!("could not reopen the project: {error}")),
            ),
        }
    }
}

type RequestError = (u16, &'static str, &'static str);

fn parse_json_request<T: DeserializeOwned>(
    stream: &mut TcpStream,
    request: &Request,
) -> Result<T, RequestError> {
    let body = request.json_body(stream)?;
    serde_json::from_slice(&body).map_err(|_| (400, "Bad Request", "request is not valid JSON"))
}

fn write_request_error(
    stream: &mut TcpStream,
    error: RequestError,
    request_name: &str,
) -> Result<(), ServerError> {
    let (status, reason, message) = error;
    let detail = if status == 400 {
        format!("the {request_name} request is not valid JSON")
    } else {
        message.to_owned()
    };
    write_json(stream, status, reason, &ErrorResponse::new(&detail))
}

enum ComparisonLoadError {
    StoreUnavailable,
    UploadMissing,
    QualityBlocked,
    Load(String),
}

fn write_comparison_load_error(
    stream: &mut TcpStream,
    error: ComparisonLoadError,
) -> Result<(), ServerError> {
    match error {
        ComparisonLoadError::StoreUnavailable => write_json(
            stream,
            500,
            "Internal Server Error",
            &ErrorResponse::new("the upload store is unavailable"),
        ),
        ComparisonLoadError::UploadMissing => write_json(
            stream,
            404,
            "Not Found",
            &ErrorResponse::new("the uploaded file is no longer available; choose it again"),
        ),
        ComparisonLoadError::QualityBlocked => write_json(
            stream,
            409,
            "Conflict",
            &ErrorResponse::new("data-quality decisions must be answered before comparing cases"),
        ),
        ComparisonLoadError::Load(error) => write_json(
            stream,
            422,
            "Unprocessable Content",
            &ErrorResponse::new(&format!(
                "could not load the event log for comparison: {error}"
            )),
        ),
    }
}

struct Request {
    method: String,
    target: String,
    host: Option<String>,
    origin: Option<String>,
    /// The browser's own statement of where a request comes from.
    fetch_site: Option<String>,
    authorization: Option<String>,
    content_length: Option<u64>,
    file_name: Option<String>,
    transfer_encoding: bool,
    body_prefix: Vec<u8>,
}

impl Request {
    fn read(stream: &mut TcpStream) -> std::io::Result<Option<Self>> {
        let mut bytes = Vec::with_capacity(1024);
        let mut buffer = [0u8; 1024];
        while !bytes.windows(4).any(|window| window == b"\r\n\r\n") {
            let count = stream.read(&mut buffer)?;
            if count == 0 || bytes.len() + count > MAX_REQUEST_HEAD {
                return Ok(None);
            }
            bytes.extend_from_slice(&buffer[..count]);
        }
        let Some(head_end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") else {
            return Ok(None);
        };
        let body_prefix = bytes[(head_end + 4)..].to_vec();
        let Ok(text) = std::str::from_utf8(&bytes[..head_end]) else {
            return Ok(None);
        };
        let mut lines = text.split("\r\n");
        let Some(start_line) = lines.next() else {
            return Ok(None);
        };
        let mut start = start_line.split_whitespace();
        let Some(method) = start.next() else {
            return Ok(None);
        };
        let Some(target) = start.next() else {
            return Ok(None);
        };
        let Some(version) = start.next() else {
            return Ok(None);
        };
        if version != "HTTP/1.1" || start.next().is_some() {
            return Ok(None);
        }

        let mut request = Self {
            method: method.to_owned(),
            target: target.to_owned(),
            host: None,
            origin: None,
            fetch_site: None,
            authorization: None,
            content_length: None,
            file_name: None,
            transfer_encoding: false,
            body_prefix,
        };
        for line in lines.take_while(|line| !line.is_empty()) {
            let Some((name, value)) = line.split_once(':') else {
                return Ok(None);
            };
            match name.trim().to_ascii_lowercase().as_str() {
                "host" => request.host = Some(value.trim().to_owned()),
                "origin" => request.origin = Some(value.trim().to_owned()),
                "sec-fetch-site" => request.fetch_site = Some(value.trim().to_owned()),
                "authorization" => request.authorization = Some(value.trim().to_owned()),
                "content-length" => {
                    if request.content_length.is_some() {
                        return Ok(None);
                    }
                    let Ok(length) = value.trim().parse() else {
                        return Ok(None);
                    };
                    request.content_length = Some(length);
                }
                "x-file-name" => request.file_name = Some(value.trim().to_owned()),
                "transfer-encoding" => request.transfer_encoding = true,
                _ => {}
            }
        }
        if request.content_length.is_some() && request.transfer_encoding {
            return Ok(None);
        }
        Ok(Some(request))
    }

    fn authorized(&self, expected: &str) -> bool {
        self.authorization
            .as_deref()
            .and_then(|value| value.strip_prefix("Bearer "))
            .is_some_and(|provided| constant_time_eq(provided.as_bytes(), expected.as_bytes()))
    }

    fn json_body(
        &self,
        stream: &mut TcpStream,
    ) -> Result<Vec<u8>, (u16, &'static str, &'static str)> {
        let Some(length) = self.content_length else {
            return Err((411, "Length Required", "request length is required"));
        };
        if length > MAX_JSON_BODY {
            return Err((413, "Content Too Large", "the JSON request is too large"));
        }
        if self.body_prefix.len() as u64 > length {
            return Err((400, "Bad Request", "request exceeds its declared length"));
        }
        let mut body = self.body_prefix.clone();
        let remaining = length - body.len() as u64;
        let mut limited = stream.take(remaining);
        if limited.read_to_end(&mut body).is_err() || body.len() as u64 != length {
            return Err((400, "Bad Request", "request body ended early"));
        }
        Ok(body)
    }
}

#[derive(Serialize)]
struct ErrorResponse<'a> {
    error: &'a str,
}

impl<'a> ErrorResponse<'a> {
    fn new(error: &'a str) -> Self {
        Self { error }
    }
}

#[derive(Serialize)]
struct ProfileResponse {
    upload_id: String,
    file_name: String,
    bytes: u64,
    rows_sampled: usize,
    columns: Vec<ColumnResponse>,
    proposed_mapping: Option<pm_ingest::Mapping>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OcelAnalysisCommand {
    upload_id: String,
    #[serde(default)]
    filter: pm_core::OcelFilter,
}

#[derive(Serialize)]
struct OcelAnalysisResponse {
    upload_id: String,
    analysis: pm_query::OcelAnalysis,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OcelObjectCommand {
    upload_id: String,
    object: String,
    #[serde(default)]
    filter: pm_core::OcelFilter,
}

#[derive(Serialize)]
struct OcelObjectResponse {
    upload_id: String,
    lifecycle: pm_query::LabelledOcelLifecycle,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LinkedExploreCommand {
    ocel_upload_id: String,
    filter: pm_core::OcelFilter,
    case_view: ComparisonOptionsRequest,
    interval: pm_query::linked_interval::LinkedInterval,
}

#[derive(Deserialize)]
struct ExploreCommand {
    #[serde(flatten)]
    view: ComparisonOptionsRequest,
    exploration: pm_query::exploration::ExplorationRequest,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OcelTimeBetweenCommand {
    upload_id: String,
    from_activity: String,
    to_activity: String,
    #[serde(default)]
    filter: pm_core::OcelFilter,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OcelSynchronizationCommand {
    upload_id: String,
    #[serde(default)]
    filter: pm_core::OcelFilter,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OcelBottleneckCommand {
    upload_id: String,
    #[serde(default)]
    filter: pm_core::OcelFilter,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OcelProjectionCommand {
    upload_id: String,
    case_object_type: String,
    #[serde(default)]
    filter: pm_core::OcelFilter,
    case_limit: usize,
    event_limit_per_case: usize,
}

fn default_variable_arc_threshold_milli() -> u16 {
    1_000
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OcelDiscoveryCommand {
    upload_id: String,
    #[serde(default)]
    filter: pm_core::OcelFilter,
    #[serde(default = "default_noise_threshold_milli")]
    noise_threshold_milli: u16,
    #[serde(default = "default_variable_arc_threshold_milli")]
    variable_arc_threshold_milli: u16,
}

#[derive(Serialize)]
struct OcelModelUploadResponse {
    model_id: String,
    file_name: String,
    places: usize,
    transitions: usize,
    object_types: usize,
}

fn default_ocel_conformance_detail_limit() -> usize {
    100
}

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum OcelConformanceAlgorithm {
    #[default]
    Token,
    Alignment,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OcelConformanceCommand {
    upload_id: String,
    model_id: String,
    #[serde(default)]
    filter: pm_core::OcelFilter,
    #[serde(default = "default_ocel_conformance_detail_limit")]
    detail_limit: usize,
    #[serde(default)]
    algorithm: OcelConformanceAlgorithm,
}

fn sanitize_ocel_model_sources(
    source: &mut String,
    model_source: &mut String,
    training_source: &mut String,
    log_name: String,
    model_name: String,
) {
    *source = log_name;
    *model_source = model_name;
    *training_source = std::path::Path::new(training_source)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct AnalyzeRequest {
    upload_id: String,
    mapping: pm_ingest::Mapping,
    #[serde(default)]
    policy: pm_ingest::Policy,
    #[serde(default)]
    rules: pm_ingest::Rules,
    #[serde(default)]
    filter: pm_query::Filter,
}

fn project_from_upload(
    upload: &TemporaryUpload,
    mapping: pm_ingest::Mapping,
    policy: pm_ingest::Policy,
    rules: pm_ingest::Rules,
    filter: pm_query::Filter,
    name: String,
) -> pm_project::Project {
    pm_project::Project {
        format_version: pm_project::FORMAT_VERSION,
        engine_version: pm_project::ENGINE_VERSION.to_owned(),
        name,
        source: upload.source(),
        mapping,
        policy,
        rules,
        view_filter: filter,
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ComparisonOptionsRequest {
    upload_id: String,
    mapping: pm_ingest::Mapping,
    #[serde(default)]
    policy: pm_ingest::Policy,
    #[serde(default)]
    rules: pm_ingest::Rules,
    #[serde(default)]
    filter: pm_query::Filter,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AttributeChoicesRequest {
    upload_id: String,
    mapping: pm_ingest::Mapping,
    #[serde(default)]
    policy: pm_ingest::Policy,
    #[serde(default)]
    rules: pm_ingest::Rules,
    #[serde(default)]
    filter: pm_query::Filter,
    attribute: String,
    #[serde(default)]
    search: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MapViewRequest {
    upload_id: String,
    mapping: pm_ingest::Mapping,
    #[serde(default)]
    policy: pm_ingest::Policy,
    #[serde(default)]
    rules: pm_ingest::Rules,
    #[serde(default)]
    filter: pm_query::Filter,
    #[serde(default)]
    selection: pm_query::MapSelection,
    /// The process name written into a BPMN export.
    #[serde(default)]
    name: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ItemsRequest {
    upload_id: String,
    mapping: pm_ingest::Mapping,
    #[serde(default)]
    policy: pm_ingest::Policy,
    #[serde(default)]
    rules: pm_ingest::Rules,
    #[serde(default)]
    filter: pm_query::Filter,
    #[serde(default)]
    order: pm_query::ItemOrder,
    #[serde(default)]
    search: String,
    #[serde(default)]
    offset: usize,
    #[serde(default = "default_item_page")]
    limit: usize,
}

fn default_item_page() -> usize {
    50
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CompareCohortsRequest {
    upload_id: String,
    mapping: pm_ingest::Mapping,
    #[serde(default)]
    policy: pm_ingest::Policy,
    #[serde(default)]
    rules: pm_ingest::Rules,
    #[serde(default)]
    filter: pm_query::Filter,
    attribute: String,
    left: String,
    right: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ValuesRequest {
    upload_id: String,
    mapping: pm_ingest::Mapping,
    #[serde(default)]
    policy: pm_ingest::Policy,
    #[serde(default)]
    rules: pm_ingest::Rules,
    #[serde(default)]
    filter: pm_query::Filter,
    field: String,
    #[serde(default)]
    grouping: pm_query::ValueGrouping,
    #[serde(default = "default_value_groups")]
    limit: usize,
}

fn default_value_groups() -> usize {
    10
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ComparePeriodsRequest {
    upload_id: String,
    mapping: pm_ingest::Mapping,
    #[serde(default)]
    policy: pm_ingest::Policy,
    #[serde(default)]
    rules: pm_ingest::Rules,
    #[serde(default)]
    filter: pm_query::Filter,
    baseline_start: i64,
    baseline_end: i64,
    comparison_start: i64,
    comparison_end: i64,
}

#[derive(Serialize)]
struct ComparisonOptionsResponse {
    attributes: Vec<ComparisonAttribute>,
    started_at_min: Option<i64>,
    started_at_max: Option<i64>,
}

#[derive(Serialize)]
struct ComparisonAttribute {
    name: String,
    values: Vec<String>,
}

fn default_noise_threshold_milli() -> u16 {
    200
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DiscoverRequest {
    upload_id: String,
    mapping: pm_ingest::Mapping,
    #[serde(default)]
    policy: pm_ingest::Policy,
    #[serde(default)]
    rules: pm_ingest::Rules,
    #[serde(default)]
    filter: pm_query::Filter,
    #[serde(default = "default_noise_threshold_milli")]
    noise_threshold_milli: u16,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CaseRequest {
    upload_id: String,
    mapping: pm_ingest::Mapping,
    #[serde(default)]
    policy: pm_ingest::Policy,
    #[serde(default)]
    rules: pm_ingest::Rules,
    case_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SaveProjectRequest {
    upload_id: String,
    mapping: pm_ingest::Mapping,
    #[serde(default)]
    policy: pm_ingest::Policy,
    #[serde(default)]
    rules: pm_ingest::Rules,
    #[serde(default)]
    filter: pm_query::Filter,
    #[serde(default)]
    name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReopenProjectRequest {
    upload_id: String,
    project: pm_project::Project,
}

#[derive(Serialize)]
struct OpenProjectResponse {
    project: pm_project::Project,
    engine_changed: bool,
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum AnalysisResponse {
    Complete {
        analysis: Box<pm_query::Analysis>,
    },
    Blocked {
        quality: Box<pm_query::DataQualityReport>,
    },
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum DiscoveryResponse {
    Complete {
        discovery: Box<pm_query::FormalDiscovery>,
    },
    Blocked {
        quality: Box<pm_query::DataQualityReport>,
    },
}

#[derive(Serialize)]
struct CaseModelUploadResponse {
    model_id: String,
    file_name: String,
    places: usize,
    transitions: usize,
    training_source: String,
}

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum CaseConformanceAlgorithm {
    #[default]
    Token,
    Alignment,
}

fn default_case_conformance_detail_limit() -> usize {
    20
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CaseConformanceCommand {
    upload_id: String,
    model_id: String,
    mapping: pm_ingest::Mapping,
    #[serde(default)]
    policy: pm_ingest::Policy,
    #[serde(default)]
    rules: pm_ingest::Rules,
    #[serde(default)]
    filter: pm_query::Filter,
    #[serde(default = "default_case_conformance_detail_limit")]
    detail_limit: usize,
    #[serde(default)]
    algorithm: CaseConformanceAlgorithm,
}

#[derive(Serialize)]
struct CaseConformanceResponse<T> {
    #[serde(flatten)]
    report: T,
    model_training_source: String,
    /// The expected process as its author named it, e.g. "Claims SOP v2.1".
    expected_process: String,
    /// Its steps in order, for drawing it as a line of steps.
    expected_steps: Vec<pm_query::ExpectedStep>,
    /// When this expected process was first saved, in Unix milliseconds:
    /// the page says when the SOP was set, and that the check is re-run now.
    model_saved_at_ms: Option<u64>,
}

#[derive(Serialize)]
struct ColumnResponse {
    name: String,
    kind: String,
    sampled: usize,
    blank: usize,
    blank_percent: f64,
    distinct: usize,
    examples: Vec<String>,
    suggested_roles: Vec<String>,
}

impl From<&pm_ingest::Column> for ColumnResponse {
    fn from(column: &pm_ingest::Column) -> Self {
        Self {
            name: column.name.clone(),
            kind: column.kind.describe(),
            sampled: column.sampled,
            blank: column.blank,
            blank_percent: column.blank_percent(),
            distinct: column.distinct,
            examples: column.examples.clone(),
            suggested_roles: column
                .suggests
                .iter()
                .map(|role| role.label().to_owned())
                .collect(),
        }
    }
}

struct TemporaryUpload {
    id: String,
    file_name: String,
    path: PathBuf,
    file: Option<File>,
    bytes: u64,
    sha256: String,
    delete_on_drop: bool,
}

impl TemporaryUpload {
    fn create(suffix: &str) -> std::io::Result<Self> {
        for _ in 0..8 {
            let mut entropy = [0u8; 16];
            OsRng.fill_bytes(&mut entropy);
            let id = hex(&entropy);
            let path = std::env::temp_dir().join(format!("open-process-{id}{suffix}"));
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            match options.open(&path) {
                Ok(file) => {
                    return Ok(Self {
                        id,
                        file_name: String::new(),
                        path,
                        file: Some(file),
                        bytes: 0,
                        sha256: String::new(),
                        delete_on_drop: true,
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "could not allocate a unique temporary file",
        ))
    }

    fn write_request(
        &mut self,
        stream: &mut TcpStream,
        length: u64,
        prefix: &[u8],
    ) -> std::io::Result<()> {
        if prefix.len() as u64 > length {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "request body exceeds Content-Length",
            ));
        }
        let file = self.file.as_mut().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::BrokenPipe, "temporary upload is closed")
        })?;
        let mut hasher = Sha256::new();
        file.write_all(prefix)?;
        hasher.update(prefix);
        let mut remaining = length - prefix.len() as u64;
        let mut buffer = [0u8; 64 * 1024];
        while remaining > 0 {
            let wanted =
                usize::try_from(remaining.min(buffer.len() as u64)).unwrap_or(buffer.len());
            let read = stream.read(&mut buffer[..wanted])?;
            if read == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "request body ended early",
                ));
            }
            file.write_all(&buffer[..read])?;
            hasher.update(&buffer[..read]);
            remaining -= read as u64;
        }
        file.flush()?;
        self.bytes = length;
        self.sha256 = hex(&hasher.finalize());
        Ok(())
    }

    fn source(&self) -> pm_project::Source {
        pm_project::Source {
            path: self.path.display().to_string(),
            bytes: self.bytes,
            sha256: self.sha256.clone(),
        }
    }
}

impl Drop for TemporaryUpload {
    fn drop(&mut self) {
        drop(self.file.take());
        if self.delete_on_drop {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn supported_suffix(file_name: &str) -> Option<&'static str> {
    let lower = file_name.to_ascii_lowercase();
    [".xes.gz", ".parquet", ".csv", ".xes"]
        .into_iter()
        .find(|suffix| lower.ends_with(suffix))
}

fn decode_file_name(encoded: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(encoded.len());
    let source = encoded.as_bytes();
    let mut index = 0;
    while index < source.len() {
        if source[index] == b'%' {
            let pair = source.get(index + 1..index + 3)?;
            let text = std::str::from_utf8(pair).ok()?;
            bytes.push(u8::from_str_radix(text, 16).ok()?);
            index += 3;
        } else {
            bytes.push(source[index]);
            index += 1;
        }
    }
    let decoded = String::from_utf8(bytes).ok()?;
    let name = decoded.rsplit(['/', '\\']).next()?;
    if name.is_empty() || name.len() > 255 || name.chars().any(char::is_control) {
        None
    } else {
        Some(name.to_owned())
    }
}

fn write_json(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    value: &impl Serialize,
) -> Result<(), ServerError> {
    let body = serde_json::to_vec(value).map_err(std::io::Error::other)?;
    write_response(stream, status, reason, "application/json", &body)
}

fn write_response(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    content_type: &str,
    body: &[u8],
) -> Result<(), ServerError> {
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nContent-Security-Policy: default-src 'self'; frame-ancestors 'none'; base-uri 'none'\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)?;
    stream.flush()?;
    close_gracefully(stream);
    Ok(())
}

/// Ends the reply, then reads and discards whatever the client still sends
/// (an unread request body) before the socket closes. Closing with unread
/// input makes the system send a reset, which can make the client lose a
/// reply it has not yet read.
fn close_gracefully(stream: &mut TcpStream) {
    let _ = stream.shutdown(std::net::Shutdown::Write);
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(50)));
    let mut sink = [0u8; 8192];
    let mut drained = 0usize;
    while drained < 64 * 1024 * 1024 {
        match stream.read(&mut sink) {
            Ok(0) | Err(_) => break,
            Ok(count) => drained += count,
        }
    }
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0u8, |difference, (left, right)| difference | (left ^ right))
        == 0
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(DIGITS[(byte >> 4) as usize] as char);
        encoded.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    encoded
}

// Browser-native TypeScript (a type-inferred subset that is also valid JS)
// keeps the first UI slice build-free. Rust embeds every UI file directly,
// so the installed application remains one executable with no Node runtime.
const INDEX: &[u8] = include_bytes!("../../../ui/index.html");
const STYLES: &[u8] = include_bytes!("../../../ui/src/app.css");
macro_rules! script {
    ($name:literal) => {
        (
            concat!("/assets/", $name, ".js"),
            include_bytes!(concat!("../../../ui/src/", $name, ".ts")),
        )
    };
}

// Classic scripts sharing one global scope, in the order ui/index.html loads them.
const SCRIPTS: &[(&str, &[u8])] = &[
    script!("core"),
    script!("navigation"),
    script!("projects"),
    script!("agent-setup"),
    script!("extracts"),
    script!("import"),
    script!("quality"),
    script!("filters"),
    script!("answer"),
    script!("process-model"),
    script!("map"),
    script!("objects"),
    script!("follow"),
    script!("provenance"),
    script!("values"),
    script!("cases"),
    script!("timing"),
    script!("results"),
    script!("explorers"),
    script!("studio"),
    script!("map-analysis"),
    script!("subway"),
    script!("map-shell"),
    script!("compare"),
    script!("conformance"),
    script!("slides"),
    script!("select"),
    script!("controls"),
    script!("main"),
];

macro_rules! font {
    ($name:literal) => {
        (
            concat!("/assets/fonts/", $name, ".woff2"),
            include_bytes!(concat!("../../../ui/fonts/", $name, ".woff2")),
        )
    };
}

// Geist and Geist Mono by Vercel (SIL Open Font License 1.1, see ui/fonts/OFL.txt).
const FONTS: &[(&str, &[u8])] = &[
    font!("Geist-Regular"),
    font!("Geist-Medium"),
    font!("Geist-SemiBold"),
    font!("GeistMono-Regular"),
    font!("GeistMono-Medium"),
];

fn font_asset(path: &str) -> Option<&'static [u8]> {
    FONTS
        .iter()
        .find(|(served, _)| *served == path)
        .map(|(_, bytes)| *bytes)
}

fn script_asset(path: &str) -> Option<&'static [u8]> {
    SCRIPTS
        .iter()
        .find(|(served, _)| *served == path)
        .map(|(_, bytes)| *bytes)
}
const FAVICON: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><rect width="24" height="24" rx="6.5" fill="#0a0a0a"/><path d="M4.6 12.6 9.7 6.3l2.6 3.1 2-2.1 5.1 5.3z" fill="#ffffff"/><path d="M4.6 14.4 9.7 18.6l2.6-2.1 2 1.4 5.1-3.5z" fill="#38bdf8"/></svg>"##;

impl Drop for Server {
    fn drop(&mut self) {
        if self.workspace_hub_owner
            && let Some(hub) = &self.workspace_hub
        {
            hub.shutdown.store(true, Ordering::Release);
            hub.stop_pages();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(server: &Server, request: &str) -> String {
        request_bytes(server, request.as_bytes())
    }

    fn request_bytes(server: &Server, request: &[u8]) -> String {
        let mut stream = TcpStream::connect(server.address()).unwrap();
        stream.write_all(request).unwrap();
        let (accepted, _peer) = server.listener.accept().unwrap();
        server.handle(accepted).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    }

    fn get(server: &Server, path: &str, headers: &str) -> String {
        request(
            server,
            &format!(
                "GET {path} HTTP/1.1\r\nHost: {}\r\n{headers}\r\n",
                server.address()
            ),
        )
    }

    #[test]
    fn binds_to_loopback_on_an_ephemeral_port() {
        let server = Server::bind(0).unwrap();
        assert!(server.address().ip().is_loopback());
        assert_ne!(server.address().port(), 0);
    }

    #[test]
    fn creates_a_fresh_256_bit_token_per_launch() {
        let first = Server::bind(0).unwrap();
        let second = Server::bind(0).unwrap();
        assert_eq!(first.token().len(), 64);
        assert_ne!(first.token(), second.token());
        assert!(first.launch_url().contains("/#token="));
    }

    #[test]
    fn health_requires_the_launch_token() {
        let server = Server::bind(0).unwrap();
        assert!(get(&server, "/api/health", "").starts_with("HTTP/1.1 401"));
        let response = get(
            &server,
            "/api/health",
            &format!("Authorization: Bearer {}\r\n", server.token()),
        );
        assert!(response.starts_with("HTTP/1.1 200"));
        let health: serde_json::Value =
            serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(health["status"], "ok");
        assert_eq!(health["project_folders"], false);
    }

    #[test]
    fn its_own_page_needs_no_key_but_other_sites_and_programs_do() {
        let server = Server::bind(0).unwrap();
        let own = format!("Origin: http://{}\r\n", server.address());
        assert!(get(&server, "/api/health", &own).starts_with("HTTP/1.1 200"));
        assert!(
            get(&server, "/api/health", "Sec-Fetch-Site: same-origin\r\n")
                .starts_with("HTTP/1.1 200")
        );
        // Another website, even claiming to be same-site, is refused.
        assert!(
            get(
                &server,
                "/api/health",
                "Origin: https://news.example\r\nSec-Fetch-Site: same-origin\r\n"
            )
            .starts_with("HTTP/1.1 403")
        );
        assert!(
            get(&server, "/api/health", "Sec-Fetch-Site: cross-site\r\n")
                .starts_with("HTTP/1.1 401")
        );
        // A program with no browser headers still needs the key.
        assert!(get(&server, "/api/health", "").starts_with("HTTP/1.1 401"));
    }

    #[test]
    fn rejects_cross_origin_and_host_header_requests() {
        let server = Server::bind(0).unwrap();
        let token = format!("Authorization: Bearer {}\r\n", server.token());
        assert!(
            get(
                &server,
                "/api/health",
                &format!("Origin: https://evil.example\r\n{token}")
            )
            .starts_with("HTTP/1.1 403")
        );
        let response = request(
            &server,
            &format!(
                "GET /api/health HTTP/1.1\r\nHost: evil.example\r\nAuthorization: Bearer {}\r\n\r\n",
                server.token()
            ),
        );
        assert!(response.starts_with("HTTP/1.1 403"));
    }

    #[test]
    fn serves_only_a_small_bounded_surface() {
        let server = Server::bind(0).unwrap();
        let index = get(&server, "/", "");
        assert!(index.starts_with("HTTP/1.1 200"));
        assert!(index.contains("glasstarn"));
        assert!(index.contains("brand-mark"));
        assert!(index.contains("ocel-process-map"));
        assert!(index.contains("ocel-flow-limit"));
        assert!(index.contains("filter-starts-with"));
        assert!(index.contains("filter-direct-pairs"));
        assert!(index.contains("filter-eventual-to"));
        assert!(index.contains("does not prove that one activity caused another"));
        assert!(index.contains("choose-case-baseline"));
        assert!(index.contains("check-case-conformance"));
        assert!(index.contains("not proof of regulatory compliance"));
        let styles = get(&server, "/assets/app.css", "");
        assert!(styles.starts_with("HTTP/1.1 200"));
        assert!(styles.contains("text/css"));
        assert!(styles.contains(".case-conformance-controls"));
        let index = get(&server, "/", "");
        let mut app = String::new();
        for (path, _) in SCRIPTS {
            assert!(index.contains(&format!("<script src=\"{path}\" defer></script>")));
            let script = get(&server, path, "");
            assert!(script.starts_with("HTTP/1.1 200"));
            assert!(script.contains("text/javascript"));
            app.push_str(&script);
        }
        assert!(get(&server, "/assets/app.js", "").starts_with("HTTP/1.1 404"));
        assert!(app.contains("sessionStorage"));
        assert!(app.contains("function renderOcelFlowMap()"));
        assert!(app.contains("function tracePairsFromControls"));
        assert!(app.contains("directly_follows: tracePairsFromControls"));
        assert!(app.contains("eventually_follows: tracePairsFromControls"));
        assert!(app.contains("available.slice(0, limit)"));
        assert!(app.contains("layout does not change the analysis"));
        assert!(app.contains("function uploadCaseBaseline"));
        assert!(app.contains("function renderCaseConformance"));
        assert!(app.contains("function checkCaseConformance"));
        assert!(get(&server, "/etc/passwd", "").starts_with("HTTP/1.1 404"));
        let response = request(
            &server,
            &format!("POST / HTTP/1.1\r\nHost: {}\r\n\r\n", server.address()),
        );
        assert!(response.starts_with("HTTP/1.1 405"));
    }

    #[test]
    fn serves_every_embedded_font_used_by_the_stylesheet() {
        let server = Server::bind(0).unwrap();
        let styles = get(&server, "/assets/app.css", "");
        for (path, bytes) in FONTS {
            assert!(styles.contains(&format!("url(\"{path}\")")));
            let mut stream = TcpStream::connect(server.address()).unwrap();
            write!(
                stream,
                "GET {path} HTTP/1.1\r\nHost: {}\r\n\r\n",
                server.address()
            )
            .unwrap();
            let (accepted, _peer) = server.listener.accept().unwrap();
            server.handle(accepted).unwrap();
            let mut response = Vec::new();
            stream.read_to_end(&mut response).unwrap();
            let split = response.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
            let head = String::from_utf8_lossy(&response[..split]);
            assert!(head.starts_with("HTTP/1.1 200"));
            assert!(head.contains("Content-Type: font/woff2"));
            assert_eq!(&response[split + 4..], *bytes);
            assert!(bytes.starts_with(b"wOF2"));
        }
        assert!(get(&server, "/assets/fonts/missing.woff2", "").starts_with("HTTP/1.1 404"));
    }

    #[test]
    fn streams_an_authenticated_upload_into_the_real_profiler() {
        let server = Server::bind(0).unwrap();
        let body = concat!(
            "case_id,activity,timestamp,resource\n",
            "A-1,Receive,2026-08-09T09:00:00Z,Ana\n",
            "A-1,Approve,2026-08-09T10:00:00Z,Bo\n",
            "A-2,Receive,2026-08-09T11:00:00Z,Ana\n"
        );
        let response = request(
            &server,
            &format!(
                "POST /api/profile HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nX-File-Name: order%20log.csv\r\nContent-Length: {}\r\n\r\n{body}",
                server.address(),
                server.token(),
                body.len()
            ),
        );
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        let payload = response.split("\r\n\r\n").nth(1).unwrap();
        let value: serde_json::Value = serde_json::from_str(payload).unwrap();
        assert_eq!(value["file_name"], "order log.csv");
        assert_eq!(value["rows_sampled"], 3);
        assert_eq!(value["columns"].as_array().unwrap().len(), 4);
        assert_eq!(value["proposed_mapping"]["case_id"], "case_id");
        assert_eq!(value["proposed_mapping"]["activity"], "activity");
        assert_eq!(value["proposed_mapping"]["timestamp"], "timestamp");
        assert_eq!(value["proposed_mapping"]["resource"], "resource");
        let upload_id = value["upload_id"].clone();

        let command = serde_json::json!({
            "upload_id": upload_id.clone(),
            "mapping": value["proposed_mapping"]
        })
        .to_string();
        let analyzed = request(
            &server,
            &format!(
                "POST /api/analyze HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{command}",
                server.address(),
                server.token(),
                command.len()
            ),
        );
        assert!(analyzed.starts_with("HTTP/1.1 200"), "{analyzed}");
        let payload = analyzed.split("\r\n\r\n").nth(1).unwrap();
        let value: serde_json::Value = serde_json::from_str(payload).unwrap();
        assert_eq!(value["status"], "complete");
        assert_eq!(value["analysis"]["source"], "order log.csv");
        assert_eq!(value["analysis"]["summary"]["events"], 3);
        assert_eq!(value["analysis"]["summary"]["cases"], 2);
        assert_eq!(value["analysis"]["nodes"].as_array().unwrap().len(), 2);
        assert_eq!(value["analysis"]["edges"].as_array().unwrap().len(), 1);

        let discovery_command = serde_json::json!({
            "upload_id": upload_id.clone(),
            "mapping": value["analysis"]["mapping"],
            "noise_threshold_milli": 125
        })
        .to_string();
        let discovered = request(
            &server,
            &format!(
                "POST /api/discover HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{discovery_command}",
                server.address(),
                server.token(),
                discovery_command.len()
            ),
        );
        assert!(discovered.starts_with("HTTP/1.1 200"), "{discovered}");
        let payload = discovered.split("\r\n\r\n").nth(1).unwrap();
        let discovery: serde_json::Value = serde_json::from_str(payload).unwrap();
        assert_eq!(discovery["status"], "complete");
        assert_eq!(discovery["discovery"]["source"], "order log.csv");
        assert_eq!(
            discovery["discovery"]["algorithm"],
            "inductive_miner_infrequent"
        );
        assert_eq!(discovery["discovery"]["noise_threshold_milli"], 125);
        assert_eq!(discovery["discovery"]["tree"]["kind"], "sequence");
        assert_eq!(discovery["discovery"]["view"]["cases_shown"], 2);

        let filtered_command = serde_json::json!({
            "upload_id": upload_id.clone(),
            "mapping": value["analysis"]["mapping"],
            "filter": { "min_events": 2 }
        })
        .to_string();
        let filtered = request(
            &server,
            &format!(
                "POST /api/analyze HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{filtered_command}",
                server.address(),
                server.token(),
                filtered_command.len()
            ),
        );
        let payload = filtered.split("\r\n\r\n").nth(1).unwrap();
        let filtered: serde_json::Value = serde_json::from_str(payload).unwrap();
        assert_eq!(filtered["analysis"]["view"]["cases_shown"], 1);
        assert_eq!(filtered["analysis"]["view"]["cases_total"], 2);
        assert_eq!(filtered["analysis"]["summary"]["events"], 2);

        let save_command = serde_json::json!({
            "upload_id": upload_id.clone(),
            "mapping": value["analysis"]["mapping"],
            "filter": { "min_events": 2 },
            "name": "Order review"
        })
        .to_string();
        let saved = request(
            &server,
            &format!(
                "POST /api/project/save HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{save_command}",
                server.address(),
                server.token(),
                save_command.len()
            ),
        );
        assert!(saved.starts_with("HTTP/1.1 200"), "{saved}");
        let project_json = saved.split("\r\n\r\n").nth(1).unwrap();
        let project: serde_json::Value = serde_json::from_str(project_json).unwrap();
        assert_eq!(project["name"], "Order review");
        assert_eq!(project["source"]["path"], "order log.csv");
        assert_eq!(project["source"]["bytes"], body.len() as u64);
        assert_eq!(project["view_filter"]["min_events"], 2);

        let opened = request(
            &server,
            &format!(
                "POST /api/project/open HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nX-File-Name: order.open-process.json\r\nContent-Length: {}\r\n\r\n{project_json}",
                server.address(),
                server.token(),
                project_json.len()
            ),
        );
        assert!(opened.starts_with("HTTP/1.1 200"), "{opened}");
        let opened: serde_json::Value =
            serde_json::from_str(opened.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(opened["project"], project);
        assert_eq!(opened["engine_changed"], false);

        let reopen_command = serde_json::json!({
            "upload_id": upload_id.clone(),
            "project": project
        })
        .to_string();
        let reopened = request(
            &server,
            &format!(
                "POST /api/project/reopen HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{reopen_command}",
                server.address(),
                server.token(),
                reopen_command.len()
            ),
        );
        assert!(reopened.starts_with("HTTP/1.1 200"), "{reopened}");
        let reopened: serde_json::Value =
            serde_json::from_str(reopened.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(reopened["status"], "complete");
        assert_eq!(reopened["analysis"]["summary"]["events"], 2);
        assert_eq!(reopened["analysis"]["view"]["cases_shown"], 1);

        let case_command = serde_json::json!({
            "upload_id": upload_id,
            "mapping": value["analysis"]["mapping"],
            "case_id": "A-1"
        })
        .to_string();
        let case = request(
            &server,
            &format!(
                "POST /api/case HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{case_command}",
                server.address(),
                server.token(),
                case_command.len()
            ),
        );
        assert!(case.starts_with("HTTP/1.1 200"), "{case}");
        let payload = case.split("\r\n\r\n").nth(1).unwrap();
        let case: serde_json::Value = serde_json::from_str(payload).unwrap();
        assert_eq!(case["case"], "A-1");
        assert_eq!(case["events"].as_array().unwrap().len(), 2);
        assert_eq!(case["events"][0]["source_row"], 2);
        assert_eq!(case["events"][1]["source_row"], 3);

        let changed_path = server.upload.lock().unwrap().as_ref().unwrap().path.clone();
        std::fs::write(
            changed_path,
            b"case_id,activity,timestamp\nchanged,A,2026-01-01T00:00:00Z\n",
        )
        .unwrap();
        let changed = request(
            &server,
            &format!(
                "POST /api/project/reopen HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{reopen_command}",
                server.address(),
                server.token(),
                reopen_command.len()
            ),
        );
        assert!(changed.starts_with("HTTP/1.1 409"), "{changed}");
        assert!(changed.contains("source file has changed"));
    }

    #[test]
    fn browser_case_conformance_uses_the_active_view_and_shares_its_baseline_with_live_mcp() {
        let policy = pm_mcp::ExposurePolicy::safe();
        let agent_token = policy.delegation_token();
        let server = Server::bind_with_agent(0, Some(policy)).unwrap();
        let body = concat!(
            "case_id,activity,timestamp\n",
            "C1,A,2026-08-09T09:00:00Z\n",
            "C1,B,2026-08-09T10:00:00Z\n",
            "C1,C,2026-08-09T11:00:00Z\n",
            "C2,A,2026-08-10T09:00:00Z\n",
            "C2,C,2026-08-10T10:00:00Z\n",
            "C3,A,2026-08-11T09:00:00Z\n",
            "C3,Unexpected,2026-08-11T10:00:00Z\n",
            "C3,B,2026-08-11T11:00:00Z\n",
            "C3,C,2026-08-11T12:00:00Z\n"
        );
        let profiled = request(
            &server,
            &format!(
                "POST /api/profile HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nX-File-Name: current.csv\r\nContent-Length: {}\r\n\r\n{body}",
                server.address(),
                server.token(),
                body.len()
            ),
        );
        assert!(profiled.starts_with("HTTP/1.1 200"), "{profiled}");
        let profile: serde_json::Value =
            serde_json::from_str(profiled.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        let filter = serde_json::json!({ "min_events": 3 });
        let analyze = serde_json::json!({
            "upload_id": profile["upload_id"],
            "mapping": profile["proposed_mapping"],
            "filter": filter
        })
        .to_string();
        let analyzed = request(
            &server,
            &format!(
                "POST /api/analyze HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{analyze}",
                server.address(),
                server.token(),
                analyze.len()
            ),
        );
        assert!(analyzed.starts_with("HTTP/1.1 200"), "{analyzed}");
        let baseline = pm_query::PortableFormalModel {
            source: "/private/approved.bpmn".to_owned(),
            net: pm_query::LabelledPetriNet {
                place_count: 4,
                transitions: vec![
                    pm_query::LabelledTransition {
                        id: 0,
                        activity: Some("A".to_owned()),
                        inputs: vec![0],
                        outputs: vec![1],
                    },
                    pm_query::LabelledTransition {
                        id: 1,
                        activity: Some("B".to_owned()),
                        inputs: vec![1],
                        outputs: vec![2],
                    },
                    pm_query::LabelledTransition {
                        id: 2,
                        activity: Some("C".to_owned()),
                        inputs: vec![2],
                        outputs: vec![3],
                    },
                ],
                initial_marking: vec![1, 0, 0, 0],
                final_marking: vec![0, 0, 0, 1],
            },
        };
        let model_body = serde_json::to_string(&baseline).unwrap();
        let uploaded_model = request(
            &server,
            &format!(
                "POST /api/model HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nX-File-Name: approved.model.json\r\nContent-Length: {}\r\n\r\n{model_body}",
                server.address(),
                server.token(),
                model_body.len()
            ),
        );
        assert!(
            uploaded_model.starts_with("HTTP/1.1 200"),
            "{uploaded_model}"
        );
        let model: serde_json::Value =
            serde_json::from_str(uploaded_model.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(model["file_name"], "approved.model.json");
        assert_eq!(model["training_source"], "approved.bpmn");
        assert_eq!(model["places"], 4);
        assert_eq!(model["transitions"], 3);

        for algorithm in ["token", "alignment"] {
            let command = serde_json::json!({
                "upload_id": profile["upload_id"],
                "model_id": model["model_id"],
                "mapping": profile["proposed_mapping"],
                "filter": filter,
                "detail_limit": 1,
                "algorithm": algorithm
            })
            .to_string();
            let conformance = request(
                &server,
                &format!(
                    "POST /api/conformance HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{command}",
                    server.address(),
                    server.token(),
                    command.len()
                ),
            );
            assert!(conformance.starts_with("HTTP/1.1 200"), "{conformance}");
            let conformance: serde_json::Value =
                serde_json::from_str(conformance.split("\r\n\r\n").nth(1).unwrap()).unwrap();
            assert_eq!(conformance["source"], "current.csv");
            assert_eq!(conformance["model_source"], "approved.model.json");
            assert_eq!(conformance["model_training_source"], "approved.bpmn");
            assert_eq!(conformance["view"]["cases_shown"], 2);
            assert_eq!(conformance["view"]["cases_total"], 3);
            assert_eq!(conformance["fit_cases"], 1);
            assert_eq!(conformance["deviating_cases"], 1);
            assert_eq!(conformance["deviations_shown"], 1);
            assert_eq!(conformance["cases"].as_array().unwrap().len(), 1);
            assert!(!conformance.to_string().contains("open-process-"));
        }

        let list_projects = serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": { "name": "list_projects", "arguments": {} }
        })
        .to_string();
        let listed_projects = request(
            &server,
            &format!(
                "POST /api/mcp HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {agent_token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{list_projects}",
                server.address(),
                list_projects.len()
            ),
        );
        let listed_projects: serde_json::Value =
            serde_json::from_str(listed_projects.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        let project_handle =
            listed_projects["result"]["structuredContent"]["projects"][0]["project"]
                .as_str()
                .unwrap();
        let list_models = serde_json::json!({
            "jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": { "name": "list_process_models", "arguments": {} }
        })
        .to_string();
        let listed_models = request(
            &server,
            &format!(
                "POST /api/mcp HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {agent_token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{list_models}",
                server.address(),
                list_models.len()
            ),
        );
        let listed_models: serde_json::Value =
            serde_json::from_str(listed_models.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(listed_models["result"]["structuredContent"]["count"], 1);
        let model_handle = listed_models["result"]["structuredContent"]["models"][0]["model"]
            .as_str()
            .unwrap();
        let check = serde_json::json!({
            "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": {
                "name": "check_process_conformance",
                "arguments": { "project": project_handle, "model": model_handle, "detail_limit": 1 }
            }
        })
        .to_string();
        let checked = request(
            &server,
            &format!(
                "POST /api/mcp HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {agent_token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{check}",
                server.address(),
                check.len()
            ),
        );
        let checked: serde_json::Value =
            serde_json::from_str(checked.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(checked["result"]["isError"], false);
        assert_eq!(
            checked["result"]["structuredContent"]["view"]["cases_shown"],
            2
        );
        assert_eq!(checked["result"]["structuredContent"]["deviating_cases"], 1);
        assert!(!checked.to_string().contains("Unexpected"));
        assert!(!checked.to_string().contains("approved.model.json"));

        // Rejected replacements must preserve the last accepted baseline for
        // both browser conformance and delegated MCP.
        for (name, body, status) in [
            ("broken.model.json", "{}", "422"),
            ("wrong.txt", model_body.as_str(), "415"),
            ("empty.model.json", "", "400"),
        ] {
            let rejected = request(
                &server,
                &format!(
                    "POST /api/model HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nX-File-Name: {name}\r\nContent-Length: {}\r\n\r\n{body}",
                    server.address(),
                    server.token(),
                    body.len()
                ),
            );
            assert!(
                rejected.starts_with(&format!("HTTP/1.1 {status}")),
                "{rejected}"
            );
            assert_eq!(
                server.case_model.lock().unwrap().as_ref().unwrap().id,
                model["model_id"].as_str().unwrap()
            );
            assert_eq!(
                server
                    .live_workspace
                    .lock()
                    .unwrap()
                    .case_model
                    .as_ref()
                    .unwrap()
                    .source_name,
                "approved.model.json"
            );
        }
        let replacement = request(
            &server,
            &format!(
                "POST /api/profile HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nX-File-Name: replacement.csv\r\nContent-Length: {}\r\n\r\n{body}",
                server.address(),
                server.token(),
                body.len()
            ),
        );
        assert!(replacement.starts_with("HTTP/1.1 200"), "{replacement}");
        assert!(server.case_model.lock().unwrap().is_none());
        assert!(server.live_workspace.lock().unwrap().case_model.is_none());
    }

    #[test]
    fn live_mcp_delegates_the_exact_active_browser_view_under_separate_auth() {
        let policy = pm_mcp::ExposurePolicy::safe();
        let agent_token = policy.delegation_token();
        let server = Server::bind_with_agent(0, Some(policy)).unwrap();
        let body = concat!(
            "case_id,activity,timestamp\n",
            "A-1,Receive,2026-08-09T09:00:00Z\n",
            "A-1,Approve,2026-08-09T10:00:00Z\n",
            "A-2,Receive,2026-08-09T11:00:00Z\n"
        );
        let profiled = request(
            &server,
            &format!(
                "POST /api/profile HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nX-File-Name: live.csv\r\nContent-Length: {}\r\n\r\n{body}",
                server.address(),
                server.token(),
                body.len()
            ),
        );
        let profile: serde_json::Value =
            serde_json::from_str(profiled.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        let analyze = serde_json::json!({
            "upload_id": profile["upload_id"],
            "mapping": profile["proposed_mapping"],
            "filter": { "min_events": 2 }
        })
        .to_string();
        let analyzed = request(
            &server,
            &format!(
                "POST /api/analyze HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{analyze}",
                server.address(),
                server.token(),
                analyze.len()
            ),
        );
        assert!(analyzed.starts_with("HTTP/1.1 200"), "{analyzed}");

        let list = serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": { "name": "list_projects", "arguments": {} }
        })
        .to_string();
        let browser_auth = request(
            &server,
            &format!(
                "POST /api/mcp HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{list}",
                server.address(),
                server.token(),
                list.len()
            ),
        );
        assert!(browser_auth.starts_with("HTTP/1.1 401"));
        let agent_cannot_use_browser_api = get(
            &server,
            "/api/health",
            &format!("Authorization: Bearer {agent_token}\r\n"),
        );
        assert!(agent_cannot_use_browser_api.starts_with("HTTP/1.1 401"));
        let listed = request(
            &server,
            &format!(
                "POST /api/mcp HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {agent_token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{list}",
                server.address(),
                list.len()
            ),
        );
        assert!(listed.starts_with("HTTP/1.1 200"), "{listed}");
        let listed: serde_json::Value =
            serde_json::from_str(listed.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        let projects = &listed["result"]["structuredContent"]["projects"];
        assert_eq!(
            listed["result"]["structuredContent"]["workspace"],
            "running_application"
        );
        assert_eq!(projects.as_array().unwrap().len(), 1);
        let project = projects[0]["project"].as_str().unwrap();

        let summary = serde_json::json!({
            "jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": { "name": "get_process_summary", "arguments": { "project": project } }
        })
        .to_string();
        let summarized = request(
            &server,
            &format!(
                "POST /api/mcp HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {agent_token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{summary}",
                server.address(),
                summary.len()
            ),
        );
        let summarized: serde_json::Value =
            serde_json::from_str(summarized.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        let output = &summarized["result"]["structuredContent"];
        assert_eq!(output["view"]["cases_shown"], 1);
        assert_eq!(output["view"]["cases_total"], 2);
        assert_eq!(output["summary"]["events"], 2);
    }

    #[test]
    fn object_centric_upload_filters_and_analyzes_without_flattening() {
        let server = Server::bind(0).unwrap();
        let body = include_str!("../../../testdata/ocel/order-management.jsonocel");
        let uploaded = request(
            &server,
            &format!(
                "POST /api/ocel HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nX-File-Name: order-management.jsonocel\r\nContent-Length: {}\r\n\r\n{body}",
                server.address(),
                server.token(),
                body.len()
            ),
        );
        assert!(uploaded.starts_with("HTTP/1.1 200"));
        let uploaded: serde_json::Value =
            serde_json::from_str(uploaded.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(uploaded["analysis"]["scope"]["events_shown"], 3);
        assert_eq!(uploaded["analysis"]["scope"]["objects_shown"], 4);
        assert_eq!(uploaded["analysis"]["graphs"]["activities"][0]["events"], 1);
        let command = serde_json::json!({
            "upload_id": uploaded["upload_id"],
            "filter": { "object_types": ["package"] }
        })
        .to_string();
        let filtered = request(
            &server,
            &format!(
                "POST /api/ocel/analyze HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{command}",
                server.address(),
                server.token(),
                command.len()
            ),
        );
        assert!(filtered.starts_with("HTTP/1.1 200"));
        let filtered: serde_json::Value =
            serde_json::from_str(filtered.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(filtered["analysis"]["scope"]["objects_shown"], 1);
        assert_eq!(filtered["analysis"]["scope"]["events_shown"], 2);
        assert_eq!(
            filtered["analysis"]["graphs"]["directly_follows"][0]["object_type"],
            "package"
        );
        assert_eq!(
            filtered["analysis"]["graphs"]["directly_follows"][0]["mean_elapsed_micros"],
            86_400_000_000_i64
        );
        assert_eq!(
            filtered["analysis"]["variants"].as_array().unwrap().len(),
            1
        );
        assert_eq!(
            filtered["analysis"]["variants"][0]["evidence_objects"][0],
            "package-1"
        );
        let lifecycle_command = serde_json::json!({
            "upload_id": uploaded["upload_id"],
            "object": "package-1",
            "filter": { "object_types": ["package"] }
        })
        .to_string();
        let lifecycle = request(
            &server,
            &format!(
                "POST /api/ocel/object HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{lifecycle_command}",
                server.address(),
                server.token(),
                lifecycle_command.len()
            ),
        );
        assert!(lifecycle.starts_with("HTTP/1.1 200"), "{lifecycle}");
        let lifecycle: serde_json::Value =
            serde_json::from_str(lifecycle.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(lifecycle["lifecycle"]["object"], "package-1");
        assert_eq!(
            lifecycle["lifecycle"]["events"].as_array().unwrap().len(),
            2
        );
        assert_eq!(lifecycle["lifecycle"]["events"][0]["event"], "e-pack");
        let timing_command = serde_json::json!({
            "upload_id": uploaded["upload_id"],
            "from_activity": "place order",
            "to_activity": "create package"
        })
        .to_string();
        let timing = request(
            &server,
            &format!(
                "POST /api/ocel/time-between HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{timing_command}",
                server.address(),
                server.token(),
                timing_command.len()
            ),
        );
        assert!(timing.starts_with("HTTP/1.1 200"), "{timing}");
        let timing: serde_json::Value =
            serde_json::from_str(timing.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(timing["measurements"].as_array().unwrap().len(), 1);
        assert_eq!(timing["measurements"][0]["object_type"], "item");
        assert_eq!(timing["measurements"][0]["objects"], 2);
        assert_eq!(
            timing["measurements"][0]["mean_elapsed_micros"],
            86_400_000_000_i64
        );
        let synchronization_command = serde_json::json!({
            "upload_id": uploaded["upload_id"]
        })
        .to_string();
        let synchronization = request(
            &server,
            &format!(
                "POST /api/ocel/synchronization HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{synchronization_command}",
                server.address(),
                server.token(),
                synchronization_command.len()
            ),
        );
        assert!(
            synchronization.starts_with("HTTP/1.1 200"),
            "{synchronization}"
        );
        let synchronization: serde_json::Value =
            serde_json::from_str(synchronization.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(synchronization["activities"].as_array().unwrap().len(), 2);
        let package = synchronization["activities"]
            .as_array()
            .unwrap()
            .iter()
            .find(|activity| activity["activity"] == "create package")
            .unwrap();
        assert_eq!(package["eligible_events"], 1);
        assert_eq!(package["first_seen_object_participations"], 1);
        assert_eq!(package["mean_arrival_spread_micros"], 0);
        assert_eq!(
            package["mean_elapsed_after_last_prior_micros"],
            86_400_000_000_i64
        );
        assert_eq!(package["evidence"][0]["event"], "e-pack");
        let bottleneck_command = serde_json::json!({
            "upload_id": uploaded["upload_id"]
        })
        .to_string();
        let bottlenecks = request(
            &server,
            &format!(
                "POST /api/ocel/bottlenecks HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{bottleneck_command}",
                server.address(),
                server.token(),
                bottleneck_command.len()
            ),
        );
        assert!(bottlenecks.starts_with("HTTP/1.1 200"), "{bottlenecks}");
        let bottlenecks: serde_json::Value =
            serde_json::from_str(bottlenecks.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(bottlenecks["lifecycle_candidates"], 2);
        assert_eq!(bottlenecks["lifecycle"][0]["rank"], 1);
        assert_eq!(bottlenecks["lifecycle"][0]["object_type"], "item");
        assert_eq!(
            bottlenecks["lifecycle"][0]["total_elapsed_micros"],
            172_800_000_000_i64
        );
        assert_eq!(bottlenecks["lifecycle"][0]["share_millionths"], 666_666);
        assert_eq!(bottlenecks["synchronization_eligible_activities"], 1);
        assert_eq!(bottlenecks["synchronization_positive_activities"], 0);
        assert!(
            bottlenecks["synchronization"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        let projection_command = serde_json::json!({
            "upload_id": uploaded["upload_id"],
            "case_object_type": "item",
            "case_limit": 1,
            "event_limit_per_case": 1
        })
        .to_string();
        let projection = request(
            &server,
            &format!(
                "POST /api/ocel/project HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{projection_command}",
                server.address(),
                server.token(),
                projection_command.len()
            ),
        );
        assert!(projection.starts_with("HTTP/1.1 200"), "{projection}");
        let projection: serde_json::Value =
            serde_json::from_str(projection.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(projection["case_object_type"], "item");
        assert_eq!(projection["source_events"], 3);
        assert_eq!(projection["projected_cases"], 2);
        assert_eq!(projection["projected_event_occurrences"], 4);
        assert_eq!(projection["convergence_events"], 2);
        assert_eq!(projection["convergence_additional_occurrences"], 2);
        assert_eq!(projection["cases_returned"], 1);
        assert_eq!(projection["cases"][0]["events_available"], 2);
        assert_eq!(projection["cases"][0]["events_returned"], 1);
        assert_eq!(projection["cases"][0]["truncated"], true);
        let discovery_command = serde_json::json!({
            "upload_id": uploaded["upload_id"],
            "noise_threshold_milli": 125,
            "variable_arc_threshold_milli": 900
        })
        .to_string();
        let discovery = request(
            &server,
            &format!(
                "POST /api/ocel/discover HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{discovery_command}",
                server.address(),
                server.token(),
                discovery_command.len()
            ),
        );
        assert!(discovery.starts_with("HTTP/1.1 200"), "{discovery}");
        let discovery: serde_json::Value =
            serde_json::from_str(discovery.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(discovery["object_types"].as_array().unwrap().len(), 3);
        assert!(
            discovery["places"]
                .as_array()
                .is_some_and(|items| !items.is_empty())
        );
        assert!(
            discovery["transitions"]
                .as_array()
                .is_some_and(|items| !items.is_empty())
        );
        assert_eq!(discovery["noise_threshold_milli"], 125);
        assert_eq!(discovery["variable_arc_threshold_milli"], 900);
        assert!(
            discovery["variable_arcs"]
                .as_u64()
                .is_some_and(|value| value > 0)
        );
        assert_eq!(discovery["source"], "order-management.jsonocel");

        let model_body = serde_json::to_string(&discovery).unwrap();
        let model_upload = request(
            &server,
            &format!(
                "POST /api/ocel/model HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nX-File-Name: approved.ocelmodel.json\r\nContent-Length: {}\r\n\r\n{model_body}",
                server.address(),
                server.token(),
                model_body.len()
            ),
        );
        assert!(model_upload.starts_with("HTTP/1.1 200"), "{model_upload}");
        let model_upload: serde_json::Value =
            serde_json::from_str(model_upload.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(model_upload["file_name"], "approved.ocelmodel.json");
        assert_eq!(model_upload["object_types"], 3);
        assert!(model_upload["places"].as_u64().unwrap() > 0);
        assert!(model_upload["transitions"].as_u64().unwrap() > 0);

        let conformance_command = serde_json::json!({
            "upload_id": uploaded["upload_id"],
            "model_id": model_upload["model_id"],
            "detail_limit": 1
        })
        .to_string();
        let conformance = request(
            &server,
            &format!(
                "POST /api/ocel/conformance HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{conformance_command}",
                server.address(),
                server.token(),
                conformance_command.len()
            ),
        );
        assert!(conformance.starts_with("HTTP/1.1 200"), "{conformance}");
        let conformance: serde_json::Value =
            serde_json::from_str(conformance.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(conformance["source"], "order-management.jsonocel");
        assert_eq!(conformance["model_source"], "approved.ocelmodel.json");
        assert_eq!(
            conformance["model_training_source"],
            "order-management.jsonocel"
        );
        assert_eq!(conformance["fitness_millionths"], 1_000_000);
        assert_eq!(conformance["lifecycle_fit_objects"], 4);
        assert_eq!(conformance["replayed_objects"], 4);
        assert_eq!(conformance["binding_violations"], 0);
        assert_eq!(conformance["objects_available"], 4);
        assert_eq!(conformance["objects_returned"], 1);
        assert_eq!(conformance["objects_truncated"], true);
        assert!(!conformance.to_string().contains("open-process-"));

        let alignment_command = serde_json::json!({
            "upload_id": uploaded["upload_id"],
            "model_id": model_upload["model_id"],
            "detail_limit": 1,
            "algorithm": "alignment"
        })
        .to_string();
        let alignment = request(
            &server,
            &format!(
                "POST /api/ocel/conformance HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{alignment_command}",
                server.address(),
                server.token(),
                alignment_command.len()
            ),
        );
        assert!(alignment.starts_with("HTTP/1.1 200"), "{alignment}");
        let alignment: serde_json::Value =
            serde_json::from_str(alignment.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(alignment["source"], "order-management.jsonocel");
        assert_eq!(alignment["model_source"], "approved.ocelmodel.json");
        assert_eq!(alignment["total_cost"], 0);
        assert_eq!(alignment["fitness_millionths"], 1_000_000);
        assert_eq!(alignment["lifecycle_fit_objects"], 4);
        assert_eq!(alignment["aligned_objects"], 4);
        assert_eq!(alignment["binding_violations"], 0);
        assert_eq!(alignment["objects_returned"], 1);
        assert_eq!(alignment["objects_truncated"], true);
        assert!(!alignment.to_string().contains("open-process-"));
    }

    #[test]
    fn object_centric_xml_upload_uses_the_same_native_analysis() {
        let server = Server::bind(0).unwrap();
        let body = include_str!("../../../testdata/ocel/order-management.xmlocel");
        let uploaded = request(
            &server,
            &format!(
                "POST /api/ocel HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nX-File-Name: order-management.xmlocel\r\nContent-Length: {}\r\n\r\n{body}",
                server.address(),
                server.token(),
                body.len()
            ),
        );
        assert!(uploaded.starts_with("HTTP/1.1 200"), "{uploaded}");
        let uploaded: serde_json::Value =
            serde_json::from_str(uploaded.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(uploaded["analysis"]["source"], "order-management.xmlocel");
        assert_eq!(uploaded["analysis"]["scope"]["events_shown"], 3);
        assert_eq!(uploaded["analysis"]["scope"]["objects_shown"], 4);
        assert_eq!(
            uploaded["analysis"]["graphs"]["directly_follows"][0]["mean_elapsed_micros"],
            86_400_000_000_i64
        );
    }

    #[test]
    fn object_centric_sqlite_upload_uses_the_same_native_analysis() {
        let server = Server::bind(0).unwrap();
        let body = include_bytes!("../../../testdata/ocel/order-management.sqlite");
        let mut request = format!(
            "POST /api/ocel HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nX-File-Name: order-management.sqlite\r\nContent-Length: {}\r\n\r\n",
            server.address(),
            server.token(),
            body.len()
        )
        .into_bytes();
        request.extend_from_slice(body);
        let uploaded = request_bytes(&server, &request);
        assert!(uploaded.starts_with("HTTP/1.1 200"), "{uploaded}");
        let uploaded: serde_json::Value =
            serde_json::from_str(uploaded.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(uploaded["analysis"]["source"], "order-management.sqlite");
        assert_eq!(uploaded["analysis"]["scope"]["events_shown"], 3);
        assert_eq!(uploaded["analysis"]["scope"]["objects_shown"], 4);
        assert_eq!(
            uploaded["analysis"]["graphs"]["directly_follows"][0]["mean_elapsed_micros"],
            86_400_000_000_i64
        );
    }

    #[test]
    fn upload_rejects_missing_auth_and_unsupported_files() {
        let server = Server::bind(0).unwrap();
        let unauthorized = request(
            &server,
            &format!(
                "POST /api/profile HTTP/1.1\r\nHost: {}\r\nX-File-Name: log.csv\r\nContent-Length: 1\r\n\r\nx",
                server.address()
            ),
        );
        assert!(unauthorized.starts_with("HTTP/1.1 401"));

        let unsupported = request(
            &server,
            &format!(
                "POST /api/profile HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nX-File-Name: notes.txt\r\nContent-Length: 1\r\n\r\nx",
                server.address(),
                server.token()
            ),
        );
        assert!(unsupported.starts_with("HTTP/1.1 415"));

        let wrong_model_name = request(
            &server,
            &format!(
                "POST /api/ocel/model HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nX-File-Name: model.json\r\nContent-Length: 2\r\n\r\n{{}}",
                server.address(),
                server.token()
            ),
        );
        assert!(wrong_model_name.starts_with("HTTP/1.1 415"));

        let malformed_model = request(
            &server,
            &format!(
                "POST /api/ocel/model HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nX-File-Name: model.ocelmodel.json\r\nContent-Length: 8\r\n\r\nnot-json",
                server.address(),
                server.token()
            ),
        );
        assert!(malformed_model.starts_with("HTTP/1.1 422"));
        assert!(malformed_model.contains("not a portable object-centric model"));

        let wrong_case_model_name = request(
            &server,
            &format!(
                "POST /api/model HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nX-File-Name: model.ocelmodel.json\r\nContent-Length: 2\r\n\r\n{{}}",
                server.address(),
                server.token()
            ),
        );
        assert!(wrong_case_model_name.starts_with("HTTP/1.1 415"));

        let malformed_case_model = request(
            &server,
            &format!(
                "POST /api/model HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nX-File-Name: model.model.json\r\nContent-Length: 8\r\n\r\nnot-json",
                server.address(),
                server.token()
            ),
        );
        assert!(malformed_case_model.starts_with("HTTP/1.1 422"));
        assert!(malformed_case_model.contains("not a portable executable process model"));
    }

    #[test]
    fn item_endpoint_pages_sorts_and_searches() {
        let server = Server::bind(0).unwrap();
        let mut body = String::from("case_id,activity,timestamp\n");
        for n in 0..120 {
            body.push_str(&format!(
                "I-{n:03},Receive,2026-01-01T00:00:00Z\nI-{n:03},Pay,2026-01-01T{:02}:00:00Z\n",
                1 + n % 20
            ));
        }
        let profiled = request(
            &server,
            &format!(
                "POST /api/profile HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nX-File-Name: items.csv\r\nContent-Length: {}\r\n\r\n{body}",
                server.address(),
                server.token(),
                body.len()
            ),
        );
        let profile: serde_json::Value =
            serde_json::from_str(profiled.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        let post = |payload: serde_json::Value| {
            let payload = payload.to_string();
            let response = request(
                &server,
                &format!(
                    "POST /api/items HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{payload}",
                    server.address(),
                    server.token(),
                    payload.len()
                ),
            );
            assert!(response.starts_with("HTTP/1.1 200"), "{response}");
            serde_json::from_str::<serde_json::Value>(response.split("\r\n\r\n").nth(1).unwrap())
                .unwrap()
        };
        let first = post(
            serde_json::json!({"upload_id": profile["upload_id"], "mapping": profile["proposed_mapping"]}),
        );
        assert_eq!(first["total"], 120);
        assert_eq!(first["items"].as_array().unwrap().len(), 50);
        assert_eq!(first["items"][0]["duration_seconds"], 20 * 3600);
        let last = post(
            serde_json::json!({"upload_id": profile["upload_id"], "mapping": profile["proposed_mapping"], "offset": 100}),
        );
        assert_eq!(last["items"].as_array().unwrap().len(), 20);
        let found = post(
            serde_json::json!({"upload_id": profile["upload_id"], "mapping": profile["proposed_mapping"], "search": "i-11", "order": "identifier"}),
        );
        assert_eq!(found["total"], 10);
        assert_eq!(found["items"][0]["case"], "I-110");
    }

    #[test]
    fn value_endpoint_totals_amounts_inside_a_number_filter() {
        let server = Server::bind(0).unwrap();
        let body = concat!(
            "case_id,activity,timestamp,amount\n",
            "I-1,Receive,2026-01-01T09:00:00Z,\"75,000\"\n",
            "I-1,Pay,2026-01-02T09:00:00Z,\"75,000\"\n",
            "I-2,Receive,2026-01-01T09:00:00Z,52000\n",
            "I-3,Receive,2026-01-01T09:00:00Z,900\n",
            "I-3,Pay,2026-01-03T09:00:00Z,900\n",
        );
        let profiled = request(
            &server,
            &format!(
                "POST /api/profile HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nX-File-Name: invoices.csv\r\nContent-Length: {}\r\n\r\n{body}",
                server.address(),
                server.token(),
                body.len()
            ),
        );
        let profile: serde_json::Value =
            serde_json::from_str(profiled.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        let mut mapping = profile["proposed_mapping"].clone();
        mapping["case_attributes"] = serde_json::json!(["amount"]);
        let post = |payload: serde_json::Value| {
            let payload = payload.to_string();
            let response = request(
                &server,
                &format!(
                    "POST /api/values HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{payload}",
                    server.address(),
                    server.token(),
                    payload.len()
                ),
            );
            let status = response[9..12].to_owned();
            let body: serde_json::Value =
                serde_json::from_str(response.split("\r\n\r\n").nth(1).unwrap()).unwrap();
            (status, body)
        };
        let (status, body) = post(serde_json::json!({
            "upload_id": profile["upload_id"],
            "mapping": mapping,
            "filter": { "attribute_between": [{ "attribute": "amount", "at_least": 50000 }] },
            "rules": { "observation_window": {
                "start_micros": 0, "end_micros": 4_000_000_000_000_000_i64, "end_activities": ["Pay"]
            } },
            "field": "amount",
            "grouping": { "by": "last_step" }
        }));
        assert_eq!(status, "200", "{body}");
        assert_eq!(body["overall"]["cases"], 2);
        assert_eq!(body["overall"]["total"], 127_000.0);
        assert_eq!(body["overall"]["open_total"], 52_000.0);
        assert_eq!(body["groups"][0]["label"], "Pay");

        let (status, body) = post(serde_json::json!({
            "upload_id": profile["upload_id"],
            "mapping": mapping,
            "field": "activity"
        }));
        assert_eq!(status, "422");
        assert!(
            body["error"]
                .as_str()
                .unwrap()
                .contains("not a case attribute"),
            "{body}"
        );
    }

    #[test]
    fn comparison_endpoints_use_the_confirmed_active_view() {
        let server = Server::bind(0).unwrap();
        let body = concat!(
            "case_id,activity,timestamp,region\n",
            "A-1,Receive,2026-01-01T09:00:00Z,East\n",
            "A-1,Approve,2026-01-01T10:00:00Z,East\n",
            "A-2,Receive,2026-01-02T09:00:00Z,West\n",
            "A-2,Review,2026-01-02T11:00:00Z,West\n",
            "B-1,Receive,2026-03-01T09:00:00Z,East\n",
            "B-1,Approve,2026-03-01T12:00:00Z,East\n",
            "B-2,Receive,2026-03-02T09:00:00Z,West\n",
            "B-2,Review,2026-03-02T13:00:00Z,West\n",
            "ignored,Receive,2026-04-01T09:00:00Z,North\n",
        );
        let profiled = request(
            &server,
            &format!(
                "POST /api/profile HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nX-File-Name: comparison.csv\r\nContent-Length: {}\r\n\r\n{body}",
                server.address(),
                server.token(),
                body.len()
            ),
        );
        let profile: serde_json::Value =
            serde_json::from_str(profiled.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        let common = serde_json::json!({
            "upload_id": profile["upload_id"],
            "mapping": profile["proposed_mapping"],
            "filter": { "min_events": 2 }
        });

        let options = common.to_string();
        let options = request(
            &server,
            &format!(
                "POST /api/compare/options HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{options}",
                server.address(),
                server.token(),
                options.len()
            ),
        );
        assert!(options.starts_with("HTTP/1.1 200"), "{options}");
        let options: serde_json::Value =
            serde_json::from_str(options.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(options["attributes"][0]["name"], "region");
        assert_eq!(
            options["attributes"][0]["values"],
            serde_json::json!(["East", "West"])
        );

        let mut cohorts = common.clone();
        cohorts["attribute"] = serde_json::json!("region");
        cohorts["left"] = serde_json::json!("East");
        cohorts["right"] = serde_json::json!("West");
        let cohorts = cohorts.to_string();
        let cohorts = request(
            &server,
            &format!(
                "POST /api/compare/cohorts HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{cohorts}",
                server.address(),
                server.token(),
                cohorts.len()
            ),
        );
        assert!(cohorts.starts_with("HTTP/1.1 200"), "{cohorts}");
        let cohorts: serde_json::Value =
            serde_json::from_str(cohorts.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(cohorts["left"]["cases"], 2);
        assert_eq!(cohorts["right"]["cases"], 2);
        assert_eq!(cohorts["mean_throughput_difference_seconds"], 3_600);

        let mut exploration = common.clone();
        exploration["exploration"] = serde_json::json!({"segment":{"from":"Receive","to":"Approve","min_micros":3600000000_i64,"max_micros":7200000000_i64},"group_by":"region"});
        let body = exploration.to_string();
        let result = request(
            &server,
            &format!(
                "POST /api/explore HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                server.address(),
                server.token(),
                body.len()
            ),
        );
        assert!(result.starts_with("HTTP/1.1 200"), "{result}");
        let result: serde_json::Value =
            serde_json::from_str(result.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(result["overall"]["items"], 4);
        assert_eq!(result["overall"]["measurable"], 2);
        assert_eq!(result["selected"]["items"], 1);
        assert_eq!(result["groups_total"], 2);

        let mut map = common.clone();
        map["selection"] = serde_json::json!({ "mode": "routes", "variants": [2] });
        let map = map.to_string();
        let map = request(
            &server,
            &format!(
                "POST /api/map HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{map}",
                server.address(),
                server.token(),
                map.len()
            ),
        );
        assert!(map.starts_with("HTTP/1.1 200"), "{map}");
        let map: serde_json::Value =
            serde_json::from_str(map.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        // The filtered view holds four cases; rank 2 selects exactly its two cases.
        assert_eq!(map["selection"]["variants"], serde_json::json!([2]));
        assert_eq!(map["cases_total"], 4);
        assert_eq!(map["cases_covered"], 2);
        assert_eq!(map["variants_total"], 2);

        // The same map as a BPMN file for modelling tools.
        let mut bpmn = common.clone();
        bpmn["selection"] = serde_json::json!({ "mode": "routes", "variants": [2] });
        bpmn["name"] = serde_json::json!("Claims <test>");
        let bpmn = bpmn.to_string();
        let bpmn = request(
            &server,
            &format!(
                "POST /api/map/bpmn HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{bpmn}",
                server.address(),
                server.token(),
                bpmn.len()
            ),
        );
        assert!(bpmn.starts_with("HTTP/1.1 200"), "{bpmn}");
        assert!(bpmn.contains("Content-Type: application/xml"));
        let body = bpmn.split("\r\n\r\n").nth(1).unwrap();
        assert!(body.starts_with("<?xml"));
        assert!(body.contains("name=\"Claims &lt;test&gt;\""));
        assert!(body.contains("<bpmndi:BPMNDiagram"));

        let mut periods = common;
        periods["baseline_start"] = serde_json::json!(1_767_225_600_000_000_i64);
        periods["baseline_end"] = serde_json::json!(1_767_398_400_000_000_i64);
        periods["comparison_start"] = serde_json::json!(1_772_323_200_000_000_i64);
        periods["comparison_end"] = serde_json::json!(1_772_496_000_000_000_i64);
        let periods = periods.to_string();
        let periods = request(
            &server,
            &format!(
                "POST /api/compare/periods HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{periods}",
                server.address(),
                server.token(),
                periods.len()
            ),
        );
        assert!(periods.starts_with("HTTP/1.1 200"), "{periods}");
        let periods: serde_json::Value =
            serde_json::from_str(periods.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(periods["baseline"]["cases"], 2);
        assert_eq!(periods["comparison"]["cases"], 2);
        assert_eq!(periods["mean_throughput_difference_seconds"], 7_200);
    }

    #[test]
    fn analysis_returns_structured_quality_decisions_instead_of_guessing() {
        let server = Server::bind(0).unwrap();
        let body = include_str!("../../../testdata/canonical/07-missing-timestamps.csv");
        let profiled = request(
            &server,
            &format!(
                "POST /api/profile HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nX-File-Name: missing.csv\r\nContent-Length: {}\r\n\r\n{body}",
                server.address(),
                server.token(),
                body.len()
            ),
        );
        let payload = profiled.split("\r\n\r\n").nth(1).unwrap();
        let profile: serde_json::Value = serde_json::from_str(payload).unwrap();
        let command = serde_json::json!({
            "upload_id": profile["upload_id"],
            "mapping": profile["proposed_mapping"]
        })
        .to_string();
        let analyzed = request(
            &server,
            &format!(
                "POST /api/analyze HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{command}",
                server.address(),
                server.token(),
                command.len()
            ),
        );
        assert!(analyzed.starts_with("HTTP/1.1 200"), "{analyzed}");
        let payload = analyzed.split("\r\n\r\n").nth(1).unwrap();
        let value: serde_json::Value = serde_json::from_str(payload).unwrap();
        assert_eq!(value["status"], "blocked");
        assert_eq!(value["quality"]["ready_for_analysis"], false);
        assert!(
            value["quality"]["summary"]["open_decisions"]
                .as_u64()
                .unwrap()
                > 0
        );
        assert!(
            value["quality"]["findings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|finding| finding["kind"] == "blank_timestamps")
        );

        let answered_command = serde_json::json!({
            "upload_id": profile["upload_id"],
            "mapping": profile["proposed_mapping"],
            "policy": { "blank_timestamps": "missing_data" }
        })
        .to_string();
        let answered = request(
            &server,
            &format!(
                "POST /api/analyze HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{answered_command}",
                server.address(),
                server.token(),
                answered_command.len()
            ),
        );
        assert!(answered.starts_with("HTTP/1.1 200"), "{answered}");
        let payload = answered.split("\r\n\r\n").nth(1).unwrap();
        let value: serde_json::Value = serde_json::from_str(payload).unwrap();
        assert_eq!(value["status"], "complete");
        assert_eq!(value["analysis"]["summary"]["cases"], 3);
        assert_eq!(value["analysis"]["summary"]["events"], 8);
        assert_eq!(value["analysis"]["summary"]["incomplete_cases"], 1);
        assert_eq!(value["analysis"]["summary"]["duration_eligible_cases"], 2);
    }

    #[test]
    fn temporary_upload_is_deleted_when_its_scope_ends() {
        let path = {
            let upload = TemporaryUpload::create(".csv").unwrap();
            assert!(upload.path.exists());
            upload.path.clone()
        };
        assert!(!path.exists());
    }

    #[test]
    fn shutdown_flag_stops_the_run_loop() {
        let server = Server::bind(0).unwrap();
        let shutdown = Arc::new(AtomicBool::new(false));
        let signal = shutdown.clone();
        let thread = std::thread::spawn(move || server.run(signal).unwrap());
        shutdown.store(true, Ordering::Release);
        thread.join().unwrap();
    }
}
