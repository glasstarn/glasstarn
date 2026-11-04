param(
    [string]$Version = $(if ($env:APP_VERSION) { $env:APP_VERSION } else { "0.1.0" })
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

if ($env:OS -ne "Windows_NT") {
    throw "Windows packages must be built and verified on Windows."
}

$ProjectRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$OutputRoot = Join-Path $ProjectRoot "dist\windows"
$Architecture = $env:PROCESSOR_ARCHITECTURE.ToLowerInvariant()
$PackageName = "glasstarn-$Version-windows-$Architecture"
$ArchivePath = Join-Path $OutputRoot "$PackageName.zip"
$TemporaryRoot = Join-Path ([IO.Path]::GetTempPath()) ("open-process-" + [Guid]::NewGuid().ToString("N"))
$Stage = Join-Path $TemporaryRoot $PackageName

try {
    & cargo build --manifest-path (Join-Path $ProjectRoot "Cargo.toml") --release --bin glasstarn
    if ($LASTEXITCODE -ne 0) { throw "The release build failed." }

    New-Item -ItemType Directory -Force -Path $Stage | Out-Null
    New-Item -ItemType Directory -Force -Path $OutputRoot | Out-Null
    $Binary = Join-Path $ProjectRoot "target\release\glasstarn.exe"
    $StagedBinary = Join-Path $Stage "glasstarn.exe"
    Copy-Item -LiteralPath $Binary -Destination $StagedBinary
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot "install.ps1") -Destination $Stage
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot "README.md") -Destination $Stage
    Copy-Item -LiteralPath (Join-Path $ProjectRoot "LICENSE") -Destination $Stage
    Copy-Item -LiteralPath (Join-Path $ProjectRoot "ui\fonts\OFL.txt") -Destination (Join-Path $Stage "FONTS-OFL.txt")

    if ($env:SIGNING_CERT_THUMBPRINT) {
        $CertificatePath = "Cert:\CurrentUser\My\$($env:SIGNING_CERT_THUMBPRINT)"
        $Certificate = Get-Item -LiteralPath $CertificatePath
        $Signature = Set-AuthenticodeSignature `
            -FilePath $StagedBinary `
            -Certificate $Certificate `
            -TimestampServer "http://timestamp.digicert.com" `
            -HashAlgorithm SHA256
        if ($Signature.Status -ne "Valid") {
            throw "Authenticode signing failed: $($Signature.StatusMessage)"
        }
    }

    & $StagedBinary help | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "The packaged executable did not start." }

    Compress-Archive -LiteralPath $Stage -DestinationPath $ArchivePath -CompressionLevel Optimal -Force
    Write-Output $ArchivePath
}
finally {
    if (Test-Path -LiteralPath $TemporaryRoot) {
        Remove-Item -LiteralPath $TemporaryRoot -Recurse -Force
    }
}
