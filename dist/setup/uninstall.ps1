# Drifting VOID beta uninstall: removes the install folder (with its ffmpeg),
# shortcuts, the .void file type and the uninstall entry. Settings in
# %APPDATA%\DriftingVOID stay unless asked. Projects (.void files) are never touched.
param([switch]$Quiet)

$ErrorActionPreference = "Stop"
[Console]::OutputEncoding = [Text.Encoding]::UTF8
$dir = $PSScriptRoot
$appName = "Drifting VOID"

function Ask([string]$q, [bool]$yes) {
    if ($Quiet) { return $yes }
    $hint = if ($yes) { "[Y/n]" } else { "[y/N]" }
    $a = Read-Host "  $q $hint"
    if ([string]::IsNullOrWhiteSpace($a)) { return $yes }
    return $a.Trim().ToLower().StartsWith("y")
}

Write-Host ""
Write-Host "  DRIFTING VOID  ·  제거" -ForegroundColor White
Write-Host "  $dir" -ForegroundColor DarkGray
Write-Host "  .void 프로젝트 파일은 지우지 않습니다." -ForegroundColor DarkGray
if (-not $Quiet -and -not (Ask "제거할까요?" $false)) { exit 0 }

$running = Get-Process -Name "DriftingVOID" -ErrorAction SilentlyContinue |
    Where-Object { $_.Path -and $_.Path.StartsWith($dir, [StringComparison]::OrdinalIgnoreCase) }
if ($running) {
    Write-Host "  Drifting VOID가 실행 중입니다. 저장하고 닫은 뒤 다시 실행하세요." -ForegroundColor Red
    if (-not $Quiet) { Read-Host "  Enter" | Out-Null }
    exit 1
}

$menu = Join-Path ([Environment]::GetFolderPath("Programs")) $appName
if (Test-Path $menu) { Remove-Item $menu -Recurse -Force }
$desk = Join-Path ([Environment]::GetFolderPath("Desktop")) "$appName.lnk"
if (Test-Path $desk) { Remove-Item $desk -Force }

# the file type only when it still points here
$cmd = "HKCU:\Software\Classes\DriftingVOID.Project\shell\open\command"
if ((Test-Path $cmd) -and ((Get-Item $cmd).GetValue("") -like "*$dir*")) {
    Remove-Item "HKCU:\Software\Classes\DriftingVOID.Project" -Recurse -Force
    if ((Test-Path "HKCU:\Software\Classes\.void") -and (Get-Item "HKCU:\Software\Classes\.void").GetValue("") -eq "DriftingVOID.Project") {
        Remove-Item "HKCU:\Software\Classes\.void" -Recurse -Force
    }
}
$un = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\DriftingVOID"
if (Test-Path $un) { Remove-Item $un -Recurse -Force }

$data = Join-Path $env:APPDATA "DriftingVOID"
if ((Test-Path $data) -and (Ask "설정과 오류 기록($data)도 지울까요?" $false)) { Remove-Item $data -Recurse -Force }

# this script lives in the folder it removes: step out first
Set-Location $env:TEMP
Remove-Item $dir -Recurse -Force
Write-Host "  제거했습니다." -ForegroundColor Green
if (-not $Quiet) { Read-Host "  Enter로 닫기" | Out-Null }
