//! Extract approval. An agent files an extract; a person approves it here, in
//! the browser session, and the app runs exactly the approved spec.
//!
//! These routes accept only the browser token. The agent's delegated token is
//! valid for `/api/mcp` alone, so an agent's own tools cannot approve.
use super::*;
use pm_project::extract;
use serde_json::{Value, json};

impl Server {
    pub(super) fn extract_request(
        &self,
        stream: &mut TcpStream,
        request: Request,
    ) -> Result<(), ServerError> {
        match self.extract_operation(stream, request) {
            Ok(value) => write_json(stream, 200, "OK", &value),
            Err(message) => write_json(stream, 409, "Conflict", &ErrorResponse::new(&message)),
        }
    }

    fn extract_operation(&self, stream: &mut TcpStream, request: Request) -> Result<Value, String> {
        let root = self
            .workspace_folder
            .lock()
            .map_err(|_| "project state is unavailable")?
            .as_ref()
            .map(|folder| folder.root.clone())
            .ok_or("open a project before reviewing extracts")?;
        if request.method == "GET" {
            return Ok(json!({"requests":extract::requests(&root)?}));
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Decision {
            spec_sha256: String,
            person: String,
        }
        let decision: Decision = serde_json::from_slice(
            &request
                .json_body(stream)
                .map_err(|(_, _, message)| message.to_owned())?,
        )
        .map_err(|_| "send the extract and the name of the person deciding")?;
        if request.target.ends_with("/decline") {
            let declined = extract::decline(&root, &decision.spec_sha256, &decision.person)?;
            return Ok(json!({"request":declined}));
        }
        let approved = extract::approve(&root, &decision.spec_sha256, &decision.person)?;
        self.run_extract(root, decision.spec_sha256);
        Ok(json!({"request":approved}))
    }

    /// Downloads without the project lock, then registers under it briefly,
    /// so a long extract never blocks the rest of the app.
    fn run_extract(&self, root: PathBuf, sha256: String) {
        let folder = Arc::clone(&self.workspace_folder);
        let cli = self.salesforce_cli.clone();
        let (failure_root, failure_sha) = (root.clone(), sha256.clone());
        let started = std::thread::Builder::new()
            .name("extract".into())
            .spawn(move || {
                let tools = extract::Tools {
                    salesforce_cli: cli,
                    curl: std::path::PathBuf::from("curl"),
                    jira_user: None,
                    servicenow_user: None,
                    hubspot_token: None,
                };
                let outcome = extract::run(&root, &sha256, &tools).and_then(|staged| {
                    let mut guard = folder
                        .lock()
                        .map_err(|_| "project state is unavailable".to_owned())?;
                    let folder = guard.as_mut().ok_or("the project was closed")?;
                    extract::register(folder, staged)
                });
                if let Err(reason) = outcome {
                    let _ = extract::fail(&root, &sha256, &reason);
                }
            });
        if started.is_err() {
            let _ = extract::fail(&failure_root, &failure_sha, "the extract could not start");
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use pm_project::workspace::{InvestigationPurpose, WorkspaceFolder};
    use std::os::unix::fs::PermissionsExt;

    fn call(server: &Server, method: &str, path: &str, body: &str, token: &str) -> (u16, Value) {
        let mut stream = TcpStream::connect(server.address).unwrap();
        write!(stream, "{method} {path} HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {token}\r\nContent-Length: {}\r\n\r\n{body}", server.address, body.len()).unwrap();
        let (incoming, _) = server.listener.accept().unwrap();
        server.handle(incoming).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        let status = response.split_whitespace().nth(1).unwrap().parse().unwrap();
        let value = serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
        (status, value)
    }

    #[test]
    fn a_person_approves_in_the_app_and_the_app_runs_the_extract() {
        let dir = tempfile::tempdir().unwrap();
        let policy = pm_mcp::ExposurePolicy::safe();
        let mut server = Server::bind_with_agent(0, Some(policy.clone())).unwrap();
        let folder = WorkspaceFolder::create_named(
            &dir.path().join("projects"),
            "Support",
            InvestigationPurpose::Improvement,
        )
        .unwrap();
        let (root, id) = (folder.root.clone(), folder.manifest.id.clone());
        *server.workspace_folder.lock().unwrap() = Some(folder);
        let sf = dir.path().join("sf");
        fs::write(&sf, "#!/bin/sh\nwhile [ \"$#\" -gt 0 ]; do\n  case \"$1\" in --output-file) shift; output=\"$1\" ;; --query) shift; query=\"$1\" ;; esac\n  shift\ndone\ncase \"$query\" in\n  \"SELECT Id,CaseId FROM\"*) printf 'Id,CaseId\\nH1,500A\\n' > \"$output\" ;;\n  *) printf 'Id,CaseId,CreatedDate\\nH1,500A,2026-01-02T00:00:00.000Z\\n' > \"$output\" ;;\nesac\nprintf '{\"status\":0,\"result\":{\"totalSize\":1}}'\n").unwrap();
        fs::set_permissions(&sf, fs::Permissions::from_mode(0o700)).unwrap();
        server.salesforce_cli = sf;
        let spec =
            json!({"version":2,"system":"salesforce","target_org":"approved-org","api_version":66,
            "start":"2026-01-01T00:00:00Z","end":"2026-02-01T00:00:00Z","objects":[
            {"name":"CaseHistory","fields":["Id","CaseId","CreatedDate"],"time_field":"CreatedDate",
             "kind":"history","item_field":"CaseId","history_gaps":[]}]})
            .to_string();
        let reviewed = extract::review(spec.as_bytes()).unwrap();
        extract::request(&root, &reviewed).unwrap();

        let (status, listed) = call(
            &server,
            "GET",
            "/api/workspace/extracts",
            "",
            server.token(),
        );
        assert_eq!(status, 200);
        assert_eq!(listed["requests"][0]["status"], "pending");
        assert!(listed["requests"][0]["preview"]["scans"].is_array());

        let decision = json!({"spec_sha256":reviewed.sha256,"person":"Pat Lee"}).to_string();
        // The agent's delegated token cannot approve. (No body: the server
        // refuses before reading one, and unread bytes would reset the socket.)
        let agent = policy.project_delegation_token(&id);
        let (status, _) = call(
            &server,
            "POST",
            "/api/workspace/extracts/approve",
            "",
            &agent,
        );
        assert_eq!(status, 401);
        let (status, approved) = call(
            &server,
            "POST",
            "/api/workspace/extracts/approve",
            &decision,
            server.token(),
        );
        assert_eq!(status, 200, "{approved}");
        assert_eq!(approved["request"]["approved_by"], "Pat Lee");

        let mut finished = Value::Null;
        for _ in 0..200 {
            finished = extract::requests(&root).unwrap().remove(0);
            if finished["status"] != "running" {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(finished["status"], "complete", "{finished}");
        assert_eq!(finished["result"]["row_counts"][0]["rows"], 1);
        let state = server
            .workspace_folder
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .state()
            .unwrap();
        assert_eq!(state.preparation_sources.len(), 1);
        assert_eq!(pm_project::audit::verify(&root).unwrap(), 3);
    }
}
