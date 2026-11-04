# npm package

`packaging/npm/build.sh` builds `dist/npm/glasstarn-<version>.tgz` from this
computer's release build: a small Node launcher, the engine for this platform
under `native/<platform>-<arch>/`, and the agent plugin under `plugin/`. No
install scripts, no dependencies, nothing downloaded.

Install and connect agents the way a person would:

```sh
npm install -g dist/npm/glasstarn-1.0.1.tgz
glasstarn setup
```

`glasstarn setup` copies the plugin to `~/.glasstarn/agent-plugin` and
registers it with Codex and Claude Code when they are installed. Other agents
with local command and file access can use the shared skill too; see the
[plugin guide](../../plugins/glasstarn/README.md).

The GitHub Actions release workflow builds binaries for macOS arm64 and x64,
Linux arm64 and x64, and Windows x64. It assembles them into **one npm package**;
the launcher selects the binary for the current computer. The local build
script above includes only the current computer’s binary and is intended for
local testing.

The standalone app/installer scripts in the sibling packaging folders are
separate build paths; the npm release workflow does not produce those installers.

For an npm installation, update with `glasstarn update` and remove it with
`glasstarn uninstall`. Uninstall keeps projects and original exports by default.
