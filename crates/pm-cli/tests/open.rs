//! `glasstarn open` gives a plain link to the project, and asking again while
//! that page is running gives the same page instead of a second glasstarn
//! that could not open the project.
use std::process::Command;

fn glasstarn(args: &[&str]) -> serde_json::Value {
    let out = Command::new(env!("CARGO_BIN_EXE_glasstarn"))
        .args(args)
        .env("GLASSTARN_NO_BROWSER", "1")
        .env("GLASSTARN_NO_UPDATE_CHECK", "1")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

#[test]
fn an_sop_check_runs_while_the_page_is_open_and_the_page_gets_the_model() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_str().unwrap();
    let made = glasstarn(&["demo", "sales-pipeline", "--workspace-dir", root, "--json"]);
    let id = made["project"]["id"].as_str().unwrap().to_owned();
    let page = glasstarn(&["open", "--workspace-dir", root, "--project", &id]);
    let port = page["url"]
        .as_str()
        .unwrap()
        .trim_start_matches("http://127.0.0.1:")
        .split('/')
        .next()
        .unwrap()
        .to_owned();
    let call = |method: &str, route: &str, body: &str| {
        let out = Command::new("curl")
            .args([
                "-s",
                "-X",
                method,
                "-H",
                &format!("Origin: http://127.0.0.1:{port}"),
                "-H",
                "Content-Type: application/json",
            ])
            .args(if body.is_empty() {
                vec![]
            } else {
                vec!["--data", body]
            })
            .arg(format!("http://127.0.0.1:{port}{route}"))
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    // The page opens the project, which then owns its settings and models.
    let opened: serde_json::Value = serde_json::from_str(&call(
        "POST",
        "/api/workspaces/open",
        &format!(r#"{{"id":"{id}"}}"#),
    ))
    .unwrap();
    let project_port = opened["port"].as_u64().unwrap();
    // Check against an expected process discovered from the data, written out
    // as an agent would write one from an SOP.
    let discover = dir.path().join("discover.json");
    std::fs::write(&discover, r#"{"type":"discover"}"#).unwrap();
    let found = glasstarn(&[
        "workspace",
        "query",
        &id,
        "--request",
        discover.to_str().unwrap(),
        "--workspace-dir",
        root,
    ]);
    let model = dir.path().join("expected.json");
    std::fs::write(
        &model,
        serde_json::json!({"source":"SOP","tree":found["response"]["result"]["tree"]}).to_string(),
    )
    .unwrap();
    let check = dir.path().join("check.json");
    std::fs::write(
        &check,
        serde_json::json!({"type":"conformance","model":model,"algorithm":"token","limit":5})
            .to_string(),
    )
    .unwrap();
    let checked = glasstarn(&[
        "workspace",
        "query",
        &id,
        "--request",
        check.to_str().unwrap(),
        "--workspace-dir",
        root,
    ]);
    assert!(
        checked["response"]["result"]["fit_cases"].is_number(),
        "{checked}"
    );
    // The open page is handed the model once, and adopts it.
    let report = Command::new("curl")
        .args([
            "-s",
            "-H",
            &format!("Origin: http://127.0.0.1:{project_port}"),
            &format!("http://127.0.0.1:{project_port}/api/workspace/investigation"),
        ])
        .output()
        .unwrap();
    let report: serde_json::Value = serde_json::from_slice(&report.stdout).unwrap();
    assert!(
        report["pending_model"]["text"]
            .as_str()
            .unwrap()
            .contains("tree")
    );
    assert_eq!(
        report["pending_model"]["sha256"].as_str().unwrap().len(),
        64
    );
    // Offered until a page adopts it, so a second open page gets it too.
    assert!(
        dir.path()
            .join(&id)
            .join("pending-model.model.json")
            .exists()
    );
    for port in [port, project_port.to_string()] {
        if let Ok(out) = Command::new("lsof")
            .args(["-ti", &format!("tcp:{port}")])
            .output()
        {
            for pid in String::from_utf8_lossy(&out.stdout).split_whitespace() {
                let _ = Command::new("kill").arg(pid).status();
            }
        }
    }
}

#[test]
fn open_twice_gives_the_same_running_page() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_str().unwrap();
    let made = glasstarn(&["demo", "sales-pipeline", "--workspace-dir", root, "--json"]);
    let id = made["project"]["id"].as_str().unwrap().to_owned();
    let first = glasstarn(&["open", "--workspace-dir", root, "--project", &id]);
    let url = first["url"].as_str().unwrap().to_owned();
    assert!(
        url.starts_with("http://127.0.0.1:") && url.ends_with(&format!("/?project={id}")),
        "{url}"
    );
    assert!(!url.contains("token"), "{url}");
    // Opening the project in that page leaves a note in the project folder.
    let port = url
        .trim_start_matches("http://127.0.0.1:")
        .split('/')
        .next()
        .unwrap();
    let body = format!("{{\"id\":\"{id}\"}}");
    let opened = Command::new("curl")
        .args([
            "-s",
            "-X",
            "POST",
            "-H",
            &format!("Origin: http://127.0.0.1:{port}"),
            "-H",
            "Content-Type: application/json",
            "--data",
            &body,
            &format!("http://127.0.0.1:{port}/api/workspaces/open"),
        ])
        .output()
        .unwrap();
    let opened: serde_json::Value = serde_json::from_slice(&opened.stdout).unwrap();
    let page = opened["url"].as_str().unwrap().to_owned();
    let again = glasstarn(&["open", "--workspace-dir", root, "--project", &id]);
    assert_eq!(
        again["url"].as_str().unwrap(),
        page.split("/#").next().unwrap().to_owned() + "/"
    );
    let _ = Command::new("pkill").args(["-f", root]).status();
}

// After an update or reinstall, a page still running from the old build
// must not be reused: an agent's `open` would show the person the old screens.
#[test]
fn a_page_left_running_by_an_older_build_is_restarted_not_reused() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_str().unwrap();
    let made = glasstarn(&["demo", "sales-pipeline", "--workspace-dir", root, "--json"]);
    let id = made["project"]["id"].as_str().unwrap().to_owned();
    let first = glasstarn(&["open", "--workspace-dir", root, "--project", &id]);
    let port = first["url"]
        .as_str()
        .unwrap()
        .trim_start_matches("http://127.0.0.1:")
        .split('/')
        .next()
        .unwrap()
        .to_owned();
    let body = format!("{{\"id\":\"{id}\"}}");
    let opened = Command::new("curl")
        .args([
            "-s",
            "-X",
            "POST",
            "-H",
            &format!("Origin: http://127.0.0.1:{port}"),
            "-H",
            "Content-Type: application/json",
            "--data",
            &body,
            &format!("http://127.0.0.1:{port}/api/workspaces/open"),
        ])
        .output()
        .unwrap();
    let opened: serde_json::Value = serde_json::from_slice(&opened.stdout).unwrap();
    let old_page = opened["url"]
        .as_str()
        .unwrap()
        .split("/#")
        .next()
        .unwrap()
        .to_owned()
        + "/";
    // Same build: the running page is reused.
    let again = glasstarn(&["open", "--workspace-dir", root, "--project", &id]);
    assert_eq!(again["already_running"], true);
    // Pretend that page came from an older install, as after a reinstall.
    let note = dir.path().join(&id).join(".browser-session.json");
    let mut recorded: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&note).unwrap()).unwrap();
    recorded["build"] = serde_json::json!("an-older-build");
    std::fs::write(&note, recorded.to_string()).unwrap();
    let fresh = glasstarn(&["open", "--workspace-dir", root, "--project", &id]);
    assert!(fresh["already_running"].is_null(), "{fresh}");
    assert_ne!(fresh["url"].as_str().unwrap(), old_page, "{fresh}");
    let old_address: std::net::SocketAddr = old_page
        .trim_start_matches("http://")
        .trim_end_matches('/')
        .parse()
        .unwrap();
    assert!(
        std::net::TcpStream::connect_timeout(&old_address, std::time::Duration::from_millis(300))
            .is_err(),
        "the old page still answers"
    );
    let _ = Command::new("pkill").args(["-f", root]).status();
}

