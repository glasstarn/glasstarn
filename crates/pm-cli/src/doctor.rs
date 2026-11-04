//! `glasstarn doctor`: is everything in order? One plain line per check, and
//! for anything wrong, the one thing to do about it.
use std::path::{Path, PathBuf};
use std::process::ExitCode;

struct Check {
    ok: bool,
    what: String,
    fix: Option<String>,
}

pub(crate) fn command(args: &[String]) -> Result<ExitCode, String> {
    if let Some(extra) = args.iter().find(|arg| *arg != "--json") {
        return Err(format!("doctor takes no options, got '{extra}'"));
    }
    let json = args.iter().any(|arg| arg == "--json");
    let home = PathBuf::from(
        std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .ok_or("cannot find your home folder")?,
    );
    let mut checks = vec![version()];
    checks.extend(agents(&home));
    checks.push(on_path());
    checks.extend(pages(&home));
    checks.extend(folders(&home));
    let failed = checks.iter().filter(|check| !check.ok).count();
    if json {
        let list: Vec<_> = checks
            .iter()
            .map(|c| serde_json::json!({"ok": c.ok, "check": c.what, "fix": c.fix}))
            .collect();
        println!(
            "{}",
            serde_json::json!({"version": crate::update::current(), "checks": list, "problems": failed})
        );
    } else {
        println!("glasstarn doctor\n");
        for check in &checks {
            println!("  {}  {}", if check.ok { "✓" } else { "✗" }, check.what);
            if let Some(fix) = &check.fix {
                println!("     Fix: {fix}");
            }
        }
        println!(
            "\n{}",
            if failed == 0 {
                "Everything is in order.".to_owned()
            } else {
                format!(
                    "{failed} {} to fix.",
                    if failed == 1 { "thing" } else { "things" }
                )
            }
        );
    }
    Ok(if failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

fn pass(what: impl Into<String>) -> Check {
    Check {
        ok: true,
        what: what.into(),
        fix: None,
    }
}

fn fail(what: impl Into<String>, fix: impl Into<String>) -> Check {
    Check {
        ok: false,
        what: what.into(),
        fix: Some(fix.into()),
    }
}

fn version() -> Check {
    let current = crate::update::current();
    match crate::update::latest_now() {
        Some(latest) if crate::update::is_newer(&latest) => fail(
            format!("glasstarn {current} is installed; {latest} is available"),
            "run `glasstarn update`, then restart your agent",
        ),
        Some(_) => pass(format!("glasstarn {current} is the latest version")),
        None => pass(format!(
            "glasstarn {current} is installed (could not check for a newer version)"
        )),
    }
}

/// Claude Code and Codex, when installed: is the glasstarn plugin connected?
fn agents(home: &Path) -> Vec<Check> {
    let codex_home = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".codex"));
    [
        (
            "Claude Code",
            "claude",
            home.join(".claude/plugins/installed_plugins.json"),
            "glasstarn@glasstarn",
        ),
        (
            "Codex",
            "codex",
            codex_home.join("config.toml"),
            "glasstarn@glasstarn",
        ),
    ]
    .into_iter()
    .filter(|(_, program, file, _)| crate::setup::on_path(program).is_some() || file.is_file())
    .map(|(label, _, file, marker)| {
        let connected = std::fs::read_to_string(&file).is_ok_and(|text| text.contains(marker));
        if connected {
            pass(format!("{label} is connected to glasstarn"))
        } else {
            fail(
                format!("{label} is installed but not connected to glasstarn"),
                "run `glasstarn setup`, then restart the agent",
            )
        }
    })
    .collect()
}

fn on_path() -> Check {
    if crate::setup::installed_command().is_some() {
        pass("The glasstarn command works from any folder")
    } else {
        fail(
            "The glasstarn command is not on your PATH, so agents cannot run it",
            "run `npx glasstarn setup`",
        )
    }
}

/// Every running results page: does it answer, and is it this version?
fn pages(home: &Path) -> Vec<Check> {
    let main = home.join(".glasstarn").join("projects");
    let mut found = Vec::new();
    for root in pm_project::folders::all_roots(&main) {
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        for entry in entries.flatten() {
            let note = entry.path().join(".browser-session.json");
            let Ok(text) = std::fs::read_to_string(&note) else {
                continue;
            };
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
                continue;
            };
            let Some(url) = value["url"].as_str() else {
                continue;
            };
            let Some(address) = url.strip_prefix("http://").and_then(|rest| {
                rest.trim_end_matches('/')
                    .parse::<std::net::SocketAddr>()
                    .ok()
            }) else {
                continue;
            };
            if std::net::TcpStream::connect_timeout(&address, std::time::Duration::from_millis(300))
                .is_err()
            {
                continue;
            }
            let name = std::fs::read_to_string(entry.path().join("workspace.json"))
                .ok()
                .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
                .and_then(|value| value["name"].as_str().map(str::to_owned))
                .unwrap_or_else(|| "a project".to_owned());
            if value["build"].as_str() == Some(pm_server::build_id()) {
                found.push(pass(format!(
                    "The page for {name} is running this version ({url})"
                )));
            } else {
                found.push(fail(
                    format!("The page for {name} is running an older version ({url})"),
                    "ask your agent to open the results again, or run `glasstarn open`; the page restarts on this version",
                ));
            }
        }
    }
    if found.is_empty() {
        found.push(pass(
            "No results page is running (one starts when your agent opens results)",
        ));
    }
    found
}

/// Can glasstarn save projects where it keeps them?
fn folders(home: &Path) -> Vec<Check> {
    let main = home.join(".glasstarn").join("projects");
    let mut roots = vec![main.clone()];
    roots.extend(pm_project::folders::remembered_roots());
    roots.dedup();
    roots
        .into_iter()
        .filter(|root| root == &main || root.is_dir())
        .map(|root| {
            let probe = root.join(format!(".glasstarn-doctor-{}", std::process::id()));
            let writable = std::fs::create_dir_all(&root).is_ok()
                && std::fs::write(&probe, b"").is_ok()
                && std::fs::remove_file(&probe).is_ok();
            if writable {
                pass(format!("Projects can be saved in {}", root.display()))
            } else {
                fail(
                    format!("Projects cannot be saved in {}", root.display()),
                    "check that the folder exists and that you can write to it, or ask your agent to use another folder",
                )
            }
        })
        .collect()
}
