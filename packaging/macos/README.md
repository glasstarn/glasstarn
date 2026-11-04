# macOS packaging

Build a native application bundle from the current source tree:

```sh
packaging/macos/build-app.sh
```

The bundle is written to `dist/macos/Glasstarn.app`. It contains the
same local Rust service and embedded browser interface as the CLI. Opening the
application starts the loopback-only service and opens its authenticated URL in
the user's default browser. The app can run from `~/Applications`, so an
administrator account is not required.

Without configuration, the build is ad-hoc signed for local testing. For a
release build, provide a Developer ID Application identity already installed in
the macOS keychain:

```sh
SIGNING_IDENTITY="Developer ID Application: Example (TEAMID)" \
APP_VERSION=1.0.0 BUILD_NUMBER=1 \
packaging/macos/build-app.sh
```

Before notarization, store credentials in the keychain with Apple's
`notarytool store-credentials`. Then submit and staple the signed app without
putting credentials on the command line:

```sh
packaging/macos/notarize-app.sh \
  "dist/macos/Glasstarn.app" \
  open-process-notary
```

Notarization requires an Apple Developer account and cannot be meaningfully
validated by an unsigned development build.