// A new chat says "analyze" while yesterday's page still has the project
// open: the analysis runs again instead of failing on the page's lock.
#[test]
fn a_project_open_in_a_page_can_be_analysed_again() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_str().unwrap();
    let made = glasstarn(&["demo", "sales-pipeline", "--workspace-dir", root, "--json"]);
    let id = made["project"]["id"].as_str().unwrap().to_owned();
    let first = glasstarn(&["open", "--workspace-dir", root, "--project", &id]);
    let port = first["url"]
        .as_str()
        .unwrap()
        .trim_start_matches("http://127.0.0.1:")
        .split('/')
        .next()
        .unwrap()
        .to_owned();
    let body = format!("{{\"id\":\"{id}\"}}");
    let opened = Command::new("curl")
        .args([
            "-s",
            "-X",
            "POST",
            "-H",
            &format!("Origin: http://127.0.0.1:{port}"),
            "-H",
            "Content-Type: application/json",
            "--data",
            &body,
            &format!("http://127.0.0.1:{port}/api/workspaces/open"),
        ])
        .output()
        .unwrap();
    assert!(serde_json::from_slice::<serde_json::Value>(&opened.stdout).unwrap()["port"].is_u64());
    let analysed = glasstarn(&[
        "workspace",
        "analyze",
        &id,
        "--workspace-dir",
        root,
        "--json",
    ]);
    assert_eq!(analysed["status"], "complete", "{analysed}");
    let saved = analysed["saved_as"].as_str().unwrap();
    assert!(
        dir.path()
            .join(&id)
            .join(saved.split(' ').next().unwrap())
            .is_file(),
        "{saved}"
    );
    let _ = Command::new("pkill").args(["-f", root]).status();
}

