# Changelog

Releases of Drifting VOID. Each release has its installer on the
[releases page](https://github.com/seven-ash/DriftingVOID/releases).

## 1.0.1.R — 2026-11-02

The first public release.

### Added
- **Updates inside the app.** A newer release shows as a quiet notice right
  of the menus. A click downloads its installer and checks it, then
  updates and reopens the app. It can be switched off in
  *Settings → Check for updates*.
- **Curve editor** for the Curves filter: points on a graph per channel
  (RGB, R, G, B). Right-click a point to set its left, right or both
  tangents to Constant, Linear, Free or Auto. Free tangents have handles.
- **Channel to alpha**, a new FX filter: R, G, B or luminance, through its
  own curve, becomes the alpha.
- **Grain** can change every frame.
- **Vignette and colour fringes** follow the camera being filmed and turn
  with it.
- **Installer** for English, Korean and Japanese. It installs per user,
  without administrator rights, and can download FFmpeg with a checksum.

### Changed
- Drifting VOID is free software under the GPL-3.0-or-later, with
  attribution terms (`NOTICE`). Third-party notices are included.
- Curves from earlier projects become points on the graph.

## 1.0.0.R — 2026-11-01

The first release, shared privately as a beta.
- Infinite canvas for photos at their full resolution.
- Raster brush and vector pen with pressure and smoothing; text, shapes
  and an OKHSV palette.
- Masks: painting, region picking, GrabCut and intelligent scissors.
- FX layers with 18 filters, all values keyable.
- 3D spaces matched to photos from vanishing lines, with a 3D workspace.
- Timeline with keyframes, canvas cameras, video and sound, and recorded
  strokes.
- Picture and video export on the GPU through FFmpeg.
- Interface and manual in Korean, English and Japanese.
