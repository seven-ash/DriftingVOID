# Builds a package: target\dist\DriftingVOID-<name>(.zip) with the release exe,
# Setup.cmd, the setup scripts, the manual, the fonts and README.
# ffmpeg is not included: the setup fetches it from GitHub on the user's PC.
#
#   package.ps1 -Release 1.0.0.R   release package; the release version is
#                                  stamped into the exe and README and logged
#                                  in docs\RELEASES.md with the dev version
#   package.ps1                    dev package, named after Cargo.toml
param([string]$Release = "", [switch]$NoBuild)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
if ($Release -and $Release -notmatch '^[0-9A-Za-z][0-9A-Za-z._-]*$') { throw "bad release version: $Release" }
$dev = ((Get-Content Cargo.toml | Select-String '^version = "(.+)"').Matches[0].Groups[1].Value)

# build.rs reads DV_RELEASE: the exe's product version and the crash log
$env:DV_RELEASE = $Release
try {
    if (-not $NoBuild) {
        cargo build --release
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
    }
} finally { Remove-Item Env:DV_RELEASE -ErrorAction SilentlyContinue }
$exe = "target\release\DriftingVOID.exe"
$stamped = (Get-Item $exe).VersionInfo.ProductVersion
$want = if ($Release) { $Release } else { $dev }
if ($stamped -ne $want) { throw "exe says $stamped, expected $want (rebuild without -NoBuild)" }

$name = if ($Release) { "DriftingVOID-$Release" } else { "DriftingVOID-$dev-dev" }
$label = if ($Release) { $Release } else { "$dev-dev" }
$out = Join-Path $root "target\dist\$name"
if (Test-Path $out) { Remove-Item $out -Recurse -Force }
New-Item -ItemType Directory -Force -Path $out | Out-Null

Copy-Item $exe $out
Copy-Item "dist\Setup.cmd" $out
Copy-Item "dist\setup" (Join-Path $out "setup") -Recurse
Copy-Item "dist\manual" (Join-Path $out "manual") -Recurse
# the manual's cover names the version
$man = Join-Path $out "manual\index.html"
[IO.File]::WriteAllText($man, [IO.File]::ReadAllText($man, [Text.Encoding]::UTF8).Replace("{VERSION}", $label), (New-Object Text.UTF8Encoding $false))
# Japanese UI face (SIL OFL 1.1, its licence beside it)
$fonts = Join-Path $root "assets\fonts"
New-Item -ItemType Directory -Force (Join-Path $out "fonts") | Out-Null
Copy-Item (Join-Path $fonts "PretendardJP-*.ttf"), (Join-Path $fonts "OFL-PretendardJP.txt") (Join-Path $out "fonts")
$readme = [IO.File]::ReadAllText((Join-Path $root "dist\README.txt"), [Text.Encoding]::UTF8).Replace("{VERSION}", $label)
[IO.File]::WriteAllText((Join-Path $out "README.txt"), $readme, (New-Object Text.UTF8Encoding $true))

$zip = "$out.zip"
if (Test-Path $zip) { Remove-Item $zip -Force }
Compress-Archive -Path $out -DestinationPath $zip -CompressionLevel Optimal
"{0}  ({1:N1} MB)" -f $zip, ((Get-Item $zip).Length / 1MB)

if ($Release) {
    $commit = (git rev-parse --short HEAD).Trim()
    if (git status --porcelain --untracked-files=no) { $commit += " (uncommitted changes)" }
    $log = Join-Path $root "docs\RELEASES.md"
    $row = "| $Release | $dev | $commit | $(Get-Date -Format yyyy-MM-dd) |"
    $lines = [IO.File]::ReadAllLines($log, [Text.Encoding]::UTF8) | Where-Object { $_ -notmatch "^\| $([regex]::Escape($Release)) \|" }
    [IO.File]::WriteAllLines($log, [string[]](@($lines) + $row), (New-Object Text.UTF8Encoding $false))
    "logged: $row"
}