#[test]
fn projects_saved_in_another_folder_are_listed_with_the_rest() {
    let home = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_glasstarn"))
            .args(args)
            .env("HOME", home.path())
            .env("GLASSTARN_NO_BROWSER", "1")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice::<serde_json::Value>(&out.stdout).unwrap()
    };
    let other = elsewhere.path().to_str().unwrap();
    run(&["demo", "sales-pipeline", "--workspace-dir", other, "--json"]);
    run(&["demo", "order-to-cash", "--json"]);
    // The projects page of the default home lists both.
    let page = run(&["open"]);
    let url = page["url"].as_str().unwrap().to_owned();
    let port = url
        .trim_start_matches("http://127.0.0.1:")
        .split('/')
        .next()
        .unwrap()
        .to_owned();
    let listed = Command::new("curl")
        .args([
            "-s",
            "-H",
            &format!("Origin: http://127.0.0.1:{port}"),
            &format!("http://127.0.0.1:{port}/api/workspaces"),
        ])
        .output()
        .unwrap();
    let listed = String::from_utf8_lossy(&listed.stdout).into_owned();
    // Stop the page this test started.
    if let Ok(out) = Command::new("lsof")
        .args(["-ti", &format!("tcp:{port}")])
        .output()
    {
        for pid in String::from_utf8_lossy(&out.stdout).split_whitespace() {
            let _ = Command::new("kill").arg(pid).status();
        }
    }
    assert!(listed.contains("Sales pipeline (demo)"), "{listed}");
    assert!(listed.contains("Order to cash (demo)"), "{listed}");
}

