//! Browser-owned consent, stable per-project connection files, and setup output.
use super::*;
use pm_project::workspace::WorkspaceFolder;
use serde_json::{Value, json};

fn issue(error: impl ToString) -> String {
    error.to_string()
}

impl Server {
    pub(super) fn restore_agent_access(&self, folder: &WorkspaceFolder) -> Result<(), String> {
        if let Some(saved) = folder
            .read_agent_file(".agent-access.json")
            .map_err(issue)?
        {
            if saved["version"] != 1 || !saved["enabled"].is_boolean() {
                return Err("invalid saved agent access settings".into());
            }
            let policy = if saved["enabled"] == true {
                let policy: pm_mcp::ExposurePolicy =
                    serde_json::from_value(saved["policy"].clone()).map_err(issue)?;
                policy.validate()?;
                Some(policy)
            } else {
                None
            };
            *self.mcp_policy.lock().map_err(issue)? = policy;
        }
        let policy = self.mcp_policy.lock().map_err(issue)?;
        self.publish_agent_session(folder, policy.as_ref())
    }

    fn publish_agent_session(
        &self,
        folder: &WorkspaceFolder,
        policy: Option<&pm_mcp::ExposurePolicy>,
    ) -> Result<(), String> {
        let value = match policy {
            Some(policy) => {
                json!({"version":1,"project_id":folder.manifest.id,"enabled":true,"port":self.address.port(),"token":policy.project_delegation_token(&folder.manifest.id)})
            }
            None => json!({"version":1,"project_id":folder.manifest.id,"enabled":false}),
        };
        folder
            .write_agent_file(".agent-session.json", &value)
            .map_err(issue)
    }

    pub(super) fn agent_request(
        &self,
        stream: &mut TcpStream,
        request: Request,
    ) -> Result<(), ServerError> {
        let result = (|| -> Result<Value, String> {
            let folder = self.workspace_folder.lock().map_err(issue)?;
            let folder = folder
                .as_ref()
                .ok_or("Open a project before connecting an agent.")?;
            if request.method == "POST" {
                let body = request
                    .json_body(stream)
                    .map_err(|(_, _, message)| message.to_owned())?;
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Selection {
                    preset: Option<String>,
                }
                let selection: Selection = serde_json::from_slice(&body).map_err(issue)?;
                let next = if request.target == "/api/agent/enable" {
                    let mut policy = pm_mcp::ExposurePolicy::safe();
                    match selection.preset.as_deref() {
                        Some("private") => {}
                        Some("business_names") => {
                            policy.activity_names = pm_mcp::Mode::Show;
                            policy.column_names = pm_mcp::Mode::Show;
                            policy.object_type_names = pm_mcp::Mode::Show;
                            policy.relationship_qualifiers = pm_mcp::Mode::Show;
                        }
                        _ => return Err("Choose which information the agent may see.".into()),
                    }
                    Some(policy)
                } else {
                    None
                };
                // Persist consent before exposing data. Revoke in memory even if
                // publishing the disabled session subsequently fails.
                folder
                    .write_agent_file(
                        ".agent-access.json",
                        &json!({"version":1,"enabled":next.is_some(),"policy":next}),
                    )
                    .map_err(issue)?;
                *self.mcp_policy.lock().map_err(issue)? = next.clone();
                *self.agent_contact.lock().map_err(issue)? = None;
                self.publish_agent_session(folder, next.as_ref())?;
            }
            self.agent_status(folder)
        })();
        match result {
            Ok(value) => write_json(stream, 200, "OK", &value),
            Err(message) => write_json(stream, 409, "Conflict", &ErrorResponse::new(&message)),
        }
    }

