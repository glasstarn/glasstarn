//! Telling people when a newer glasstarn exists, and installing it.
//!
//! At most once a day glasstarn asks the npm registry for the latest version
//! number. Nothing about the person, their projects or their data is sent; it
//! is a plain request for one public number. `GLASSTARN_NO_UPDATE_CHECK=1`, or
//! `"update_check": false` in `~/.glasstarn/settings.json`, turns it off.
//! `glasstarn update` installs the new version and refreshes the agents.
use serde_json::{Value, json};
use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};

const LATEST_URL: &str = "https://registry.npmjs.org/glasstarn/latest";
const DAY: u64 = 24 * 60 * 60;

pub(crate) fn current() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

fn glasstarn_home() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(PathBuf::from(home).join(".glasstarn"))
}

fn checks_allowed() -> bool {
    if std::env::var_os("GLASSTARN_NO_UPDATE_CHECK").is_some() {
        return false;
    }
    let settings = glasstarn_home()
        .and_then(|home| std::fs::read_to_string(home.join("settings.json")).ok())
        .and_then(|text| serde_json::from_str::<Value>(&text).ok());
    settings.is_none_or(|settings| settings["update_check"] != json!(false))
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Whether `latest` is a later version than `current`, by number.
fn newer(latest: &str, current: &str) -> bool {
    let parts = |v: &str| -> Vec<u64> {
        v.split(['.', '-'])
            .take(3)
            .map(|p| p.parse().unwrap_or(0))
            .collect()
    };
    parts(latest) > parts(current)
}

/// The newer version, when one is known. Refreshes what is known in the
/// background, at most once a day, without slowing anything down.
pub(crate) fn available() -> Option<String> {
    if !checks_allowed() {
        return None;
    }
    let file = glasstarn_home()?.join("update-check.json");
    let known = std::fs::read_to_string(&file)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok());
    let checked = known
        .as_ref()
        .and_then(|k| k["checked_at"].as_u64())
        .unwrap_or(0);
    if now().saturating_sub(checked) > DAY {
        std::thread::spawn(move || {
            let Some(latest) = fetch_latest() else { return };
            let _ = std::fs::create_dir_all(file.parent().unwrap_or(&file));
            let _ = std::fs::write(
                &file,
                json!({"checked_at": now(), "latest": latest}).to_string(),
            );
        });
    }
    let latest = known.as_ref()?["latest"].as_str()?.to_owned();
    newer(&latest, current()).then_some(latest)
}

/// The latest published version, asked now (for `glasstarn doctor`), unless
/// update checks are turned off.
pub(crate) fn latest_now() -> Option<String> {
    checks_allowed().then(fetch_latest).flatten()
}

/// Whether `latest` is a later version than this one.
pub(crate) fn is_newer(latest: &str) -> bool {
    newer(latest, current())
}

fn fetch_latest() -> Option<String> {
    let out = Command::new("curl")
        .args(["-fsS", "--max-time", "4", LATEST_URL])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let value: Value = serde_json::from_slice(&out.stdout).ok()?;
    Some(value["version"].as_str()?.to_owned())
}

/// `glasstarn update`: installs the latest version the way glasstarn was
/// installed, then reconnects the agents.
pub(crate) fn command(args: &[String]) -> Result<ExitCode, String> {
    if let Some(extra) = args.first() {
        return Err(format!("update takes no options, got '{extra}'"));
    }
    let latest =
        fetch_latest().ok_or("could not reach the npm registry to check for a newer glasstarn")?;
    if !newer(&latest, current()) {
        println!("glasstarn {} is the latest version.", current());
        return Ok(ExitCode::SUCCESS);
    }
    println!("Updating glasstarn {} to {latest}…", current());
    // The npm launcher sets this; other installs update their own way.
    if std::env::var_os("GLASSTARN_PLUGIN_DIR").is_none() {
        println!(
            "This copy was not installed with npm. Install the npm version with `npx glasstarn setup`."
        );
        return Ok(ExitCode::SUCCESS);
    }
    let npm = if cfg!(windows) { "npm.cmd" } else { "npm" };
    let installed = Command::new(npm)
        .args(["install", "-g", &format!("glasstarn@{latest}")])
        .status()
        .map_err(|e| format!("could not run npm: {e}"))?;
    if !installed.success() {
        return Err("npm could not install the new version".into());
    }
    // The new version's own setup refreshes Codex and Claude.
    let glasstarn = if cfg!(windows) {
        "glasstarn.cmd"
    } else {
        "glasstarn"
    };
    let _ = Command::new(glasstarn).arg("setup").status();
    if let Some(home) = glasstarn_home() {
        let _ = std::fs::write(
            home.join("update-check.json"),
            json!({"checked_at": now(), "latest": latest}).to_string(),
        );
    }
    println!("Done. Restart your agent to use glasstarn {latest}.");
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::newer;

    #[test]
    fn compares_versions_by_number() {
        assert!(newer("0.2.0", "0.1.0"));
        assert!(newer("0.10.0", "0.9.3"));
        assert!(newer("1.0.0", "0.99.99"));
        assert!(!newer("0.1.0", "0.1.0"));
        assert!(!newer("0.1.0", "0.2.0"));
    }
}
