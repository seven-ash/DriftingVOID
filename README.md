# Drifting VOID

An infinite-canvas photo collage and drawing app for Windows, built for pen tablets
(tested with Super Display + S Pen). Drop photos in at their original resolution,
scribble around and over them with a pressure-sensitive pen or brush, add notes,
and export just the part you want.

## Build and run

```bash
cargo build --release
```

The executable is `target/release/DriftingVOID.exe`. Opening a `.void` file passes
it as the first argument.

Requires a Rust toolchain with MSVC on Windows. Rendering uses OpenGL (egui/glow,
4x MSAA). The frosted glass falls back to flat cards if shaders fail to compile.

## Using it

| Input | Action |
| --- | --- |
| Drag files / Ctrl+V | Import photos (or paste text as a note) |
| Wheel | Zoom at cursor |
| Middle drag / pen button / Space+drag | Pan |
| Shift + middle drag | Zoom by dragging |
| Ctrl + drag (brush, pen, eraser, shapes) | Change size / stroke width |
| Alt + click | OKHSV colour palette |
| Shift + drag (select tool) | Box selection by centre (Shift+Ctrl: entirely inside); pen strokes and brush pixels are picked individually; Shift+click toggles |
| Group handles (several selected) | Scale / rotate about the common centre (Shift/Ctrl: 15° from start, Alt: absolute 15°) |
| Layer list | Drag to reorder, double-click to rename |
| Number fields | Accept maths: `40*1.5`, `2^3+1` |
| Dials | Drag the tick ring (Shift: fine, wheel: one step); ↺ resets to the default |
| Tab | Tap: switch 2D ↔ 3D workspace (needs a solvable 3D space). Hold: peek at the other one |
| Ctrl+H | Hide/show the UI |
| F12 | Input log (pen flags, mouse/touch events) |

Tools: V select, Q lasso, H pan, B brush (raster), P pen (vector), E eraser, T text,
L line, A arrow, R rectangle, O ellipse, C crop, F export frame, K 3D space
(perspective match from vanishing lines with a magnifier, optional mask; see `docs/SPACES.md`), I eyedropper.

Drawing layers are created when you first draw, reused afterwards, and removed
when empty. Photos are placed under drawing layers.

## Video

Drop a video (mp4, mov, mkv, webm, …) to bring it in as a layer showing a frame; play,
scrub, step frames with `,` and `.`, and turn a frame into a photo layer from the
properties panel. Needs ffmpeg (`winget install Gyan.FFmpeg`, or put it in
`%APPDATA%\DriftingVOIDfmpeg`). See `docs/VIDEO.md` for the timeline and export plans.

## Timeline (Ctrl+T)

Alt+A plays, `,`/`.` step frames, I keys the selection (Alt+I removes), drag keys and span
ends in the panel, right-click a track to set where a layer starts or ends. Canvas
cameras live in the region tool (F) and can be keyed too. Sound files (mp3, wav, …) go
on the timeline as clips you can move and trim; video soundtracks play along.
"● 녹화" in the timeline records pen and brush strokes as you draw them, and they replay
over time. Ctrl+Shift+E exports a video (camera, export frame, content or view; H.264/H.265/AV1,
ProRes, VP9, GIF or PNG frames; size by width or height).

## G/R/S and Space

With the select tool, G/R/S moves/rotates/scales the selection from the keyboard (on
the 2D canvas too; layers in a space use the 3D modal). Holding Space during any
drag pans the view while the dragged thing stays under the pointer.

## 3D workspace (Tab)

Blender-style keys. Middle drag or the pen's side button orbits; Shift pans; Ctrl
zooms; the wheel zooms. 1/3/7 give front/right/top views (Ctrl: the opposite side),
5 toggles orthographic, 0 looks through the matched camera, . frames the selection,
and Home resets the view. G/R/S move/rotate/scale the selected layer: X/Y/Z
constrains the axis (press again for local, again for free), digits type a value,
Ctrl snaps, Shift is fine control, and Enter or a click confirms while Esc or a
right-click cancels. Alt+G/R/S clears the value.

## Files

