#!/bin/sh
# Builds the npm package from this computer's release build:
#   dist/npm/glasstarn-<version>.tgz
# Install it the way a person would: npm install -g dist/npm/glasstarn-*.tgz
set -eu
repo=$(cd "$(dirname "$0")/../.." && pwd)
cd "$repo"
cargo build --release -p pm-cli
stage=$(mktemp -d)
cp -R packaging/npm/package.json packaging/npm/bin "$stage/"
platform=$(node -p 'process.platform + "-" + process.arch')
mkdir -p "$stage/native/$platform"
cp target/release/glasstarn "$stage/native/$platform/"
mkdir -p "$stage/plugin"
cp -R plugins/glasstarn plugins/.agents plugins/.claude-plugin "$stage/plugin/"
find "$stage" -name .DS_Store -delete
cp LICENSE THIRD_PARTY_NOTICES.txt "$stage/"
cp ui/fonts/OFL.txt "$stage/FONTS-OFL.txt"
cp packaging/npm/PACKAGE_README.md "$stage/README.md"
mkdir -p dist/npm
(cd "$stage" && npm pack --silent --pack-destination "$repo/dist/npm")
rm -rf "$stage"
ls -1 dist/npm/glasstarn-*.tgz
