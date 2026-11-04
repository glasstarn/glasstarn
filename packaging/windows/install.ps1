$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$InstallRoot = Join-Path $env:LOCALAPPDATA "Glasstarn"
$TargetBinary = Join-Path $InstallRoot "glasstarn.exe"
$StartMenu = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs"
$ShortcutPath = Join-Path $StartMenu "Glasstarn.lnk"
$SourceBinary = Join-Path $PSScriptRoot "glasstarn.exe"

if (-not (Test-Path -LiteralPath $SourceBinary -PathType Leaf)) {
    throw "The package does not contain glasstarn.exe."
}

New-Item -ItemType Directory -Force -Path $InstallRoot | Out-Null
New-Item -ItemType Directory -Force -Path $StartMenu | Out-Null
Copy-Item -LiteralPath $SourceBinary -Destination $TargetBinary -Force

$Shell = New-Object -ComObject WScript.Shell
$Shortcut = $Shell.CreateShortcut($ShortcutPath)
$Shortcut.TargetPath = $TargetBinary
$Shortcut.WorkingDirectory = $InstallRoot
$Shortcut.Description = "Glasstarn"
$Shortcut.Save()

Write-Host "Installed Glasstarn for the current user."
Write-Host "Launch it from the Start Menu or run: $TargetBinary"
