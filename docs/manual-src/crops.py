# Turns the captures in target/manual-shots into dist/manual/img: full
# scenes at 1920x1080, panels and dialogs cropped at their own pixels
# (boxes in the 2560x1440 capture).
import os
import sys
from PIL import Image

root = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
shots = os.path.join(root, "target", "manual-shots")
out = os.path.join(root, "dist", "manual", "img")
full = ["overview", "light", "space", "ws3d", "fx"]
crops = {
    "lines": ("lines", (700, 100, 2300, 800)),
    "mask-panel": ("mask", (1960, 10, 2300, 640)),
    "palette-card": ("palette", (420, 220, 1200, 580)),
    "timeline-bar": ("timeline", (40, 1180, 2520, 1400)),
    "vexport-card": ("vexport", (520, 200, 880, 700)),
    "text-card": ("text", (1960, 10, 2300, 520)),
}
for n in full:
    im = Image.open(os.path.join(shots, "man_%s.png" % n)).convert("RGB")
    im.resize((1920, 1080), Image.LANCZOS).save(os.path.join(out, n + ".jpg"), quality=84, optimize=True)
for dst, (src, box) in crops.items():
    im = Image.open(os.path.join(shots, "man_%s.png" % src)).convert("RGB").crop(box)
    im.save(os.path.join(out, dst + ".jpg"), quality=88, optimize=True)
print("written to", out)