- `.void` project: zip with `doc.json`, the original photo files, and raw raster tiles.
- Settings: `%APPDATA%\DriftingVOID\settings.json` (glass look) and `prefs.json` (tools, recent colours, export).

## Developer flags

- `DriftingVOID.exe --selftest out.png` builds a sample document, round-trips it
  through `.void`, and exports a PNG (`out.log` gets the result).
- `SB_SUB=1` with `--selftest` moves part of the brush ring and a pen stroke.
- `SB_GROUP=1` with `--selftest` also adds a scaled+rotated copy of every layer.
- `--selftest --list-fonts fonts.txt` dumps the installed font families.
- `DV_SHOW_PALETTE=RRGGBB` / `DV_SHOW_EXPORT=1` open those panels at start (for screenshots).
- `DV_SHOW_UNSAVED=1` shows the unsaved-changes dialog at start.
- `DV_SPACE_LINES="x1,y1,x2,y2,axis;..."` (+ `DV_SPACE_ORIGIN="x,y"`, photo pixels) opens the
  space tool on a space fitted to the first photo. `DV_SPACE_DEMO=space|select` also drops
  two text layers into it, and `DV_SPACE_EXPORT=out.png` exports the result.
  `DV_SPACE_MASK="x0,y0,x1,y1"` sets a mask, `DV_LOUPE=1` holds a line end mid-drag (magnifier),
  `DV_WALL="x,y;x,y;x,y;x,y"` adds a wall with the second text on it, `DV_THICK=30` gives both texts thickness.
- `DV_SHOW_VEXPORT=1` opens the video export window at start.
- `DV_THEME=0|1|2` (light, dark, system); `DV_SHOW_VIEW=1` shows the View menu contents in a card.
- `DV_IMPORT="a.mp4;b.png"` imports files at start; `DV_PLAY=1` also plays the video.
- `DV_TEST_TEXT_FONT="<family>"` starts editing a text box in that installed font.
- Crashes are logged to `%APPDATA%\DriftingVOID\crash.log`.

## Layout

| Path | What |
| --- | --- |
| `src/app.rs` | App state, tools, input, panels, palette |
| `src/doc.rs` | Document model (layers, images, text, shapes) |
| `src/raster.rs` | Tiled raster layers and the brush engine |
| `src/vector.rs` | Pressure vector strokes |
| `src/canvas.rs` | GPU display and texture caches |
| `src/export.rs` | Full-resolution CPU compositor (export, clipboard, eyedropper) |
| `src/text.rs`, `src/fontdb.rs` | Text layout/raster, installed font scan |
| `src/glass.rs` | Frosted glass paint callback (blur, refraction, dispersion) and the scene/overlay render layers |
| `src/theme.rs` | Tokens, visuals, icons |
| `src/view3d.rs` | 3D workspace: orbit view, presets, picking, G/R/S modal |
| `src/anim.rs` | Timeline, keyframes, easing, canvas cameras |
| `src/vexport.rs` | Video export: frame rendering into ffmpeg, formats, sound mix |
| `src/audio.rs` | Sound: ffmpeg decode, mixer on the output device |
| `src/video.rs` | ffmpeg/ffprobe: probe, frame decode, streaming playback |
| `src/persp.rs` | 3D spaces: vanishing-line camera solver (robust LM fit) |
| `src/dial.rs` | Focus-ring dial control with reset pin |
| `src/calc.rs` | Maths parser for number fields |
| `src/okhsv.rs` | OKHSV colour conversions |
| `src/penhook.rs` | Pen button/eraser flags via a window subclass |
| `src/io.rs` | Image import and `.void` save/load |
| `docs/ROADMAP.md` | Planned work and known issues |
| `docs/CHANGELOG.md` | What shipped, per version |
| `docs/VIDEO.md` | Video, audio, timeline, cameras: design and phases |
| `docs/SPACES.md` | 3D spaces: design, maths, references, phases |
| `assets/icon.svg`, `icon.ico`, `icon-256.png` | App icon (source, exe icon, window icon) |
| `assets/icon-candidates/` | Other icon candidates (`preview.html` shows them all) |
| `build.rs` | Embeds the icon and version info into the exe |
