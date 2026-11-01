# Manual sources

`dist/manual/index.html` is built from here; edit these, not the page.

- `ko.html`, `en.html`, `ja.html`: one booklet per language, same chapters
  and figures. Ids and links use the `P-` prefix (`id="P-c5"`,
  `href="#P-c5"`); the build makes them `ko-c5` and so on. Pages print as
  one A4 sheet each: after an edit, print each language to PDF and check
  it is still 16 pages.
- `style.css`: the booklet's design (tokens from `src/theme.rs`). The build
  adds the language rules after it.
- `tail.html`: the scripts (language bar, theme toggle).
- `build.py <repo>`: writes `dist/manual/index.html` and reports duplicate
  ids and broken links.
- `capture.ps1` (`-Only overview,fx`, `-Lang en`): full-screen captures in
  English with a new user's settings (`appdata/`), then `crops.py` writes
  `dist/manual/img/`. Captures use `DV_*` scene flags and PrintWindow, never
  desktop input.

The page picks its language before the first paint: a saved choice, else
the browser's languages in order, else English. The 한/EN/日 buttons switch
and remember it. `package.ps1` replaces `{VERSION}` on the cover.
