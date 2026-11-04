"""Writes THIRD_PARTY_NOTICES.txt: every crate compiled into the glasstarn
program, with its licence and the licence texts it ships (MIT and BSD ask for
their notices to travel with binaries). Run from the repository root:

    python3 packaging/notices/generate.py
"""
import json
import pathlib
import subprocess

meta = json.loads(subprocess.check_output(
    ["cargo", "metadata", "--format-version", "1", "--locked", "--filter-platform", "aarch64-apple-darwin"]))
packages = {p["id"]: p for p in meta["packages"]}
nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
workspace = set(meta["workspace_members"])

# Crates linked into pm-cli: follow normal and build dependencies, not dev ones.
root = next(i for i in workspace if packages[i]["name"] == "pm-cli")
seen, todo = set(), [root]
while todo:
    current = todo.pop()
    if current in seen:
        continue
    seen.add(current)
    for dep in nodes[current]["deps"]:
        if any(kind["kind"] in (None, "build") for kind in dep["dep_kinds"]):
            todo.append(dep["pkg"])

out = ["Third-party software in glasstarn",
       "=================================", "",
       "glasstarn is licensed under the Apache License 2.0 (see LICENSE).",
       "It includes the following open-source components. Fonts: see FONTS-OFL.txt.", ""]
for package in sorted((packages[i] for i in seen if i not in workspace), key=lambda p: (p["name"], p["version"])):
    folder = pathlib.Path(package["manifest_path"]).parent
    texts = sorted(f for f in folder.iterdir()
                   if f.is_file() and f.name.upper().startswith(("LICENSE", "LICENCE", "COPYING", "NOTICE", "COPYRIGHT")))
    out += ["-" * 72, f"{package['name']} {package['version']}",
            f"Licence: {package.get('license') or package.get('license_file') or 'see below'}"]
    if package.get("repository"):
        out.append(f"Source: {package['repository']}")
    for text in texts:
        out += ["", f"[{text.name}]", text.read_text(errors="replace").strip()]
    out.append("")
pathlib.Path("THIRD_PARTY_NOTICES.txt").write_text("\n".join(out) + "\n")
print(f"{len(seen) - 1} crates written to THIRD_PARTY_NOTICES.txt")
