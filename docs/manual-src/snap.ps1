param([string]$Exe, [string]$Arg, [string]$Out, [int]$Wait = 4, [string]$EnvName = "")
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System; using System.Runtime.InteropServices;
public static class Snap {
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
}
"@
if ($EnvName) { [Environment]::SetEnvironmentVariable($EnvName, "1", "Process") }
# start minimized-free but without activating: use WindowStyle Normal; focus may still move to the new window
$psi = New-Object System.Diagnostics.ProcessStartInfo $Exe
$psi.Arguments = $Arg
$psi.UseShellExecute = $false
$p = [System.Diagnostics.Process]::Start($psi)
Start-Sleep -Seconds $Wait
$p.Refresh(); $h = $p.MainWindowHandle
$r = New-Object Snap+RECT; [Snap]::GetWindowRect($h, [ref]$r) | Out-Null
$w = $r.R - $r.L; $hh = $r.B - $r.T
$bmp = New-Object System.Drawing.Bitmap $w, $hh
$g = [System.Drawing.Graphics]::FromImage($bmp); $hdc = $g.GetHdc()
[Snap]::PrintWindow($h, $hdc, 2) | Out-Null
$g.ReleaseHdc($hdc); $bmp.Save($Out)
$p.Kill()
"saved $w x $hh"
