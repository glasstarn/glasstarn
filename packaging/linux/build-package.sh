#!/bin/sh
set -eu

if [ "$(uname -s)" != "Linux" ]; then
  echo "Linux packages must be built and verified on Linux." >&2
  exit 2
fi

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
PROJECT_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/../.." && pwd)
VERSION=${APP_VERSION:-0.1.0}
ARCH=$(uname -m)
OUTPUT_ROOT="$PROJECT_ROOT/dist/linux"
PACKAGE_NAME="glasstarn-$VERSION-linux-$ARCH"
TEMP_ROOT=$(mktemp -d)
STAGE="$TEMP_ROOT/$PACKAGE_NAME"

cleanup() {
  rm -rf -- "$TEMP_ROOT"
}
trap cleanup EXIT HUP INT TERM

cargo build --manifest-path "$PROJECT_ROOT/Cargo.toml" --release --bin glasstarn

mkdir -p "$STAGE/bin" "$STAGE/share/applications" "$OUTPUT_ROOT"
cp "$PROJECT_ROOT/target/release/glasstarn" "$STAGE/bin/glasstarn"
cp "$SCRIPT_DIR/glasstarn.desktop.in" "$STAGE/share/applications/"
cp "$SCRIPT_DIR/install.sh" "$STAGE/install.sh"
cp "$SCRIPT_DIR/README.md" "$STAGE/README.md"
cp "$PROJECT_ROOT/LICENSE" "$STAGE/LICENSE"
cp "$PROJECT_ROOT/ui/fonts/OFL.txt" "$STAGE/FONTS-OFL.txt"
chmod 755 "$STAGE/bin/glasstarn" "$STAGE/install.sh"

"$STAGE/bin/glasstarn" help >/dev/null
if command -v desktop-file-validate >/dev/null 2>&1; then
  desktop-file-validate "$STAGE/share/applications/glasstarn.desktop.in"
fi

tar -C "$TEMP_ROOT" -czf "$OUTPUT_ROOT/$PACKAGE_NAME.tar.gz" "$PACKAGE_NAME"
echo "$OUTPUT_ROOT/$PACKAGE_NAME.tar.gz"
