//! Exercise the installed stdio command across real application restarts.
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
struct Fixture(PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn app(root: &Path) -> (Process, String) {
    let mut child = Process(
        Command::new(env!("CARGO_BIN_EXE_glasstarn"))
            .args(["serve", "--no-open", "--workspace-dir"])
            .arg(root)
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let mut output = BufReader::new(child.0.stdout.take().unwrap());
    loop {
        let mut line = String::new();
        assert!(
            output.read_line(&mut line).unwrap() > 0,
            "service stopped before launch"
        );
        if let Some(url) = line.trim().strip_prefix("open ") {
            return (child, url.into());
        }
    }
}
fn request(url: &str, path: &str, body: Option<Value>) -> Value {
    let (address, token) = url
        .strip_prefix("http://")
        .unwrap()
        .split_once("/#token=")
        .unwrap();
    let mut stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let method = if body.is_some() { "POST" } else { "GET" };
    let body = body.map(|v| v.to_string()).unwrap_or_default();
    write!(stream,"{method} {path} HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {token}\r\nContent-Length: {}\r\n\r\n{body}",body.len()).unwrap();
    let mut response = String::new();
    let _ = stream.read_to_string(&mut response);
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap()
}
#[test]
fn configured_proxy_reconnects_and_fails_closed_for_disabled_or_wrong_project() {
    let root = Fixture(std::env::temp_dir().join(format!(
            "pm-agent-cli-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )));
    let (service, hub) = app(&root.0);
    let first = request(
        &hub,
        "/api/workspaces/create",
        Some(json!({"name":"One","purpose":"audit"})),
    );
    let url = first["url"].as_str().unwrap();
    let setup = request(url, "/api/agent/enable", Some(json!({"preset":"private"})));
    let child = request(url, "/api/workspaces/create", Some(json!({"name":"Two"})));
    assert_eq!(
        request(child["url"].as_str().unwrap(), "/api/agent", None)["enabled"],
        false,
        "per-project consent must not spread to a newly created project"
    );
    let args: Vec<&str> = setup["generic_config"]["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    let mut proxy = Process(
        Command::new(env!("CARGO_BIN_EXE_glasstarn"))
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let mut input = proxy.0.stdin.take().unwrap();
    let mut output = BufReader::new(proxy.0.stdout.take().unwrap());
    let mut rpc = |message: Value| {
        writeln!(input, "{message}").unwrap();
        input.flush().unwrap();
        let mut line = String::new();
        assert!(output.read_line(&mut line).unwrap() > 0, "proxy stopped");
        serde_json::from_str::<Value>(&line).unwrap()
    };
    let initialized = rpc(
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"integration-client","version":"1"}}}),
    );
    assert!(
        initialized["result"]["serverInfo"].is_object(),
        "{initialized}"
    );
    let tools = rpc(json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}));
    assert!(
        tools["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["name"] == "list_projects")
    );
    assert_eq!(
        request(url, "/api/agent", None)["contact"]["tools_listed"],
        true
    );
    drop(service);
    let offline = rpc(json!({"jsonrpc":"2.0","id":3,"method":"tools/list"}));
    assert_eq!(offline["error"]["code"], -32000);
    let (_service, hub) = app(&root.0);
    let reopened = request(
        &hub,
        "/api/workspaces/open",
        Some(json!({"id":first["project"]["id"]})),
    );
    let url = reopened["url"].as_str().unwrap();
    assert!(
        rpc(json!({"jsonrpc":"2.0","id":4,"method":"tools/list"}))["result"]["tools"].is_array()
    );
    request(url, "/api/agent/disable", Some(json!({})));
    assert!(
        rpc(json!({"jsonrpc":"2.0","id":5,"method":"tools/list"}))["error"]["message"]
            .as_str()
            .unwrap()
            .contains("disabled")
    );
    request(
        url,
        "/api/agent/enable",
        Some(json!({"preset":"business_names"})),
    );
    assert!(
        rpc(json!({"jsonrpc":"2.0","id":6,"method":"tools/list"}))["result"]["tools"].is_array()
    );
    let session = Path::new(args[2]);
    let original = std::fs::read(session).unwrap();
    let mut wrong: Value = serde_json::from_slice(&original).unwrap();
    wrong["project_id"] = child["project"]["id"].clone();
    std::fs::write(session, wrong.to_string()).unwrap();
    assert!(
        rpc(json!({"jsonrpc":"2.0","id":7,"method":"tools/list"}))["error"]["message"]
            .as_str()
            .unwrap()
            .contains("different project")
    );
    std::fs::write(session, original).unwrap();
}
