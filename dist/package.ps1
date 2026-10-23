# Builds the closed-beta package: target\dist\DriftingVOID-<version>-beta(.zip)
# with the release exe, Setup.cmd, the setup scripts, the manual and README.
# ffmpeg is not included: the setup fetches it from GitHub on the tester's PC.
param([switch]$NoBuild)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
if (-not $NoBuild) {
    cargo build --release
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
}
$ver = ((Get-Content Cargo.toml | Select-String '^version = "(.+)"').Matches[0].Groups[1].Value)
$name = "DriftingVOID-$ver-beta"
$out = Join-Path $root "target\dist\$name"
if (Test-Path $out) { Remove-Item $out -Recurse -Force }
New-Item -ItemType Directory -Force -Path $out | Out-Null

Copy-Item "target\release\DriftingVOID.exe" $out
Copy-Item "dist\Setup.cmd" $out
Copy-Item "dist\setup" (Join-Path $out "setup") -Recurse
Copy-Item "dist\manual" (Join-Path $out "manual") -Recurse
Copy-Item "dist\README.txt" $out

$zip = "$out.zip"
if (Test-Path $zip) { Remove-Item $zip -Force }
Compress-Archive -Path $out -DestinationPath $zip -CompressionLevel Optimal
"{0}  ({1:N1} MB)" -f $zip, ((Get-Item $zip).Length / 1MB)
