# Drifting VOID beta setup: copies the app to the user's programs folder,
# adds shortcuts, the .void file type and an uninstall entry, and fetches
# ffmpeg (video, sound) from GitHub when none is installed.
# Per-user install: no administrator rights needed.
param(
    [string]$InstallDir = (Join-Path $env:LOCALAPPDATA "Programs\DriftingVOID"),
    [switch]$NoShell,      # no shortcuts, file type or uninstall entry (tests)
    [switch]$SkipFfmpeg,
    [switch]$ForceFfmpeg,  # download even when ffmpeg is found
    [switch]$Quiet         # take the defaults instead of asking
)

$ErrorActionPreference = "Stop"
[Console]::OutputEncoding = [Text.Encoding]::UTF8
$src = Split-Path -Parent $PSScriptRoot
$exeName = "DriftingVOID.exe"
$appName = "Drifting VOID"
$ffRepo = "BtbN/FFmpeg-Builds"

function Say([string]$t, [string]$c = "Gray") { Write-Host $t -ForegroundColor $c }
function Step([string]$t) { Write-Host ""; Write-Host "  $t" -ForegroundColor Yellow }
function Ask([string]$q, [bool]$yes) {
    if ($Quiet) { return $yes }
    $hint = if ($yes) { "[Y/n]" } else { "[y/N]" }
    $a = Read-Host "  $q $hint"
    if ([string]::IsNullOrWhiteSpace($a)) { return $yes }
    return $a.Trim().ToLower().StartsWith("y")
}
function Works([string]$exe) {
    try {
        $p = Start-Process -FilePath $exe -ArgumentList "-version" -NoNewWindow -Wait -PassThru `
            -RedirectStandardOutput "$env:TEMP\dv-ff-out.txt" -RedirectStandardError "$env:TEMP\dv-ff-err.txt"
        return $p.ExitCode -eq 0
    } catch { return $false }
}

$exeSrc = Join-Path $src $exeName
if (-not (Test-Path $exeSrc)) { Say "  $exeName 없음: 압축을 모두 푼 폴더에서 Setup.cmd를 실행하세요." Red; exit 1 }
$ver = (Get-Item $exeSrc).VersionInfo.ProductVersion

Write-Host ""
Write-Host "  DRIFTING VOID  $ver  ·  클로즈 베타 설치" -ForegroundColor White
Say "  설치 위치  $InstallDir" DarkGray

# ---------------------------------------------------------------- the app
Step "1/3  프로그램 복사"
$running = Get-Process -Name "DriftingVOID" -ErrorAction SilentlyContinue |
    Where-Object { $_.Path -and $_.Path.StartsWith($InstallDir, [StringComparison]::OrdinalIgnoreCase) }
if ($running) {
    Say "  Drifting VOID가 실행 중입니다. 작업을 저장하고 닫은 뒤 계속하세요." Red
    if (-not $Quiet) { Read-Host "  닫았으면 Enter" | Out-Null }
    if (Get-Process -Id $running.Id -ErrorAction SilentlyContinue) { Say "  아직 실행 중이라 설치를 멈춥니다." Red; exit 1 }
}
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
Copy-Item $exeSrc (Join-Path $InstallDir $exeName) -Force
Copy-Item (Join-Path $PSScriptRoot "uninstall.ps1") (Join-Path $InstallDir "uninstall.ps1") -Force
$manualSrc = Join-Path $src "manual"
if (Test-Path $manualSrc) {
    $manualDst = Join-Path $InstallDir "manual"
    if (Test-Path $manualDst) { Remove-Item $manualDst -Recurse -Force }
    Copy-Item $manualSrc $manualDst -Recurse -Force
}
# files from a downloaded zip carry the web mark; the copies are trusted now
Get-ChildItem $InstallDir -Recurse -File | Unblock-File -ErrorAction SilentlyContinue
Say "  완료" Green

# ---------------------------------------------------------------- ffmpeg
Step "2/3  ffmpeg (영상·소리)"
$ffDir = Join-Path $InstallDir "ffmpeg"
$found = $null
if (-not $ForceFfmpeg) {
    $cands = @((Join-Path $ffDir "bin"), (Join-Path $env:APPDATA "DriftingVOID\ffmpeg\bin"), (Join-Path $env:APPDATA "DriftingVOID\ffmpeg"))
    foreach ($d in $cands) {
        $f = Join-Path $d "ffmpeg.exe"
        if ((Test-Path $f) -and (Test-Path (Join-Path $d "ffprobe.exe")) -and (Works $f)) { $found = $f; break }
    }
    if (-not $found) {
        $c = Get-Command ffmpeg -ErrorAction SilentlyContinue
        if ($c -and (Get-Command ffprobe -ErrorAction SilentlyContinue) -and (Works $c.Source)) { $found = $c.Source }
    }
}
if ($found) {
    Say "  이미 설치된 ffmpeg를 사용합니다: $found" Green
} elseif ($SkipFfmpeg) {
    Say "  건너뜀 (영상과 소리는 ffmpeg 설치 후 사용 가능)" DarkGray
} else {
    Say "  ffmpeg는 FFmpeg 프로젝트의 별도 프로그램(GPL)입니다." DarkGray
    Say "  GitHub의 $ffRepo 공개 빌드를 내려받아 설치 폴더에만 둡니다 (약 80 MB)." DarkGray
    if (Ask "내려받을까요?" $true) {
        $tmp = Join-Path $env:TEMP ("dv-ffmpeg-" + [Guid]::NewGuid().ToString("N").Substring(0, 8))
        try {
            [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
            Add-Type -AssemblyName System.Net.Http
            $http = New-Object System.Net.Http.HttpClient
            $http.DefaultRequestHeaders.UserAgent.ParseAdd("DriftingVOID-setup")
            $http.Timeout = [TimeSpan]::FromMinutes(30)

            # the newest numbered release (n9.0, n8.1, ...), GPL with shared libraries
            $rel = $http.GetStringAsync("https://api.github.com/repos/$ffRepo/releases/latest").Result | ConvertFrom-Json
            $pick = $rel.assets | Where-Object { $_.name -match '^ffmpeg-n(\d+)\.(\d+)-latest-win64-gpl-shared-[\d.]+\.zip$' } |
                Sort-Object { [int]($_.name -replace '^ffmpeg-n(\d+)\.(\d+).*', '$1') * 1000 + [int]($_.name -replace '^ffmpeg-n(\d+)\.(\d+).*', '$2') } -Descending |
                Select-Object -First 1
            if (-not $pick) { $pick = $rel.assets | Where-Object { $_.name -eq "ffmpeg-master-latest-win64-gpl-shared.zip" } | Select-Object -First 1 }
            if (-not $pick) { throw "릴리스에서 Windows 빌드를 찾지 못함" }

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
            if (-not $want) { throw "체크섬을 찾지 못함" }

            New-Item -ItemType Directory -Force -Path $tmp | Out-Null
            $zip = Join-Path $tmp $pick.name
            Say ("  {0}  ({1:N0} MB)" -f $pick.name, ($pick.size / 1MB)) DarkGray
            $resp = $http.GetAsync($pick.browser_download_url, [System.Net.Http.HttpCompletionOption]::ResponseHeadersRead).Result
            if (-not $resp.IsSuccessStatusCode) { throw "다운로드 실패: HTTP $([int]$resp.StatusCode)" }
            $total = $resp.Content.Headers.ContentLength
            $in = $resp.Content.ReadAsStreamAsync().Result
            $out = [IO.File]::Create($zip)
            $buf = New-Object byte[] (1MB)
            $done = 0L; $shown = -1
            try {
                while (($n = $in.Read($buf, 0, $buf.Length)) -gt 0) {
                    $out.Write($buf, 0, $n); $done += $n
                    $pct = if ($total) { [int](100 * $done / $total) } else { 0 }
                    if ($pct -ne $shown) {
                        $shown = $pct
                        Write-Progress -Activity "ffmpeg 내려받는 중" -Status ("{0:N0} / {1:N0} MB" -f ($done / 1MB), ($total / 1MB)) -PercentComplete $pct
                    }
                }
            } finally { $out.Close(); $in.Close() }
            Write-Progress -Activity "ffmpeg 내려받는 중" -Completed

            $got = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
            if ($got -ne $want) { throw "체크섬 불일치 (받은 파일 $got)" }
            Say "  체크섬 확인 (SHA-256)" Green

            Add-Type -AssemblyName System.IO.Compression.FileSystem
            $ex = Join-Path $tmp "x"
            [IO.Compression.ZipFile]::ExtractToDirectory($zip, $ex)
            $top = Get-ChildItem $ex -Directory | Select-Object -First 1
            if (-not $top -or -not (Test-Path (Join-Path $top.FullName "bin\ffmpeg.exe"))) { throw "압축 안에 bin\ffmpeg.exe 없음" }
            if (Test-Path $ffDir) { Remove-Item $ffDir -Recurse -Force }
            New-Item -ItemType Directory -Force -Path $ffDir | Out-Null
            Move-Item (Join-Path $top.FullName "bin") (Join-Path $ffDir "bin")
            Get-ChildItem $top.FullName -File | Move-Item -Destination $ffDir
            Set-Content -Path (Join-Path $ffDir "SOURCE.txt") -Encoding UTF8 -Value @(
                "FFmpeg (https://ffmpeg.org), GPL build by https://github.com/$ffRepo",
                "File: $($pick.name)",
                "SHA-256: $want",
                "Downloaded: $(Get-Date -Format 'yyyy-MM-dd HH:mm')",
                "License: see LICENSE.txt in this folder. Sources: https://github.com/$ffRepo and https://ffmpeg.org/download.html")
            if (-not (Works (Join-Path $ffDir "bin\ffmpeg.exe"))) { throw "설치한 ffmpeg가 실행되지 않음" }
            Say "  설치 완료: $ffDir" Green
        } catch {
            Say "  ffmpeg 설치 실패: $($_.Exception.Message)" Red
            Say "  앱은 그대로 쓸 수 있고, 영상과 소리만 빠집니다. 나중에 Setup.cmd를 다시 실행하거나" DarkGray
            Say "  winget install Gyan.FFmpeg 로 설치하세요." DarkGray
        } finally {
            if (Test-Path $tmp) { Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue }
        }
    } else {
        Say "  건너뜀 (영상과 소리는 ffmpeg 설치 후 사용 가능)" DarkGray
    }
}

# ---------------------------------------------------------------- shortcuts, file type, uninstall
Step "3/3  바로 가기 · .void 파일 연결"
$exe = Join-Path $InstallDir $exeName
if ($NoShell) {
    Say "  건너뜀 (-NoShell)" DarkGray
} else {
    $ws = New-Object -ComObject WScript.Shell
    function Link([string]$path, [string]$target, [string]$desc, [string]$icon) {
        $l = $ws.CreateShortcut($path)
        $l.TargetPath = $target; $l.WorkingDirectory = $InstallDir; $l.Description = $desc
        if ($icon) { $l.IconLocation = $icon }
        $l.Save()
    }
    $menu = Join-Path ([Environment]::GetFolderPath("Programs")) $appName
    New-Item -ItemType Directory -Force -Path $menu | Out-Null
    Link (Join-Path $menu "$appName.lnk") $exe $appName "$exe,0"
    $manual = Join-Path $InstallDir "manual\index.html"
    if (Test-Path $manual) { Link (Join-Path $menu "$appName 사용 설명서.lnk") $manual "$appName 사용 설명서" "$exe,0" }
    if (Ask "바탕 화면에 바로 가기를 만들까요?" $true) {
        Link (Join-Path ([Environment]::GetFolderPath("Desktop")) "$appName.lnk") $exe $appName "$exe,0"
    }

    # .void projects open in the app (this user only)
    $cls = "HKCU:\Software\Classes"
    New-Item -Force -Path "$cls\.void" | Out-Null
    Set-Item -Path "$cls\.void" -Value "DriftingVOID.Project"
    New-Item -Force -Path "$cls\DriftingVOID.Project\DefaultIcon" | Out-Null
    Set-Item -Path "$cls\DriftingVOID.Project" -Value "Drifting VOID 프로젝트"
    Set-Item -Path "$cls\DriftingVOID.Project\DefaultIcon" -Value "`"$exe`",0"
    New-Item -Force -Path "$cls\DriftingVOID.Project\shell\open\command" | Out-Null
    Set-Item -Path "$cls\DriftingVOID.Project\shell\open\command" -Value "`"$exe`" `"%1`""

    # Settings > Apps lists it, with its uninstaller
    $un = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\DriftingVOID"
    New-Item -Force -Path $un | Out-Null
    $kb = [int]((Get-ChildItem $InstallDir -Recurse -File | Measure-Object Length -Sum).Sum / 1KB)
    $props = @{
        DisplayName = $appName; DisplayVersion = $ver; Publisher = "Ash"
        InstallLocation = $InstallDir; DisplayIcon = "$exe,0"
        UninstallString = "powershell.exe -NoProfile -ExecutionPolicy Bypass -File `"$InstallDir\uninstall.ps1`""
    }
    foreach ($k in $props.Keys) { New-ItemProperty -Path $un -Name $k -Value $props[$k] -PropertyType String -Force | Out-Null }
    foreach ($k in @("NoModify", "NoRepair")) { New-ItemProperty -Path $un -Name $k -Value 1 -PropertyType DWord -Force | Out-Null }
    New-ItemProperty -Path $un -Name "EstimatedSize" -Value $kb -PropertyType DWord -Force | Out-Null

    Add-Type -Namespace DV -Name Shell -MemberDefinition '[DllImport("shell32.dll")] public static extern void SHChangeNotify(int e, uint f, IntPtr a, IntPtr b);'
    [DV.Shell]::SHChangeNotify(0x08000000, 0, [IntPtr]::Zero, [IntPtr]::Zero)
    Say "  시작 메뉴 · .void 연결 · 앱 목록(설정 > 앱) 등록 완료" Green
}

Write-Host ""
Write-Host "  설치가 끝났습니다." -ForegroundColor White
Say "  제거: 설정 > 앱 > Drifting VOID, 또는 $InstallDir\uninstall.ps1" DarkGray
if (-not $Quiet -and (Ask "지금 실행할까요?" $true)) { Start-Process $exe }
