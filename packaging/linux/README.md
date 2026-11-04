# Linux packaging

Build and verify the current architecture's release tarball on Linux:

```sh
packaging/linux/build-package.sh
```

The script refuses to package on another operating system, preventing a binary
for the wrong platform from being labelled as Linux. The output is written to
`dist/linux/` and contains the release binary, desktop-entry template, and a
per-user installer.

After extracting the archive, run:

```sh
./install.sh
```

It installs under `$HOME/.local` (or the matching XDG locations), requires no
administrator access, and adds Glasstarn to the desktop application
menu. Launching it starts the loopback-only service and opens the authenticated
browser interface.

Build separate archives on x86-64 and ARM64 Linux. Distribution signing is a
release-channel concern; publish hashes and signatures beside each immutable
archive rather than embedding a private key in this repository.
