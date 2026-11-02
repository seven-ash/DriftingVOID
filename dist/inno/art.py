# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Ash (seven-ash). Additional terms (GPL-3.0 section 7): see NOTICE.
"""The installer's wizard images, drawn from the app icon's parts
(assets/icon.svg: the dot grid, the dashed frame, the violet-to-amber
stroke) on the icon's night colour. Writes dist/inno/img/:

- wizard-<w>x<h>.png: the tall image of the welcome and finish pages
  (164x314 at 100 % and its 125-200 % sizes, as Inno Setup asks);
- small-<n>.png: the small image at the top right of the other pages.

Run: python dist/inno/art.py
"""
import math
import os

from PIL import Image, ImageDraw, ImageFont

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT = os.path.join(ROOT, "dist", "inno", "img")
FONT = os.path.join(ROOT, "assets", "fonts", "PretendardJP-SemiBold.ttf")

NIGHT = (13, 12, 19)
INK = (236, 232, 225)
VIOLET = (110, 80, 220)
AMBER = (242, 165, 65)
SS = 4  # supersampling


def bez(p0, p1, p2, p3, n=48):
    return [
        (
            (1 - t) ** 3 * p0[0] + 3 * (1 - t) ** 2 * t * p1[0] + 3 * (1 - t) * t * t * p2[0] + t ** 3 * p3[0],
            (1 - t) ** 3 * p0[1] + 3 * (1 - t) ** 2 * t * p1[1] + 3 * (1 - t) * t * t * p2[1] + t ** 3 * p3[1],
        )
        for t in (k / n for k in range(n + 1))
    ]


# the stroke of icon.svg, in its 512 units
STROKE = (
    bez((120, 330), (190, 262), (250, 262), (284, 300))
    + bez((284, 300), (312, 334), (360, 330), (414, 266))
    + bez((414, 266), (362, 352), (306, 364), (272, 330))
    + bez((272, 330), (240, 298), (188, 302), (120, 330))
)


def mix(a, b, t):
    return tuple(round(a[i] + (b[i] - a[i]) * t) for i in range(3))


