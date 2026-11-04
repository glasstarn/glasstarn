#!/bin/sh
set -eu

if [ "$#" -ne 2 ]; then
  echo "usage: notarize-app.sh <Glasstarn.app> <notary-keychain-profile>" >&2
  exit 2
fi

APP_PATH=$1
NOTARY_PROFILE=$2

case "$APP_PATH" in
  *.app) ;;
  *) echo "the first argument must be a .app bundle" >&2; exit 2 ;;
esac

if [ ! -d "$APP_PATH" ]; then
  echo "app bundle not found: $APP_PATH" >&2
  exit 2
fi

ARCHIVE_PATH="${APP_PATH%.app}.notarization.zip"
ditto -c -k --keepParent "$APP_PATH" "$ARCHIVE_PATH"
xcrun notarytool submit "$ARCHIVE_PATH" --keychain-profile "$NOTARY_PROFILE" --wait
xcrun stapler staple "$APP_PATH"
xcrun stapler validate "$APP_PATH"
spctl --assess --type execute --verbose "$APP_PATH"
rm -f -- "$ARCHIVE_PATH"
