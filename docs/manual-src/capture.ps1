param([string]$Only = "", [string]$Lang = "en")
# Full-screen captures for the user manual (English UI, a new user's settings
# from appdata\ plus the old manual's blue and text size). Writes
# target\manual-shots\man_<case>.png; crops.py turns them into dist\manual\img.
# No desktop input: the scenes come from DV_* flags, PrintWindow captures them.
$here = $PSScriptRoot
$root = (Resolve-Path "$here\..\..").Path
$sp = Join-Path $root "target\manual-shots"
New-Item -ItemType Directory -Force $sp | Out-Null
Copy-Item "$here\appdata" $sp -Recurse -Force
$exe = Join-Path $root "target\release\DriftingVOID.exe"
$room = Join-Path $root "tests\data\room.void"
$lines = "566.4,407.3,691.6,380.2,0;563.9,198.1,697.9,189.4,0;447.1,181.2,567.5,175.4,0;566.4,407.3,456.0,354.7,1;563.9,198.1,447.1,181.2,1;697.9,189.4,567.5,175.4,1;566.4,407.3,563.9,198.1,2;691.6,380.2,697.9,189.4,2;456.0,354.7,447.1,181.2,2"
function Clear-DV { Get-ChildItem Env: | Where-Object { $_.Name -like "DV_*" } | ForEach-Object { [Environment]::SetEnvironmentVariable($_.Name, $null, "Process") } }
$cases = [ordered]@{
  "overview" = @($room, "DV_SELECT_PHOTO=1")
  "light"    = @($room, "DV_SELECT_PHOTO=1|DV_THEME=0")
  "space"    = @($room, "DV_SPACE_LINES=$lines|DV_SPACE_DEMO=1|DV_THICK=30|DV_LIGHT=1")
  "lines"    = @($room, "DV_SPACE_LINES=0|DV_LINE_FIND=1")
  "ws3d"     = @($room, "DV_SPACE_LINES=$lines|DV_SPACE_DEMO=1|DV_THICK=30|DV_LIGHT=1|DV_WS3D=persp")
  "mask"     = @($room, "DV_SELECT_PHOTO=1|DV_MASK_DEMO=1")
  "palette"  = @($room, "DV_SHOW_PALETTE=1")
  "timeline" = @($room, "DV_TL_DEMO=1")
  "vexport"  = @($room, "DV_SHOW_VEXPORT=1|DV_EXPORT_FRAME=0,0,1280,720")
  "fx"       = @($room, "DV_FX=13,0.8")
  "text"     = @("", "DV_TEST_TEXT_FONT=Malgun Gothic|DV_TEST_TEXT_LOOK=1")
}
$appdata = $env:APPDATA
foreach ($c in $cases.Keys) {
  if ($Only -and ($Only -split ',') -notcontains $c) { continue }
  Clear-DV
  $envs = "DV_FULLSCREEN=1|DV_LANG=$Lang|DV_NOSAVE=1|DV_THEME=1|" + $cases[$c][1]
  foreach ($kv in ($envs -split '\|')) { $k, $v = $kv -split '=', 2; [Environment]::SetEnvironmentVariable($k, $v, "Process") }
  [Environment]::SetEnvironmentVariable("APPDATA", "$sp\appdata", "Process")
  $arg = if ($cases[$c][0]) { "`"$($cases[$c][0])`"" } else { "" }
  & "$here\snap.ps1" -Exe $exe -Arg $arg -Out "$sp\man_$c.png" -Wait 8 | Out-Null
  "== $c"
}
Clear-DV
[Environment]::SetEnvironmentVariable("APPDATA", $appdata, "Process")