    fn agent_status(&self, folder: &WorkspaceFolder) -> Result<Value, String> {
        let policy = self.mcp_policy.lock().map_err(issue)?;
        let live = self.live_workspace.lock().map_err(issue)?;
        let executable = std::env::current_exe()
            .map_err(issue)?
            .display()
            .to_string();
        let connection = folder
            .root
            .join(".agent-session.json")
            .display()
            .to_string();
        let id = &folder.manifest.id;
        let name = format!("glasstarn-{id}");
        let args = vec!["mcp", "--connection", &connection, "--project-id", id];
        let quoted = std::iter::once(executable.as_str())
            .chain(args.iter().copied())
            .map(shell_quote)
            .collect::<Vec<_>>()
            .join(" ");
        let config = json!({"command":executable,"args":args});
        let toml = format!(
            "[mcp_servers.{name}]\ncommand = {}\nargs = {}\n",
            serde_json::to_string(&executable).map_err(issue)?,
            serde_json::to_string(&args).map_err(issue)?
        );
        let contact = self.agent_contact.lock().map_err(issue)?.clone();
        let prompt = format!(
            "Use the {name} tools to look at my process in Glasstarn. Start with get_claim_contract and list_projects. Take every number from Glasstarn, cite the evidence, and ask me what I want to find out first."
        );
        Ok(json!({
            "enabled":policy.is_some(),"project_id":id,"project_name":folder.manifest.name,
            "purpose":folder.manifest.purpose,"analysis_ready":live.case_project.is_some() || live.ocel_log.is_some(),
            "policy":policy.as_ref().map(pm_mcp::ExposurePolicy::public_summary),
            "contact":contact,"server_name":name,"prompt":prompt,
            "shell":if cfg!(windows) {"PowerShell"} else {"Terminal"},
            "codex_command":format!("codex mcp add {name} -- {quoted}"),
            "claude_command":format!("claude mcp add --transport stdio --scope local {name} -- {quoted}"),
            "codex_config":toml,
            "claude_config":json!({"mcpServers":{&name:config}}),
            "vscode_config":json!({"servers":{&name:{"type":"stdio","command":executable,"args":args}}}),
            "generic_config":config,
        }))
    }

    pub(super) fn record_agent_contact(&self, message: &Value) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        if let Ok(mut contact) = self.agent_contact.lock() {
            let mut value = contact.take().unwrap_or_else(|| json!({}));
            value["last_seen"] = json!(now);
            if message["method"] == "initialize" {
                value["client"] = json!(
                    message["params"]["clientInfo"]["name"]
                        .as_str()
                        .unwrap_or("MCP client")
                        .chars()
                        .take(100)
                        .collect::<String>()
                );
                value["initialized"] = json!(true);
            }
            if message["method"] == "tools/list" {
                value["tools_listed"] = json!(true);
            }
            *contact = Some(value);
        }
    }
}

