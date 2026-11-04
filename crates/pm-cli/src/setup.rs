//! `glasstarn setup`: connects glasstarn to the agents on this computer.
//!
//! It copies the agent plugin that ships with glasstarn to `~/.glasstarn`,
//! then registers it with Codex and Claude Code when they are installed, using
//! their own plugin commands. Running it again refreshes everything. Nothing
//! is downloaded and nothing leaves the computer.
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

const MARKETPLACE: &str = "glasstarn";
const PLUGIN: &str = "glasstarn@glasstarn";

pub(crate) fn command(args: &[String]) -> Result<ExitCode, String> {
    if let Some(extra) = args.first() {
        return Err(format!("setup takes no options, got '{extra}'"));
    }
    let bundle = plugin_bundle()
        .ok_or("cannot find the agent plugin that ships with glasstarn; reinstall glasstarn")?;
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .ok_or("cannot find your home folder")?;
    let target = PathBuf::from(&home).join(".glasstarn").join("agent-plugin");
    if target.exists() {
        std::fs::remove_dir_all(&target)
            .map_err(|e| format!("cannot replace {}: {e}", target.display()))?;
    }
    copy_dir(&bundle, &target).map_err(|e| format!("cannot copy the agent plugin: {e}"))?;
    let place = target.to_string_lossy().into_owned();

    let home = PathBuf::from(home);
    println!("glasstarn setup\n");
    let mut connected = 0;
    for (name, label, steps) in [
        (
            "codex",
            "Codex",
            vec![
                vec!["plugin", "marketplace", "remove", MARKETPLACE],
                vec!["plugin", "marketplace", "add", place.as_str()],
                vec!["plugin", "add", PLUGIN],
            ],
        ),
        (
            "claude",
            "Claude Code",
            vec![
                vec!["plugin", "marketplace", "remove", MARKETPLACE],
                vec!["plugin", "marketplace", "add", place.as_str()],
                vec!["plugin", "install", PLUGIN],
            ],
        ),
    ] {
        let Some(program) = on_path(name) else {
            println!("  -  {label}: not installed, skipped");
            continue;
        };
        // The first step only clears an earlier setup, so its failure is fine.
        let _ = run(&program, &steps[0]);
        let failed = steps[1..].iter().find_map(|step| run(&program, step).err());
        match failed {
            None => {
                connected += 1;
                println!("  ✓  {label}: connected");
                // Codex runs glasstarn inside its sandbox, which stops only the
                // local results page, so that one command is allowed. Claude
                // Code asks before every command it has not been told about,
                // so glasstarn's commands are allowed there. Nothing else, and
                // setup says so.
                let allowed = if name == "codex" {
                    allow_in_codex(&home)
                } else {
                    allow_in_claude(&home)
                };
                let what = if name == "codex" {
                    "`glasstarn open` without asking, to show results in your browser"
                } else {
                    "glasstarn commands without asking"
                };
                match allowed {
                    Ok(()) => println!("     {label} may run {what}"),
                    Err(why) => println!("     {label} will ask before running glasstarn ({why})"),
                }
            }
            Some(why) => println!("  ✗  {label}: could not connect ({why})"),
        }
    }

    // Run through `npx glasstarn setup`, the program is only in npm's
    // temporary cache: install that same package for good, so agents can run
    // `glasstarn` from any folder afterwards.
    if installed_command().is_none()
        && let Some(package) = std::env::var_os("GLASSTARN_PLUGIN_DIR")
            .map(PathBuf::from)
            .and_then(|plugin| plugin.parent().map(Path::to_path_buf))
            .filter(|package| package.join("package.json").is_file())
        && let Some(npm) = on_path("npm")
    {
        match install_copy(&npm, &package) {
            Ok(()) => println!("  ✓  Installed the glasstarn command"),
            Err(why) => println!("  !  Could not install the glasstarn command ({why})"),
        }
    }
    match installed_command() {
        Some(_) => println!("  ✓  The glasstarn command works from any folder"),
        None => println!(
            "  !  The glasstarn command is not on your PATH, so agents cannot run it.\n     Install with `npm install -g` rather than npx."
        ),
    }
    if connected > 0 {
        println!(
            "\nNext: restart your agent, then type /glasstarn in Claude Code or $glasstarn\nin Codex, or just ask, for example \"why are our claims slow?\"."
        );
    } else {
        println!("\nInstall Codex or Claude Code, then run `glasstarn setup` again.");
    }
    println!("To open glasstarn in your browser, run `glasstarn`.");
    Ok(ExitCode::SUCCESS)
}

