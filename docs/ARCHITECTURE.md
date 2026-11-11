# Architecture

Drifting VOID is a single Rust application: an [egui](https://github.com/emilk/egui)
interface on [wgpu](https://wgpu.rs). One GPU renderer draws the canvas,
the glass panels, playback and every export.

## Data

- **Document** (`src/doc.rs`): a list of layers. A layer is an image,
  text, shape, brush (tiled raster), pen strokes (vectors), video, sound,
  FX, mask or 3D space, and has its properties.
- World units are pixels: a photo placed at scale 1 exports at its
  original resolution.
- **Animation** (`src/anim.rs`): keyframes per property channel, spans on
  the timeline, canvas cameras and easing.
- **Project files** (`.void`, `src/io.rs`): a zip holding `doc.json`, the
  original photo files and the raw raster tiles. Older files are upgraded
  on load.

## Rendering

- **Frame description** (`src/scene.rs`):
  - The document, a time and a view become a flat list of draw items:
    images, meshes, projected layers, groups with blend, opacity and mask,
    and FX passes.
  - Every renderer reads this list, so their outputs can be compared in
    tests.
- **GPU renderer** (`src/gpu.rs`, `src/gpu.wgsl` and the other `.wgsl` files):
  - Draws into offscreen targets, tiled past the GPU's texture limit.
  - Layers in a 3D space are drawn through an exact inverse homography.
  - Thick layers' sides are built from their outlines and drawn with a
    depth buffer.
  - Text and shapes are filled from their outlines with exact coverage
    (`src/gpu_paths.rs`).
  - Groups, masks, blend modes and FX layers render to textures.
  - Exports supersample on the GPU and stream frames to FFmpeg from a
    worker thread (`src/vexport.rs`).
- **Canvas.** Egui draws in one render pass. The layers are therefore
  rendered into a texture first, which egui then shows; glass panels sample
  that texture behind them (`src/glass_gpu.rs`).
- **Playback cache** (`src/playcache.rs`): rendered frames are kept in RAM,
  up to a share of physical memory, so scrubbing and playback stay smooth.
- **CPU reference renderer** (`src/export.rs`, tiny-skia): used by the
  tests, and as a fallback past the GPU's texture size.

## FX layers

- An FX layer filters what is below it in the list.
- The renderer draws those layers again on their own, at a scale that
  suits the filter, then runs the filter's passes (`fs_fx`). It resamples
  the result to the target and writes it back by blend mode, opacity and
  mask.
- Point filters, such as curves, levels and channel to alpha, work on
  straight sRGB values. Curves are baked into a 256-entry table.

## 3D spaces

- `src/persp.rs` solves a camera from vanishing lines that the user draws
  on a photo, with a robust least-squares fit.
- `src/lines.rs` can suggest those lines.
- Layers then stand on the space's floor and walls.
- `src/view3d.rs` is the free 3D workspace: orbit, views, picking and
  the G/R/S modal.

## Masks and selection

- `src/segment.rs` holds GrabCut (Rother, Kolmogorov and Blake, 2004) and
  intelligent scissors (Mortensen and Barrett, 1995).
- Masks are layers drawn as coverage.

## Video and sound

- `src/video.rs` runs FFmpeg and ffprobe as separate processes to probe,
  decode and stream.
- `src/audio.rs` decodes sound with FFmpeg and mixes it on the output
  device (cpal).
- `src/vcache.rs` caches decoded video frames.

## Interface

- `src/app.rs` holds the app state, tools, input and panels.
- `src/kit.rs` holds shared widgets (switches, segmented controls,
  drop-downs, menus); `src/dial.rs` holds the dial.
- `src/theme.rs` holds the design tokens; `src/glass.rs` the glass
  settings.
- `src/i18n.rs` with `lang/` holds the translations; Korean is the source
  language.

## Updates

`src/update.rs` handles updates; it asks only in release builds.
1. At start it asks the GitHub API once for the latest release, using
   Windows' `curl.exe`.
2. If the release is newer, a notice appears in the top bar.
3. A click downloads the release's `-Setup.exe` and checks it against the
   SHA-256 that GitHub publishes.
4. The app then quits through the usual unsaved-changes question and runs
   the installer silently. The installer starts the app again.

## Source map

| Path | What |
| --- | --- |
| `src/app.rs` | App state, tools, input, panels, palette |
| `src/doc.rs` | Document model, filters and curves |
| `src/anim.rs` | Timeline, keyframes, easing, canvas cameras |
| `src/scene.rs` | Frame description shared by the renderers |
| `src/gpu.rs`, `*.wgsl` | GPU renderer and shaders |
| `src/gpu_paths.rs` | Text and shapes filled on the GPU |
| `src/glass_gpu.rs`, `src/glass.rs` | Glass panels |
| `src/playcache.rs`, `src/vcache.rs` | Playback cache, video frame cache |
| `src/export.rs` | CPU reference renderer |
| `src/raster.rs`, `src/vector.rs` | Brush layers and pen strokes |
| `src/text.rs`, `src/fontdb.rs` | Text layout and installed fonts |
| `src/persp.rs`, `src/lines.rs`, `src/lsd.rs`, `src/view3d.rs` | 3D spaces and the 3D workspace (`lsd.rs`: the line segment detector, under the GNU AGPL) |
| `src/extrude.rs` | Thickness: side meshes from a layer's outline |
| `src/edges.rs` | Edge loupe for placing lines on a photo |
| `src/segment.rs` | GrabCut and intelligent scissors |
| `src/video.rs`, `src/audio.rs`, `src/vexport.rs` | Video, sound, video export |
| `src/io.rs` | Import and `.void` files |
| `src/update.rs` | Release updates |
| `src/i18n.rs`, `lang/` | Translations |
| `src/kit.rs`, `src/dial.rs`, `src/theme.rs` | Widgets and design tokens |
| `src/okhsv.rs`, `src/calc.rs` | Colour maths, maths in number fields |
| `src/penhook.rs`, `src/oledrop.rs` | Pen buttons, drag and drop from browsers |
| `src/golden.rs`, `tests/` | Rendering tests and golden images |
| `dist/` | Manual, installer, packaging, licence tooling |