fn shell_quote(value: &str) -> String {
    if cfg!(windows) {
        format!("'{}'", value.replace('\'', "''"))
    } else {
        format!("'{}'", value.replace('\'', "'\"'\"'"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn setup() -> (Server, PathBuf) {
        let mut entropy = [0; 16];
        OsRng.fill_bytes(&mut entropy);
        let id = hex(&entropy);
        let root = std::env::temp_dir().join(format!("pm-agent-{id}"));
        let folder = WorkspaceFolder::create(
            &root,
            &id,
            "Audit's $project",
            pm_project::workspace::InvestigationPurpose::Audit,
        )
        .unwrap();
        let server = Server::bind(0).unwrap();
        server.restore_agent_access(&folder).unwrap();
        *server.workspace_folder.lock().unwrap() = Some(folder);
        (server, root)
    }
    fn call(server: &Server, method: &str, path: &str, body: Value, token: &str) -> (u16, Value) {
        let body = if method == "GET" {
            String::new()
        } else {
            body.to_string()
        };
        let mut stream = TcpStream::connect(server.address).unwrap();
        write!(stream,"{method} {path} HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {token}\r\nContent-Length: {}\r\n\r\n{body}",server.address,body.len()).unwrap();
        let (incoming, _) = server.listener.accept().unwrap();
        server.handle(incoming).unwrap();
        let mut response = String::new();
        if let Err(error) = stream.read_to_string(&mut response) {
            assert!(
                error.kind() == std::io::ErrorKind::ConnectionReset
                    && response.contains("\r\n\r\n"),
                "{error}"
            );
        }
        (
            response.split_whitespace().nth(1).unwrap().parse().unwrap(),
            serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap(),
        )
    }
    #[test]
    fn explicit_enable_contact_revoke_and_restart_preserve_project_scope() {
        let (server, root) = setup();
        let (status, initial) = call(&server, "GET", "/api/agent", json!({}), server.token());
        assert_eq!(status, 200);
        assert_eq!(initial["enabled"], false);
        assert!(initial["contact"].is_null());
        assert_eq!(
            call(
                &server,
                "POST",
                "/api/agent/enable",
                json!({}),
                server.token()
            )
            .0,
            409
        );
        let (status, enabled) = call(
            &server,
            "POST",
            "/api/agent/enable",
            json!({"preset":"business_names"}),
            server.token(),
        );
        assert_eq!(status, 200);
        assert_eq!(enabled["policy"]["activity_names"], "show");
        assert_eq!(enabled["policy"]["case_ids"], "alias");
        let session: Value =
            serde_json::from_slice(&fs::read(root.join(".agent-session.json")).unwrap()).unwrap();
        let token = session["token"].as_str().unwrap();
        assert!(!enabled.to_string().contains(token));
        assert!(!enabled.to_string().contains("alias_key"));
        let id = enabled["project_id"].as_str().unwrap();
        assert_eq!(enabled["generic_config"]["args"][4], id);
        assert!(
            enabled["codex_command"]
                .as_str()
                .unwrap()
                .contains("--connection")
        );
        assert_eq!(call(&server, "GET", "/api/agent", json!({}), token).0, 401);
        assert_eq!(call(&server,"POST","/api/mcp",json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test-client","version":"1"}}}),token).0,200);
        call(
            &server,
            "POST",
            "/api/mcp",
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
            token,
        );
        let state = call(&server, "GET", "/api/agent", json!({}), server.token()).1;
        assert_eq!(state["contact"]["client"], "test-client");
        assert_eq!(state["contact"]["tools_listed"], true);
        let old_port = server.address.port();
        drop(server);
        let restored = Server::bind(0).unwrap();
        let folder = WorkspaceFolder::open(&root).unwrap();
        restored.restore_agent_access(&folder).unwrap();
        *restored.workspace_folder.lock().unwrap() = Some(folder);
        let restored_session: Value =
            serde_json::from_slice(&fs::read(root.join(".agent-session.json")).unwrap()).unwrap();
        assert_eq!(restored_session["project_id"], session["project_id"]);
        assert_eq!(restored_session["port"], restored.address.port());
        let _ = old_port; // Ephemeral ports may legally be reused.
        assert_eq!(
            call(
                &restored,
                "POST",
                "/api/agent/disable",
                json!({}),
                restored.token()
            )
            .0,
            200
        );
        assert_eq!(
            call(
                &restored,
                "POST",
                "/api/mcp",
                json!({"jsonrpc":"2.0","id":3,"method":"tools/list"}),
                token
            )
            .0,
            401
        );
        drop(restored);
        // An explicit project opt-out also overrides a serve-time default policy.
        let final_server =
            Server::bind_with_agent(0, Some(pm_mcp::ExposurePolicy::safe())).unwrap();
        let folder = WorkspaceFolder::open(&root).unwrap();
        final_server.restore_agent_access(&folder).unwrap();
        assert!(final_server.mcp_policy.lock().unwrap().is_none());
        drop(folder);
        drop(final_server);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn running_service_accepts_mcp_body_arriving_after_headers() {
        let (server, root) = setup();
        call(
            &server,
            "POST",
            "/api/agent/enable",
            json!({"preset":"private"}),
            server.token(),
        );
        let session: Value =
            serde_json::from_slice(&fs::read(root.join(".agent-session.json")).unwrap()).unwrap();
        let address = server.address;
        let shutdown = Arc::new(AtomicBool::new(false));
        let signal = shutdown.clone();
        let worker = std::thread::spawn(move || server.run(signal));
        let mut stream = TcpStream::connect(address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let body = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"split-request","version":"1"}}}).to_string();
        write!(stream, "POST /api/mcp HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {}\r\nContent-Length: {}\r\n\r\n", session["token"].as_str().unwrap(), body.len()).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        let _ = stream.write_all(body.as_bytes());
        let mut response = String::new();
        let _ = stream.read_to_string(&mut response);
        shutdown.store(true, Ordering::Release);
        worker.join().unwrap().unwrap();
        fs::remove_dir_all(root).unwrap();
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    }

    #[cfg(unix)]
    #[test]
    fn connection_secrets_are_private_and_symlinks_cannot_overwrite_other_files() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let (server, root) = setup();
        call(
            &server,
            "POST",
            "/api/agent/enable",
            json!({"preset":"private"}),
            server.token(),
        );
        for name in [".agent-access.json", ".agent-session.json"] {
            assert_eq!(
                fs::metadata(root.join(name)).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let protected = root.join("keep.txt");
        fs::write(&protected, "unchanged").unwrap();
        fs::remove_file(root.join(".agent-session.json")).unwrap();
        symlink(&protected, root.join(".agent-session.json")).unwrap();
        assert_eq!(
            call(
                &server,
                "POST",
                "/api/agent/disable",
                json!({}),
                server.token()
            )
            .0,
            409
        );
        assert!(server.mcp_policy.lock().unwrap().is_none());
        assert_eq!(fs::read_to_string(protected).unwrap(), "unchanged");
        drop(server);
        fs::remove_dir_all(root).unwrap();
    }
}
