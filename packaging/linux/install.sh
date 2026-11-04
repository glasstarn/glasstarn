#!/bin/sh
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
BIN_HOME=${XDG_BIN_HOME:-"$HOME/.local/bin"}
DATA_HOME=${XDG_DATA_HOME:-"$HOME/.local/share"}
APPLICATIONS_HOME="$DATA_HOME/applications"
TARGET_BINARY="$BIN_HOME/glasstarn"
TARGET_DESKTOP="$APPLICATIONS_HOME/glasstarn.desktop"

case "$TARGET_BINARY" in
  *'"'*|*'
'*) echo "the installation path contains unsupported characters" >&2; exit 2 ;;
esac
ESCAPED_BINARY=$(printf '%s' "$TARGET_BINARY" | sed 's/[&|\\]/\\&/g')

mkdir -p "$BIN_HOME" "$APPLICATIONS_HOME"
cp "$SCRIPT_DIR/bin/glasstarn" "$TARGET_BINARY"
chmod 755 "$TARGET_BINARY"
sed "s|@EXECUTABLE@|$ESCAPED_BINARY|g" \
  "$SCRIPT_DIR/share/applications/glasstarn.desktop.in" \
  >"$TARGET_DESKTOP"
chmod 644 "$TARGET_DESKTOP"

if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$APPLICATIONS_HOME" >/dev/null 2>&1 || true
fi

echo "Installed Glasstarn for the current user."
echo "Launch it from the application menu or run: $TARGET_BINARY"
