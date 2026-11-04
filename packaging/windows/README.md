# Windows packaging

Build and verify the current architecture's release ZIP in PowerShell on
Windows:

```powershell
packaging\windows\build-package.ps1
```

The builder refuses to package on another operating system. The output under
`dist\windows` contains the release executable and a per-user installer. After
extracting the ZIP, run:

```powershell
.\install.ps1
```

It installs below `%LOCALAPPDATA%`, adds a Start Menu shortcut, and does not
request administrator access. The executable starts the loopback-only service
and opens its authenticated URL in the default browser.

For release signing, place the code-signing certificate in the current user's
certificate store and provide only its thumbprint:

```powershell
$env:SIGNING_CERT_THUMBPRINT = "0123456789ABCDEF..."
packaging\windows\build-package.ps1
```

The private key remains in the certificate store. The builder applies a SHA-256
Authenticode signature with a timestamp and rejects any signing result that is
not valid. Build separate archives on x64 and ARM64 Windows runners.
