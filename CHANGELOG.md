# Changelog

Releases of Drifting VOID. Each release has its installer on the
[releases page](https://github.com/seven-ash/DriftingVOID/releases).

## 1.1.0.R — 2026-11-12

### Licence
- From this release Drifting VOID is under the **GNU Affero General Public
  License, version 3 or later**, with the same attribution terms
  (`NOTICE`). Releases up to 1.0.1.R were published under the GNU GPL,
  version 3 or later, and stay available under it.
- The notices now name the code this program's source follows: the LSD line
  segment detector (AGPL), the Oklab and Okhsv colour conversions and one
  shader function (MIT), and two formulas from GIMP (GPL).

### Added
- **Recovery copies.** Unsaved changes are copied every 30 seconds. After a
  crash or a power cut, the next start offers to recover them.
- **FX layers remember each filter.** Changing a layer's filter keeps the
  old filter's settings, curves and keys, and brings them back when the
  layer returns to it.
- **A looping video's sound loops** with its picture, on the timeline and
  in exported video.
- **Text has its own colour.** The text tool keeps the last colour a text
  was given; changing the main colour no longer recolours a selected text.
  The tool's options show the colour.
- **Pretendard JP is the default font** for new text (Korean, Japanese and
  Latin; it ships with the app). Text in existing projects keeps its font.
- **Locked layers** show their properties locked: dials in the accent
  colour, fields with an accent edge, and no input taken.
- After an update the project that was open is opened again.

### Changed
- **Undo** no longer moves the playhead or switches the timeline. It also
  leaves a video's preview position, loop switch and speed and a space's
  grid switch alone.
- A copy run from an unzipped folder does not check for updates; only an
  installed copy can update itself.
- A text's outline, border and box colours come back as they were when
  switched off and on.
- The palette's hue bar starts at pure red.
- Project files state their format version; a project made by a newer
  version says so instead of failing with an unclear error.
- Projects are written through a temporary file, so a failed save cannot
  damage the existing file.

### Fixed
- A second close request closed the app without asking about unsaved
  changes.
- Dials changed a value that was outside their range just by showing it; a
  text turned past 180° could jump.
- With no GPU path available, exports left FX layers out without saying
  so. They now refuse and say why.
- The playback cache missed edits to FX curves, the grain switch, the
  alpha source and pen strokes changed inside the same bounds.
- A 3D space that follows a photo did not follow it in exported video.
- Stacked FX layers became slow very quickly (nine layers: about 77 ms a
  frame, now about 3 ms).
- A missing video file started FFmpeg hundreds of times; a sound device
  that went away stopped timeline playback; an update that could not start
  still closed the app.
- Crashes: pasting an address with Korean text in it, trimming a sound
  clip of zero length.
- Keys dragged together could change order or merge; the eraser missed on
  a drawing placed in a 3D space; a deleted space left its layers marked
  as placed; one wheel notch moved a dial many steps.
- New and Open left parts of the previous document behind (selection,
  cameras, playback).
- A sound still decoding was left out of an exported video; an export
  whose video file was missing ended as if it had worked.

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
- **The manual online** at
  [seven-ash.github.io/DriftingVOID](https://seven-ash.github.io/DriftingVOID/).

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
