#!/usr/bin/env node
// Runs the glasstarn engine built for this computer. The package carries one
// executable per platform under native/ and the agent plugin under plugin/;
// there are no install scripts and nothing is downloaded.
"use strict";
const { spawnSync } = require("node:child_process");
const path = require("node:path");
const fs = require("node:fs");

const root = path.join(__dirname, "..");
const name = process.platform === "win32" ? "glasstarn.exe" : "glasstarn";
const binary = path.join(root, "native", `${process.platform}-${process.arch}`, name);
if (!fs.existsSync(binary)) {
  console.error(`glasstarn is not available for ${process.platform}-${process.arch} in this package yet.`);
  process.exit(1);
}
const result = spawnSync(binary, process.argv.slice(2), {
  stdio: "inherit",
  env: { ...process.env, GLASSTARN_PLUGIN_DIR: path.join(root, "plugin") },
});
if (result.error) {
  console.error(result.error.message);
  process.exit(1);
}
process.exit(result.status ?? 1);
