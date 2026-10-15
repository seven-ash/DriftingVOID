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
| Shift + drag (select tool) | Box selection; Shift+click toggles |
| Group handles (several selected) | Scale / rotate about the common centre |
| Number fields | Accept maths: `40*1.5`, `2^3+1` |
| Tab | Hide/show UI |
| F12 | Input log (pen flags, mouse/touch events) |

Tools: V select, H pan, B brush (raster), P pen (vector), E eraser, T text,
L line, A arrow, R rectangle, O ellipse, C crop, F export frame, I eyedropper.

Drawing layers are created when you first draw, reused afterwards, and removed
when empty. Photos are placed under drawing layers.

## Files

- `.void` project: zip with `doc.json`, the original photo files, and raw raster tiles.
- Settings: `%APPDATA%\DriftingVOID\settings.json` (glass, shadow, edge strength).

## Developer flags

- `DriftingVOID.exe --selftest out.png` builds a sample document, round-trips it
  through `.void`, and exports a PNG (`out.log` gets the result).
- `SB_GROUP=1` with `--selftest` also adds a scaled+rotated copy of every layer.
- `--selftest --list-fonts fonts.txt` dumps the installed font families.
- `DV_SHOW_PALETTE=RRGGBB` / `DV_SHOW_EXPORT=1` open those panels at start (for screenshots).
- `DV_SHOW_UNSAVED=1` shows the unsaved-changes dialog at start.
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
| `src/okhsv.rs` | OKHSV colour conversions |
| `src/penhook.rs` | Pen button/eraser flags via a window subclass |
| `src/io.rs` | Image import and `.void` save/load |
| `docs/ROADMAP.md` | Planned work and known issues |
| `docs/CHANGELOG.md` | What shipped, per version |
| `assets/icon.svg`, `icon.ico`, `icon-256.png` | App icon (source, exe icon, window icon) |
| `assets/icon-candidates/` | Other icon candidates (`preview.html` shows them all) |
| `build.rs` | Embeds the icon and version info into the exe |