#[test]
fn an_unused_page_stops_by_itself() {
    let dir = tempfile::tempdir().unwrap();
    let mut serve = Command::new(env!("CARGO_BIN_EXE_glasstarn"))
        .args([
            "serve",
            "--no-open",
            "--workspace-dir",
            dir.path().to_str().unwrap(),
        ])
        .env("GLASSTARN_IDLE_SECONDS", "2")
        .env("GLASSTARN_NO_UPDATE_CHECK", "1")
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let started = std::time::Instant::now();
    loop {
        if serve.try_wait().unwrap().is_some() {
            break;
        }
        assert!(
            started.elapsed().as_secs() < 15,
            "still running after 15 seconds"
        );
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
}

#[test]
fn projects_can_be_renamed_and_deleted_to_the_trash() {
    let home = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_glasstarn"))
            .args(args)
            .env("HOME", home.path())
            .env("GLASSTARN_NO_BROWSER", "1")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice::<serde_json::Value>(&out.stdout).unwrap()
    };
    let made = run(&["demo", "sales-pipeline", "--json"]);
    let id = made["project"]["id"].as_str().unwrap().to_owned();
    let page = run(&["open"]);
    let port = page["url"]
        .as_str()
        .unwrap()
        .trim_start_matches("http://127.0.0.1:")
        .split('/')
        .next()
        .unwrap()
        .to_owned();
    let post = |route: &str, body: String| {
        let out = Command::new("curl")
            .args([
                "-s",
                "-X",
                "POST",
                "-H",
                &format!("Origin: http://127.0.0.1:{port}"),
                "-H",
                "Content-Type: application/json",
                "--data",
                &body,
                &format!("http://127.0.0.1:{port}{route}"),
            ])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    // Opened in this browser first, then renamed and deleted from the list.
    let opened = post("/api/workspaces/open", format!(r#"{{"id":"{id}"}}"#));
    assert!(opened.contains("url"), "{opened}");
    let renamed = post(
        "/api/workspaces/rename",
        format!(r#"{{"id":"{id}","name":"Deals review"}}"#),
    );
    assert!(renamed.contains("Deals review"), "{renamed}");
    let deleted = post("/api/workspaces/delete", format!(r#"{{"id":"{id}"}}"#));
    if let Ok(out) = Command::new("lsof")
        .args(["-ti", &format!("tcp:{port}")])
        .output()
    {
        for pid in String::from_utf8_lossy(&out.stdout).split_whitespace() {
            let _ = Command::new("kill").arg(pid).status();
        }
    }
    assert!(deleted.contains("moved_to"), "{deleted}");
    assert!(!home.path().join(".glasstarn/projects").join(&id).exists());
    // Moved, not erased: the project is wherever the reply says it went
    // (the Trash on a Mac or Linux desktop, a recoverable folder elsewhere).
    let moved: serde_json::Value = serde_json::from_str(&deleted).unwrap();
    let moved_to = std::path::PathBuf::from(moved["moved_to"].as_str().unwrap());
    assert!(
        moved_to.ends_with("Deals review (glasstarn project)"),
        "{deleted}"
    );
    assert!(moved_to.join("workspace.json").is_file(), "{deleted}");
}

#[test]
fn a_known_newer_version_is_mentioned_unless_checks_are_off() {
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join(".glasstarn")).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    std::fs::write(
        home.path().join(".glasstarn/update-check.json"),
        format!(r#"{{"checked_at":{now},"latest":"99.0.0"}}"#),
    )
    .unwrap();
    let open = |off: bool| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_glasstarn"));
        command
            .arg("open")
            .env("HOME", home.path())
            .env("GLASSTARN_NO_BROWSER", "1");
        if off {
            command.env("GLASSTARN_NO_UPDATE_CHECK", "1");
        }
        let out = command.output().unwrap();
        serde_json::from_slice::<serde_json::Value>(&out.stdout).unwrap()
    };
    let first = open(false);
    let port = first["url"]
        .as_str()
        .unwrap()
        .trim_start_matches("http://127.0.0.1:")
        .split('/')
        .next()
        .unwrap()
        .to_owned();
    assert_eq!(first["update_available"], "99.0.0");
    // The page answers its own health check with the same news.
    let health = Command::new("curl")
        .args([
            "-s",
            "-H",
            &format!("Origin: http://127.0.0.1:{port}"),
            &format!("http://127.0.0.1:{port}/api/health"),
        ])
        .output()
        .unwrap();
    let health: serde_json::Value = serde_json::from_slice(&health.stdout).unwrap();
    assert_eq!(health["update_available"], "99.0.0");
    assert!(open(true)["update_available"].is_null());
    if let Ok(out) = Command::new("lsof")
        .args(["-ti", &format!("tcp:{port}")])
        .output()
    {
        for pid in String::from_utf8_lossy(&out.stdout).split_whitespace() {
            let _ = Command::new("kill").arg(pid).status();
        }
    }
}

// "Show me the three main variants": a saved answer can say what the open
// page should show, and the record command checks it like the rest.
#[test]
fn an_answer_can_say_what_the_page_shows() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_str().unwrap();
    let made = glasstarn(&["demo", "purchase-to-pay", "--workspace-dir", root, "--json"]);
    let id = made["project"]["id"].as_str().unwrap().to_owned();
    let record = |show: &str| {
        let request = dir.path().join("show.json");
        std::fs::write(&request, format!(r#"{{"version":1,"question":"Show me the three main variants","answer":"The three main variants.","findings":[],"show":{show}}}"#)).unwrap();
        Command::new(env!("CARGO_BIN_EXE_glasstarn"))
            .args([
                "workspace",
                "record",
                &id,
                "--request",
                request.to_str().unwrap(),
                "--workspace-dir",
                root,
            ])
            .env("GLASSTARN_NO_UPDATE_CHECK", "1")
            .output()
            .unwrap()
    };
    let saved = record(r#"{"view":"variants","filter":{"top_variants":3}}"#);
    assert!(
        saved.status.success(),
        "{}",
        String::from_utf8_lossy(&saved.stderr)
    );
    let artifact: serde_json::Value = serde_json::from_slice(&saved.stdout).unwrap();
    let stored: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(
            dir.path()
                .join(&id)
                .join(artifact["artifact"].as_str().unwrap()),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(stored["note"]["show"]["view"], "variants");
    assert_eq!(stored["note"]["show"]["filter"]["top_variants"], 3);
    for (show, why) in [
        (r#"{"view":"dashboard"}"#, "an unknown page"),
        (
            r#"{"view":"process","detail":"some"}"#,
            "an unknown map detail",
        ),
        (r#"{"view":"process","colour":"red"}"#, "an unknown setting"),
        (
            r#"{"view":"items","filter":{"not_a_filter":1}}"#,
            "an unknown filter",
        ),
    ] {
        assert!(!record(show).status.success(), "{why} was accepted");
    }
}