def motif(img, cx, cy, size, frame=True):
    """The icon's frame and stroke, centred at (cx, cy), `size` px for its 512 units."""
    k = size / 512.0
    # the dashes drawn opaque on their own layer, then laid on at 60 %
    # (strokes drawn translucent overlap at their joints)
    layer = Image.new("L", img.size, 0)
    d = ImageDraw.Draw(layer)
    if frame:
        # the dashed frame, turned -8 degrees
        a = math.radians(-8)
        ca, sa = math.cos(a), math.sin(a)

        def at(x, y):
            return (cx + (x * ca - y * sa) * k, cy + (x * sa + y * ca) * k - 6 * k)

        w, h, r = 140, 104, 20
        pts = []
        # the rounded rectangle's outline, walked in steps of one unit
        for (x0, y0, x1, y1) in [(-w + r, -h, w - r, -h), (w, -h + r, w, h - r), (w - r, h, -w + r, h), (-w, h - r, -w, -h + r)]:
            n = int(max(abs(x1 - x0), abs(y1 - y0)))
            pts += [(x0 + (x1 - x0) * i / n, y0 + (y1 - y0) * i / n) for i in range(n)]
            # the corner after this side
            if (x0, y0, x1, y1) == (-w + r, -h, w - r, -h):
                c, s0 = (w - r, -h + r), -90
            elif x0 == w:
                c, s0 = (w - r, h - r), 0
            elif y0 == h:
                c, s0 = (-w + r, h - r), 90
            else:
                c, s0 = (-w + r, -h + r), 180
            pts += [(c[0] + r * math.cos(math.radians(s0 + q)), c[1] + r * math.sin(math.radians(s0 + q))) for q in range(0, 90, 3)]
        def stamp(path):
            # a dash as round dabs every half unit (PIL's wide polylines
            # leave slivers between short segments)
            rr = 4.5 * k
            for (ax, ay), (bx, by) in zip(path, path[1:]):
                n = max(1, int(math.dist((ax, ay), (bx, by)) / (0.5 * k)))
                for i in range(n + 1):
                    x, y = ax + (bx - ax) * i / n, ay + (by - ay) * i / n
                    d.ellipse([x - rr, y - rr, x + rr, y + rr], fill=255)

        # dashes of 28 units with gaps of 18, along the outline
        run, on, dash = 0.0, True, []
        for i in range(len(pts)):
            p, q = pts[i], pts[(i + 1) % len(pts)]
            seg = math.dist(p, q)
            if on:
                dash.append(at(*p))
            run += seg
            if run >= (28 if on else 18):
                if on and len(dash) > 1:
                    stamp(dash + [at(*q)])
                dash, run, on = [], 0.0, not on
        if on and len(dash) > 1:
            stamp(dash)
    img.paste(Image.new("RGB", img.size, INK), (0, 0), layer.point(lambda v: v * 150 // 255))
    # the stroke, violet to amber
    poly = [(cx + (x - 256) * k, cy + (y - 256) * k) for x, y in STROKE]
    mask = Image.new("L", img.size, 0)
    ImageDraw.Draw(mask).polygon(poly, fill=255)
    x0, x1 = cx + (120 - 256) * k, cx + (414 - 256) * k
    grad = Image.new("RGB", img.size)
    gd = ImageDraw.Draw(grad)
    for x in range(img.size[0]):
        gd.line([(x, 0), (x, img.size[1])], fill=mix(VIOLET, AMBER, min(1, max(0, (x - x0) / (x1 - x0)))))
    img.paste(grad, (0, 0), mask)


def dots(img, step, r, alpha):
    d = ImageDraw.Draw(img, "RGBA")
    w, h = img.size
    off = step / 2
    y = off
    while y < h:
        x = off
        while x < w:
            d.ellipse([x - r, y - r, x + r, y + r], fill=INK + (alpha,))
            x += step
        y += step


def wizard(w, h):
    s = w / 164.0
    img = Image.new("RGB", (w * SS, h * SS), NIGHT)
    dots(img, 20 * s * SS, 1.1 * s * SS, 46)
    # a soft amber glow under the motif
    glow = Image.new("RGBA", img.size, (0, 0, 0, 0))
    gd = ImageDraw.Draw(glow)
    cx, cy = img.size[0] / 2, img.size[1] * 0.40
    for i in range(40, 0, -1):
        rr = i * 3.2 * s * SS
        gd.ellipse([cx - rr, cy - rr, cx + rr, cy + rr], fill=AMBER + (3,))
    img.paste(glow, (0, 0), glow)
    motif(img, cx, cy, 190 * s * SS)
    d = ImageDraw.Draw(img, "RGBA")
    f = ImageFont.truetype(FONT, round(19 * s * SS))
    f2 = ImageFont.truetype(FONT, round(8.5 * s * SS))
    ty = img.size[1] * 0.74
    d.text((img.size[0] / 2, ty), "Drifting VOID", font=f, fill=INK, anchor="mm")
    d.text((img.size[0] / 2, ty + 20 * s * SS), "INFINITE CANVAS", font=f2, fill=INK + (120,), anchor="mm")
    # the accent rule
    lw = 22 * s * SS
    d.rounded_rectangle([img.size[0] / 2 - lw, ty + 31 * s * SS, img.size[0] / 2 + lw, ty + 33 * s * SS], radius=SS, fill=AMBER)
    return img.resize((w, h), Image.LANCZOS)


def small(n):
    img = Image.new("RGBA", (n * SS, n * SS), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    d.rounded_rectangle([0, 0, n * SS - 1, n * SS - 1], radius=n * SS * 0.22, fill=NIGHT)
    motif(img, n * SS / 2, n * SS / 2, n * SS)
    return img.resize((n, n), Image.LANCZOS)


if __name__ == "__main__":
    os.makedirs(OUT, exist_ok=True)
    # 100, 125, 150, 175, 200 %
    for w, h in [(164, 314), (205, 393), (246, 471), (287, 550), (328, 628)]:
        wizard(w, h).save(os.path.join(OUT, f"wizard-{w}x{h}.png"))
    for n in [55, 69, 83, 96, 110]:
        small(n).save(os.path.join(OUT, f"small-{n}.png"))
    print("written to", OUT)