/// `glasstarn uninstall [--delete-projects]`: takes glasstarn out of Codex
/// and Claude Code (plugin, marketplace, permission rules), stops its pages,
/// removes its own settings and, for an npm install, the program itself.
/// Projects stay unless asked; deleted ones go to the Trash.
pub(crate) fn uninstall(args: &[String]) -> Result<ExitCode, String> {
    let delete_projects = match args {
        [] => false,
        [flag] if flag == "--delete-projects" => true,
        _ => return Err("uninstall takes only --delete-projects".into()),
    };
    let home = PathBuf::from(
        std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .ok_or("cannot find your home folder")?,
    );
    println!("glasstarn uninstall\n");
    for (name, label, steps) in [
        (
            "codex",
            "Codex",
            [
                vec!["plugin", "remove", PLUGIN],
                vec!["plugin", "marketplace", "remove", MARKETPLACE],
            ],
        ),
        (
            "claude",
            "Claude Code",
            [
                vec!["plugin", "uninstall", PLUGIN],
                vec!["plugin", "marketplace", "remove", MARKETPLACE],
            ],
        ),
    ] {
        let Some(program) = on_path(name) else {
            continue;
        };
        for step in &steps {
            let _ = run(&program, step);
        }
        let rules = if name == "codex" {
            forget_in_codex(&home)
        } else {
            forget_in_claude(&home)
        };
        match rules {
            Ok(()) => println!("  ✓  {label}: glasstarn removed"),
            Err(why) => println!(
                "  !  {label}: plugin removed, but its settings could not be cleaned ({why})"
            ),
        }
    }
    // Pages left running in the background.
    if cfg!(unix) {
        let _ = Command::new("pkill")
            .args(["-f", "glasstarn serve"])
            .status();
    }
    let own = home.join(".glasstarn");
    for name in [
        "agent-plugin",
        "update-check.json",
        "folders.json",
        "settings.json",
    ] {
        let path = own.join(name);
        let _ = if path.is_dir() {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
    }
    let projects = own.join("projects");
    if delete_projects && projects.is_dir() {
        let to = pm_project::folders::move_to_trash(&projects, "glasstarn projects")
            .map_err(|e| format!("could not move the projects to the Trash: {e}"))?;
        println!("  ✓  Projects moved to the Trash ({})", to.display());
    } else if projects.is_dir() {
        println!("  -  Your projects are kept in {}", projects.display());
    }
    println!("     Projects saved inside your own folders stay there.");
    if std::env::var_os("GLASSTARN_PLUGIN_DIR").is_some() {
        let npm = if cfg!(windows) { "npm.cmd" } else { "npm" };
        let removed = Command::new(npm)
            .args(["uninstall", "-g", "glasstarn"])
            .status();
        match removed {
            Ok(status) if status.success() => println!("  ✓  The glasstarn program is removed"),
            _ => println!("  !  Remove the program with: npm uninstall -g glasstarn"),
        }
    } else {
        println!("     Remove the glasstarn program the way you installed it.");
    }
    Ok(ExitCode::SUCCESS)
}

fn forget_in_codex(home: &Path) -> Result<(), String> {
    let root = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".codex"));
    let rules = root.join("rules").join("default.rules");
    let Ok(text) = std::fs::read_to_string(&rules) else {
        return Ok(());
    };
    let kept: Vec<&str> = text
        .lines()
        .filter(|line| line.trim() != OPEN_RULE)
        .collect();
    let mut out = kept.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    std::fs::write(&rules, out).map_err(|e| e.to_string())
}

fn forget_in_claude(home: &Path) -> Result<(), String> {
    let path = home.join(".claude").join("settings.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(());
    };
    let mut settings: serde_json::Value = serde_json::from_str(&text)
        .map_err(|_| "its settings file could not be read".to_owned())?;
    if let Some(allow) = settings
        .pointer_mut("/permissions/allow")
        .and_then(serde_json::Value::as_array_mut)
    {
        allow.retain(|rule| rule != CLAUDE_ALLOW && rule != CLAUDE_ALLOW_BEFORE);
    }
    let mut out = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
    out.push('\n');
    std::fs::write(&path, out).map_err(|e| e.to_string())
}

/// The command Codex and Claude Code may run without asking.
const OPEN_RULE: &str = r#"prefix_rule(pattern=["glasstarn", "open"], decision="allow")"#;
const CLAUDE_ALLOW: &str = "Bash(glasstarn:*)";
/// What earlier versions allowed in Claude Code, replaced by `CLAUDE_ALLOW`.
const CLAUDE_ALLOW_BEFORE: &str = "Bash(glasstarn open:*)";

/// Adds the `glasstarn open` rule to Codex's rules, once.
fn allow_in_codex(home: &Path) -> Result<(), String> {
    let root = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".codex"));
    let rules = root.join("rules").join("default.rules");
    let current = std::fs::read_to_string(&rules).unwrap_or_default();
    if current.lines().any(|line| line.trim() == OPEN_RULE) {
        return Ok(());
    }
    std::fs::create_dir_all(rules.parent().unwrap_or(&root)).map_err(|e| e.to_string())?;
    let mut text = current;
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(OPEN_RULE);
    text.push('\n');
    std::fs::write(&rules, text).map_err(|e| e.to_string())
}

