# Assembles dist/manual/index.html from the three language fragments here and
# the old page's style block (kept as the design source).
import io
import os
import re
import sys

here = os.path.dirname(os.path.abspath(__file__))
repo = sys.argv[1]
page = os.path.join(repo, "dist", "manual", "index.html")
src = open(os.path.join(here, "style.css"), encoding="utf-8").read() if os.path.exists(os.path.join(here, "style.css")) else None
old = open(page, encoding="utf-8").read()
if src is None:
    src = old[old.index("<style>"):old.index("</style>") + len("</style>")]
    open(os.path.join(here, "style.css"), "w", encoding="utf-8").write(src)
style = src

add = '''
/* ------------------------------------------------------------ languages
   One booklet per language; the head script picks one before first paint
   (saved choice, else the browser's languages, else English). */
@font-face { font-family: "Pretendard JP"; font-weight: 400; src: local("Pretendard JP"), url("../fonts/PretendardJP-Regular.ttf") format("truetype"); }
@font-face { font-family: "Pretendard JP"; font-weight: 600 800; src: local("Pretendard JP SemiBold"), url("../fonts/PretendardJP-SemiBold.ttf") format("truetype"); }
:lang(ja) { --font-text: "Pretendard JP", "Pretendard", "Yu Gothic UI", "Yu Gothic", Meiryo, sans-serif; font-family: var(--font-text); }
main:lang(en) { word-break: normal; }
html[data-lang] main.book { display: none; }
html[data-lang="ko"] main.book[lang="ko"],
html[data-lang="en"] main.book[lang="en"],
html[data-lang="ja"] main.book[lang="ja"] { display: grid; }
.bar .langs { display: flex; gap: 2px; padding-right: var(--space-1); margin-right: var(--space-1); border-right: 1px solid var(--hair); }
.bar .langs button { padding: 6px 9px; min-width: 34px; }
.bar .langs button[aria-pressed="true"] { background: var(--accent); color: var(--accent-ink); }
@media (max-width: 480px) {
  .bar { left: var(--space-2); justify-content: space-between; }
  .bar a, .bar button { padding: 6px 7px; font-size: 12.5px; }
  .bar .langs button { min-width: 0; padding: 6px 8px; }
}
@media print {
  ol.toc a { padding: 1.5mm 0; }
  ol.toc .t { font-size: 11pt; }
  .compact img { max-height: 58mm; }
  .pin { width: 15px; height: 15px; margin: -7.5px 0 0 -7.5px; font-size: 8.5px; box-shadow: 0 0 0 2px rgb(13 12 19 / 0.55); }
  h3 { margin: 3.5mm 0 1.8mm; }
  ol.steps > li { margin-bottom: 1.4mm; }
  html[data-lang] main.book { display: none !important; }
  html[data-lang="ko"] main.book[lang="ko"],
  html[data-lang="en"] main.book[lang="en"],
  html[data-lang="ja"] main.book[lang="ja"] { display: block !important; }
}
</style>'''
style = style.replace("</style>", add, 1)

head = '''<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Drifting VOID manual</title>
<meta name="description" content="Drifting VOID user manual · 사용 설명서 · ユーザーマニュアル">
<link rel="icon" href="img/icon.png">
<link rel="stylesheet" href="https://cdn.jsdelivr.net/gh/orioncactus/pretendard@v1.3.9/dist/web/static/pretendard.min.css">
<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=JetBrains+Mono:wght@400;600&display=swap">
<script>
// the manual's language, before the first paint: a saved choice, else the
// browser's languages in order, else English
(function () {
  var langs = ["ko", "en", "ja"], pick = "";
  try { pick = localStorage.getItem("dv-manual-lang") || ""; } catch (e) {}
  if (langs.indexOf(pick) < 0) {
    pick = "en";
    var list = navigator.languages && navigator.languages.length ? navigator.languages : [navigator.language || ""];
    for (var i = 0; i < list.length; i++) {
      var c = String(list[i]).slice(0, 2).toLowerCase();
      if (langs.indexOf(c) >= 0) { pick = c; break; }
    }
  }
  document.documentElement.dataset.lang = pick;
  document.documentElement.lang = pick;
})();
</script>
<!-- Hallmark · pre-emit critique: P5 H4 E4 S5 R5 V4
     genre: atmospheric (the app's own system: void ground, one amber accent, glass cards)
     macrostructure: booklet (cover · contents · numbered chapters · reference card · colophon),
     in three languages (ko, en, ja), one shown at a time -->
''' + style + '''
</head>
<body>

<nav class="bar" aria-label="Manual">
  <span class="langs" role="group" aria-label="Language">
    <button type="button" data-lang="ko" lang="ko" title="한국어">한</button>
    <button type="button" data-lang="en" lang="en" title="English">EN</button>
    <button type="button" data-lang="ja" lang="ja" title="日本語">日</button>
  </span>
  <a id="go-toc" href="#en-toc">Contents</a>
  <a id="go-keys" href="#en-keys">Keys</a>
  <button type="button" id="theme">
    <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" aria-hidden="true"><circle cx="8" cy="8" r="5.5"/><path d="M8 2.5v11" /><path d="M8 2.5a5.5 5.5 0 0 1 0 11z" fill="currentColor"/></svg>
  </button>
  <button type="button" id="print" onclick="window.print()">Print</button>
</nav>

'''

body = ""
for l in ["ko", "en", "ja"]:
    f = open(os.path.join(here, l + ".html"), encoding="utf-8").read().replace(" hidden>", ">", 1)
    for attr in ("id", "href", "aria-labelledby"):
        pre = "#" if attr == "href" else ""
        f = f.replace('%s="%sP-' % (attr, pre), '%s="%s%s-' % (attr, pre, l))
    body += f + "\n"

script = open(os.path.join(here, "tail.html"), encoding="utf-8").read()
out = head + body + script
io.open(page, "w", encoding="utf-8", newline="\n").write(out)

ids = re.findall(r'id="([^"]+)"', out)
dups = sorted({i for i in ids if ids.count(i) > 1})
missing = sorted({h for h in re.findall(r'href="#([^"]+)"', out) if h not in ids})
print(len(out), "bytes; books:", out.count('<main class="book"'), "dups:", dups, "missing:", missing)
