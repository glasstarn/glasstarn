//! `glasstarn setup` copies the agent plugin to ~/.glasstarn and connects the
//! agents it finds; with none installed it says so and changes nothing else.
use std::process::Command;

#[test]
fn setup_copies_the_plugin_and_skips_missing_agents() {
    let home = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_glasstarn"))
        .arg("setup")
        .env("HOME", home.path())
        .env("PATH", home.path())
        .env_remove("GLASSTARN_PLUGIN_DIR")
        .output()
        .unwrap();
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{said}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(said.contains("Codex: not installed, skipped"), "{said}");
    assert!(
        said.contains("Claude Code: not installed, skipped"),
        "{said}"
    );
    assert!(said.contains("not on your PATH"), "{said}");
    let plugin = home.path().join(".glasstarn/agent-plugin");
    assert!(plugin.join("glasstarn/skills/glasstarn/SKILL.md").is_file());
    assert!(plugin.join(".agents/plugins/marketplace.json").is_file());
    assert!(plugin.join(".claude-plugin/marketplace.json").is_file());
}

#[cfg(unix)]
#[test]
fn setup_lets_connected_agents_open_results_without_asking_and_keeps_their_settings() {
    use std::os::unix::fs::PermissionsExt;
    let home = tempfile::tempdir().unwrap();
    let bin = home.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    // Stand-ins for Codex and Claude Code that accept any plugin command.
    for agent in ["codex", "claude"] {
        let path = bin.join(agent);
        std::fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    std::fs::create_dir_all(home.path().join(".claude")).unwrap();
    std::fs::write(
        home.path().join(".claude/settings.json"),
        r#"{"theme":"dark","permissions":{"allow":["Bash(git status)"]}}"#,
    )
    .unwrap();
    std::fs::create_dir_all(home.path().join(".codex/rules")).unwrap();
    std::fs::write(
        home.path().join(".codex/config.toml"),
        "model = \"gpt\"\n\n[sandbox_workspace_write]\nnetwork_access = false\n\n[tools]\nweb = true\n",
    )
    .unwrap();
    std::fs::write(
        home.path().join(".codex/rules/default.rules"),
        "prefix_rule(pattern=[\"git\", \"add\"], decision=\"allow\")",
    )
    .unwrap();
    let run = || {
        let out = Command::new(env!("CARGO_BIN_EXE_glasstarn"))
            .arg("setup")
            .env("HOME", home.path())
            .env("PATH", &bin)
            .env_remove("CODEX_HOME")
            .env_remove("GLASSTARN_PLUGIN_DIR")
            .output()
            .unwrap();
        assert!(out.status.success());
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    let said = run();
    assert!(
        said.contains("Codex may run `glasstarn open` without asking"),
        "{said}"
    );
    assert!(
        said.contains("Claude Code may run glasstarn commands without asking"),
        "{said}"
    );
    run();
    let rules = std::fs::read_to_string(home.path().join(".codex/rules/default.rules")).unwrap();
    assert!(
        rules.starts_with("prefix_rule(pattern=[\"git\", \"add\"]"),
        "{rules}"
    );
    assert_eq!(
        rules.matches(r#"pattern=["glasstarn", "open"]"#).count(),
        1,
        "{rules}"
    );
    // Setup leaves Codex's own settings alone.
    let config = std::fs::read_to_string(home.path().join(".codex/config.toml")).unwrap();
    assert!(!config.contains("writable_roots"), "{config}");
    let settings: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(home.path().join(".claude/settings.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(settings["theme"], "dark");
    assert_eq!(
        settings["permissions"]["allow"],
        serde_json::json!(["Bash(git status)", "Bash(glasstarn:*)"])
    );
}

#[cfg(unix)]
#[test]
fn uninstall_takes_glasstarn_out_of_the_agents_and_keeps_projects() {
    use std::os::unix::fs::PermissionsExt;
    let home = tempfile::tempdir().unwrap();
    let bin = home.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    for agent in ["codex", "claude"] {
        let path = bin.join(agent);
        std::fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    std::fs::create_dir_all(home.path().join(".claude")).unwrap();
    std::fs::write(
        home.path().join(".claude/settings.json"),
        r#"{"permissions":{"allow":["Bash(git status)"]}}"#,
    )
    .unwrap();
    std::fs::create_dir_all(home.path().join(".glasstarn/projects/keepme")).unwrap();
    let run = |what: &str| {
        let out = Command::new(env!("CARGO_BIN_EXE_glasstarn"))
            .arg(what)
            .env("HOME", home.path())
            .env("PATH", &bin)
            .env_remove("CODEX_HOME")
            .env_remove("GLASSTARN_PLUGIN_DIR")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    run("setup");
    let said = run("uninstall");
    assert!(
        said.contains("Codex: glasstarn removed")
            && said.contains("Claude Code: glasstarn removed"),
        "{said}"
    );
    let rules =
        std::fs::read_to_string(home.path().join(".codex/rules/default.rules")).unwrap_or_default();
    assert!(!rules.contains("glasstarn"), "{rules}");
    let settings = std::fs::read_to_string(home.path().join(".claude/settings.json")).unwrap();
    assert!(
        settings.contains("Bash(git status)") && !settings.contains("glasstarn"),
        "{settings}"
    );
    assert!(!home.path().join(".glasstarn/agent-plugin").exists());
    assert!(
        home.path().join(".glasstarn/projects/keepme").is_dir(),
        "projects are kept"
    );
}

// `npx glasstarn setup` runs from npm's temporary cache. The command it
// installs must be a copy, still working once npm clears that cache.
// Unix only here; the release workflow runs the same check on Windows.
#[cfg(unix)]
#[test]
fn npx_setup_installs_a_copy_that_survives_the_npx_cache_being_cleared() {
    let Some(npm) = [
        "/opt/homebrew/bin/npm",
        "/usr/local/bin/npm",
        "/usr/bin/npm",
    ]
    .into_iter()
    .map(std::path::PathBuf::from)
    .find(|path| path.is_file()) else {
        eprintln!("npm is not installed here; skipping");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    // Only node and npm on PATH: no installed glasstarn and no agents.
    let node_dir = dir.path().join("tools");
    std::fs::create_dir_all(&node_dir).unwrap();
    for tool in ["node", "npm"] {
        let found = npm.parent().unwrap().join(tool);
        std::os::unix::fs::symlink(
            std::fs::canonicalize(&found).unwrap_or(found),
            node_dir.join(tool),
        )
        .unwrap();
    }
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    // The package as npx unpacks it.
    let package = dir.path().join("cache/_npx/abc/node_modules/glasstarn");
    let platform = format!(
        "{}-{}",
        if cfg!(target_os = "macos") {
            "darwin"
        } else if cfg!(windows) {
            "win32"
        } else {
            "linux"
        },
        if cfg!(target_arch = "aarch64") {
            "arm64"
        } else {
            "x64"
        }
    );
    let native = package.join("native").join(&platform);
    std::fs::create_dir_all(&native).unwrap();
    std::fs::create_dir_all(package.join("bin")).unwrap();
    std::fs::copy(
        repo.join("packaging/npm/package.json"),
        package.join("package.json"),
    )
    .unwrap();
    std::fs::copy(
        repo.join("packaging/npm/bin/glasstarn.js"),
        package.join("bin/glasstarn.js"),
    )
    .unwrap();
    std::fs::copy(env!("CARGO_BIN_EXE_glasstarn"), native.join("glasstarn")).unwrap();
    for file in ["LICENSE", "THIRD_PARTY_NOTICES.txt"] {
        std::fs::copy(repo.join(file), package.join(file)).unwrap();
    }
    std::fs::copy(repo.join("ui/fonts/OFL.txt"), package.join("FONTS-OFL.txt")).unwrap();
    let plugin = package.join("plugin");
    for part in ["glasstarn", ".agents", ".claude-plugin"] {
        copy(&repo.join("plugins").join(part), &plugin.join(part));
    }
    let prefix = dir.path().join("prefix");
    let path = format!(
        "{}:{}:/usr/bin:/bin",
        prefix.join("bin").display(),
        node_dir.display()
    );
    let out = Command::new(native.join("glasstarn"))
        .arg("setup")
        .env("HOME", dir.path().join("home"))
        .env("PATH", &path)
        .env("npm_config_prefix", &prefix)
        .env("npm_config_cache", dir.path().join("npm-cache"))
        .env("GLASSTARN_PLUGIN_DIR", &plugin)
        .env("GLASSTARN_NO_UPDATE_CHECK", "1")
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("Installed the glasstarn command"),
        "{text}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    // npm clears its cache: the installed command keeps working.
    std::fs::remove_dir_all(dir.path().join("cache")).unwrap();
    let installed = prefix.join("lib/node_modules/glasstarn");
    assert!(
        !std::fs::symlink_metadata(&installed)
            .unwrap()
            .file_type()
            .is_symlink(),
        "a copy, not a link"
    );
    let version = Command::new(prefix.join("bin/glasstarn"))
        .arg("--version")
        .env("PATH", &path)
        .output()
        .unwrap();
    assert!(
        version.status.success(),
        "{}",
        String::from_utf8_lossy(&version.stderr)
    );
    assert!(String::from_utf8_lossy(&version.stdout).starts_with("glasstarn "));
}

fn copy(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}