/// Adds glasstarn's commands to Claude Code's allowed commands, once, keeping
/// every other setting.
fn allow_in_claude(home: &Path) -> Result<(), String> {
    let path = home.join(".claude").join("settings.json");
    let mut settings: serde_json::Value = match std::fs::read_to_string(&path) {
        Ok(text) if !text.trim().is_empty() => serde_json::from_str(&text)
            .map_err(|_| "its settings file could not be read".to_owned())?,
        _ => serde_json::json!({}),
    };
    let root = settings
        .as_object_mut()
        .ok_or("its settings file is not a settings object")?;
    let permissions = root
        .entry("permissions")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or("its permissions are not a settings object")?;
    let allow = permissions
        .entry("allow")
        .or_insert_with(|| serde_json::json!([]))
        .as_array_mut()
        .ok_or("its allowed commands are not a list")?;
    if allow.iter().any(|rule| rule == CLAUDE_ALLOW) {
        return Ok(());
    }
    allow.retain(|rule| rule != CLAUDE_ALLOW_BEFORE);
    allow.push(serde_json::json!(CLAUDE_ALLOW));
    std::fs::create_dir_all(path.parent().unwrap_or(home)).map_err(|e| e.to_string())?;
    let mut text = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
    text.push('\n');
    std::fs::write(&path, text).map_err(|e| e.to_string())
}

/// The plugin folder shipped next to the executable: set by the npm launcher,
/// beside the binary in an install, or the repository's own in a dev build.
fn plugin_bundle() -> Option<PathBuf> {
    let has_plugin = |dir: &Path| dir.join("glasstarn").join("skills").is_dir();
    if let Some(dir) = std::env::var_os("GLASSTARN_PLUGIN_DIR").map(PathBuf::from) {
        return has_plugin(&dir).then_some(dir);
    }
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    let mut candidates = Vec::new();
    for dir in exe.ancestors().skip(1).take(4) {
        candidates.push(dir.join("plugin"));
        candidates.push(dir.join("plugins"));
    }
    candidates.into_iter().find(|dir| has_plugin(dir))
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == ".DS_Store" {
            continue;
        }
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            copy_dir(&path, &to.join(&name))?;
        } else {
            std::fs::copy(&path, to.join(&name))?;
        }
    }
    Ok(())
}

/// Installs a self-contained copy of the package npx is running from. A plain
/// `npm install -g <folder>` would only link to npm's temporary cache, and the
/// command would break once npm clears it; a packed archive is copied in.
fn install_copy(npm: &Path, package: &Path) -> Result<(), String> {
    let staging = std::env::temp_dir().join(format!("glasstarn-setup-{}", std::process::id()));
    std::fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
    let packed = Command::new(npm)
        .args(["pack", "--silent", "--pack-destination"])
        .arg(&staging)
        .arg(package)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;
    let name = String::from_utf8_lossy(&packed.stdout)
        .lines()
        .last()
        .unwrap_or_default()
        .trim()
        .to_owned();
    let archive = staging.join(&name);
    let result = if packed.status.success() && !name.is_empty() && archive.is_file() {
        run(npm, &["install", "-g", &archive.to_string_lossy()])
    } else {
        Err("npm could not pack the package".to_owned())
    };
    let _ = std::fs::remove_dir_all(&staging);
    result
}

/// The installed `glasstarn` command. While `npx` runs us, its own temporary
/// copy is on PATH too; that one disappears afterwards, so it does not count.
pub(crate) fn installed_command() -> Option<PathBuf> {
    installed_in(&std::env::var_os("PATH")?)
}

fn installed_in(paths: &std::ffi::OsStr) -> Option<PathBuf> {
    std::env::split_paths(paths)
        .filter(|dir| !dir.components().any(|part| part.as_os_str() == "_npx"))
        .find_map(|dir| {
            ["glasstarn", "glasstarn.exe", "glasstarn.cmd"]
                .into_iter()
                .map(|file| dir.join(file))
                .find(|path| path.is_file())
        })
}

pub(crate) fn on_path(name: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths).find_map(|dir| {
        [
            name.to_owned(),
            format!("{name}.exe"),
            format!("{name}.cmd"),
        ]
        .into_iter()
        .map(|file| dir.join(file))
        .find(|path| path.is_file())
    })
}

fn run(program: &Path, args: &[&str]) -> Result<(), String> {
    let output = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        return Ok(());
    }
    let said = String::from_utf8_lossy(&output.stderr);
    let said = said
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("failed");
    Err(said.trim().chars().take(160).collect())
}

#[cfg(test)]
mod tests {
    // `npx glasstarn setup` runs from npm's temporary `_npx` folder, which is
    // on PATH only while it runs: that copy is not an installed command.
    #[test]
    fn the_temporary_npx_copy_does_not_count_as_installed() {
        let dir = tempfile::tempdir().unwrap();
        let npx = dir
            .path()
            .join("_npx")
            .join("abc")
            .join("node_modules")
            .join(".bin");
        let global = dir.path().join("prefix").join("bin");
        for folder in [&npx, &global] {
            std::fs::create_dir_all(folder).unwrap();
        }
        std::fs::write(npx.join("glasstarn"), b"").unwrap();
        let only_npx = std::env::join_paths([&npx]).unwrap();
        assert_eq!(super::installed_in(&only_npx), None);
        std::fs::write(global.join("glasstarn"), b"").unwrap();
        let both = std::env::join_paths([&npx, &global]).unwrap();
        assert_eq!(super::installed_in(&both), Some(global.join("glasstarn")));
    }
}
