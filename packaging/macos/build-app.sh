#!/bin/sh
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
PROJECT_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/../.." && pwd)
BUILD_ROOT=${GLASSTARN_BUILD_ROOT:-"$PROJECT_ROOT/target/package-macos"}
OUTPUT_ROOT="$PROJECT_ROOT/dist/macos"
APP_VERSION=${APP_VERSION:-0.1.0}
BUILD_NUMBER=${BUILD_NUMBER:-1}
APP_PATH="$OUTPUT_ROOT/Glasstarn.app"

cargo build \
  --manifest-path "$PROJECT_ROOT/Cargo.toml" \
  --release \
  --bin glasstarn \
  --target-dir "$BUILD_ROOT"

rm -rf -- "$APP_PATH"
mkdir -p "$APP_PATH/Contents/MacOS" "$APP_PATH/Contents/Resources"
cp "$SCRIPT_DIR/Info.plist" "$APP_PATH/Contents/Info.plist"
cp "$BUILD_ROOT/release/glasstarn" "$APP_PATH/Contents/MacOS/glasstarn"
chmod 755 "$APP_PATH/Contents/MacOS/glasstarn"
cp "$PROJECT_ROOT/LICENSE" "$APP_PATH/Contents/Resources/LICENSE"
cp "$PROJECT_ROOT/ui/fonts/OFL.txt" "$APP_PATH/Contents/Resources/FONTS-OFL.txt"

plutil -replace CFBundleShortVersionString -string "$APP_VERSION" "$APP_PATH/Contents/Info.plist"
plutil -replace CFBundleVersion -string "$BUILD_NUMBER" "$APP_PATH/Contents/Info.plist"

if [ -n "${SIGNING_IDENTITY:-}" ]; then
  codesign --force --options runtime --timestamp --sign "$SIGNING_IDENTITY" "$APP_PATH"
else
  codesign --force --sign - "$APP_PATH"
fi

codesign --verify --deep --strict "$APP_PATH"
plutil -lint "$APP_PATH/Contents/Info.plist"
echo "$APP_PATH"
