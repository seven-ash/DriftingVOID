# Building, testing and releasing

## Requirements

- Windows 10 or 11 (64-bit), a GPU with DX12 or Vulkan.
- Rust (stable) with the MSVC toolchain.
- For video and sound while developing: `ffmpeg` and `ffprobe` on `PATH`,
  in `%APPDATA%\DriftingVOID\ffmpeg`, or in `ffmpeg\bin` next to the exe.
- For packaging: PowerShell 5.1 (built in), [Inno Setup](https://jrsoftware.org/isinfo.php)
  6.7 or later for the installer, Python 3 with Pillow for the installer
  art, and [`cargo-about`](https://github.com/EmbarkStudios/cargo-about)
  for the third-party notices.

## Build and run

```bash
cargo build --release
```

The app is `target/release/DriftingVOID.exe`; a `.void` file given as the
first argument is opened. Close a running copy before building, or the
link step fails because the exe is locked.

## Tests

```bash
cargo test --release
```

- Unit tests sit next to the code they test.
- `src/golden.rs` holds rendering tests:
  - images of the CPU reference renderer against `tests/golden/`;
  - the GPU renderer against the CPU one, with tolerances per scene;
  - filters measured against their definitions.
- GPU tests skip themselves on a machine without a usable GPU.
- When a change to the look is intended, rewrite the golden images with
  `DV_GOLDEN_WRITE=1 cargo test --release golden`. Look at the `.got.png`
  files a failing run leaves before you do.
- Some slow or file-writing tests are `#[ignore]`d; run them by name.

## Checking the interface

The app has environment flags (`DV_*`) that open a scene, a dialog or a
state at start, so screens can be captured without driving the mouse.
Examples:
- `DV_LANG=en|ko|ja` sets the language.
- `DV_THEME=0|1|2` sets the theme: light, dark or the system's.
- `DV_FULLSCREEN=1` starts full screen.
- `DV_NOSAVE=1` keeps your settings untouched.
- `DV_SHOW_EXPORT=1` and `DV_SHOW_VEXPORT=1` open the export dialogs.
- `DV_FX="<filter>,<v0>,<v1>"` adds an FX layer.
- `DV_UPDATE_FAKE=<version>` shows an update notice.

`docs/manual-src/snap.ps1` captures the window with `PrintWindow`. Each
flag is described where the code reads it (`grep -n "DV_" src/*.rs`).

## Interface text and languages

- Korean is the source language. Every string goes through `crate::i18n`:
  - `tr("…")` for plain text;
  - `trf("… {} …", &[value])` for text with values.
- English and Japanese live in `lang/translations.py`: one row per Korean
  string. After adding rows, run `python lang/translations.py`; it writes
  `lang/en.json` and `lang/ja.json`.
- Then run the tests: `every_string_has_a_translation` lists any missing
  key or a key with mismatched slots.
- Labels are short. Dial and row labels have about 58 px of room; measure
  new words in all three languages.

## The manual

The user manual is one HTML page in three languages, built from
`docs/manual-src/` (see its README):

```bash
python docs/manual-src/build.py .
```

That writes `dist/manual/index.html`, which GitHub Pages publishes.
After changing text, print each language to PDF and check that the page
count holds (16 A4 pages). `capture.ps1` retakes the screenshots.

## Third-party notices

After changing dependencies, regenerate the notices:

```bash
cargo about generate -c dist/legal/about.toml dist/legal/about.hbs -o THIRD-PARTY-NOTICES.html
```

Every dependency must be under a licence compatible with the AGPL-3.0; the
accepted list is in `dist/legal/about.toml`.

## Packaging and releases

Release versions (`1.0.1.R`) are separate from the development versions in
`Cargo.toml`.

```powershell
powershell -ExecutionPolicy Bypass -File dist\package.ps1 -Release 1.0.1.R
```

This command:
1. builds the exe with the release version stamped in;
2. writes `target\dist\DriftingVOID-<version>\` and its `.zip`;
3. builds `DriftingVOID-<version>-Setup.exe` with Inno Setup
   (`dist/inno/DriftingVOID.iss`).

Without `-Release` it makes a dev package named after `Cargo.toml`.

The installer:
- installs per user, into `%LOCALAPPDATA%\Programs\DriftingVOID`;
- offers an FFmpeg download. `dist/inno/ffmpeg.ps1` picks the build;
  Inno Setup downloads it and checks the SHA-256. FFmpeg is never bundled;
- accepts `/relaunch=1`, which the in-app updater passes to start the app
  again after a silent install.

`python dist/inno/art.py` redraws the wizard images.

To publish a release:
1. Tag the commit `release-<version>` and push it.
2. Create a GitHub release for the tag.
3. Attach the `-Setup.exe`.

The app's update check (`src/update.rs`) reads the latest release. It
takes the asset whose name ends in `-Setup.exe` and the SHA-256 GitHub
publishes for it, so keep that naming.
