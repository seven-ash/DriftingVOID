# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Ash (seven-ash). Additional terms (GPL-3.0 section 7): see NOTICE.
# FFmpeg for the Inno Setup installer (DriftingVOID.iss), run hidden:
#   -Resolve -Out <file>: picks the newest numbered GPL shared build of
#     BtbN/FFmpeg-Builds and writes "url|sha256|name|bytes" (or "error|...")
#   -Install -Zip <zip> -Name <asset> -Sum <sha256> -Dir <app>\ffmpeg -Out <file>:
#     unpacks it there with SOURCE.txt and checks that ffmpeg runs; writes
#     "ok" or "error|..."
# The download itself is Inno Setup's (progress and the SHA-256 check).
param(
    [switch]$Resolve,
    [switch]$Install,
    [string]$Out,
    [string]$Zip,
    [string]$Name,
    [string]$Sum,
    [string]$Dir
)

$ErrorActionPreference = "Stop"
$ffRepo = "BtbN/FFmpeg-Builds"

function Put([string]$t) { [IO.File]::WriteAllText($Out, $t, (New-Object Text.UTF8Encoding $false)) }

try {
    if ($Resolve) {
        [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
        Add-Type -AssemblyName System.Net.Http
        $http = New-Object System.Net.Http.HttpClient
        $http.DefaultRequestHeaders.UserAgent.ParseAdd("DriftingVOID-setup")
        $http.Timeout = [TimeSpan]::FromMinutes(2)
        $rel = $http.GetStringAsync("https://api.github.com/repos/$ffRepo/releases/latest").Result | ConvertFrom-Json
        # the newest numbered release (n8.1, n8.0, ...), GPL with shared libraries
        $pick = $rel.assets | Where-Object { $_.name -match '^ffmpeg-n(\d+)\.(\d+)-latest-win64-gpl-shared-[\d.]+\.zip$' } |
            Sort-Object { [int]($_.name -replace '^ffmpeg-n(\d+)\.(\d+).*', '$1') * 1000 + [int]($_.name -replace '^ffmpeg-n(\d+)\.(\d+).*', '$2') } -Descending |
            Select-Object -First 1
        if (-not $pick) { $pick = $rel.assets | Where-Object { $_.name -eq "ffmpeg-master-latest-win64-gpl-shared.zip" } | Select-Object -First 1 }
        if (-not $pick) { throw "no Windows build in the latest release" }
        # the checksum GitHub keeps for the asset, else the release's checksums file
        $want = $null
        if ($pick.PSObject.Properties["digest"] -and $pick.digest -match '^sha256:([0-9a-f]{64})$') { $want = $Matches[1] }
        if (-not $want) {
            $sums = $rel.assets | Where-Object { $_.name -eq "checksums.sha256" } | Select-Object -First 1
            if ($sums) {
                $line = ($http.GetStringAsync($sums.browser_download_url).Result -split "`n") | Where-Object { $_ -match [regex]::Escape($pick.name) } | Select-Object -First 1
                if ($line -match '^([0-9a-fA-F]{64})') { $want = $Matches[1].ToLower() }
            }
        }
        if (-not $want) { throw "no checksum published" }
        Put ("{0}|{1}|{2}|{3}" -f $pick.browser_download_url, $want, $pick.name, $pick.size)
    } elseif ($Install) {
        $got = (Get-FileHash $Zip -Algorithm SHA256).Hash.ToLower()
        if ($got -ne $Sum.ToLower()) { throw "checksum mismatch: $got" }
        $tmp = Join-Path $env:TEMP ("dv-ffmpeg-" + [Guid]::NewGuid().ToString("N").Substring(0, 8))
        try {
            Add-Type -AssemblyName System.IO.Compression.FileSystem
            [IO.Compression.ZipFile]::ExtractToDirectory($Zip, $tmp)
            $top = Get-ChildItem $tmp -Directory | Select-Object -First 1
            if (-not $top -or -not (Test-Path (Join-Path $top.FullName "bin\ffmpeg.exe"))) { throw "no ffmpeg.exe in the archive" }
            if (Test-Path $Dir) { Remove-Item $Dir -Recurse -Force }
            New-Item -ItemType Directory -Force -Path $Dir | Out-Null
            Move-Item (Join-Path $top.FullName "bin") (Join-Path $Dir "bin")
            Get-ChildItem $top.FullName -File | Move-Item -Destination $Dir
        } finally {
            if (Test-Path $tmp) { Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue }
        }
        Set-Content -Path (Join-Path $Dir "SOURCE.txt") -Encoding UTF8 -Value @(
            "FFmpeg (https://ffmpeg.org), GPL build by https://github.com/$ffRepo",
            "File: $Name",
            "SHA-256: $Sum",
            "Downloaded: $(Get-Date -Format 'yyyy-MM-dd HH:mm')",
            "License: see LICENSE.txt in this folder. Sources: https://github.com/$ffRepo and https://ffmpeg.org/download.html")
        $p = Start-Process -FilePath (Join-Path $Dir "bin\ffmpeg.exe") -ArgumentList "-version" -NoNewWindow -Wait -PassThru `
            -RedirectStandardOutput "$env:TEMP\dv-ff-out.txt" -RedirectStandardError "$env:TEMP\dv-ff-err.txt"
        if ($p.ExitCode -ne 0) { throw "ffmpeg does not run" }
        Put "ok"
    }
} catch {
    Put ("error|" + $_.Exception.Message)
    exit 1
}
