"""Draws the alternative app icons (docs/features/themes.md, "Custom app icons").

Each icon is the StickTalk mark (a speech bubble with a stick figure) in another
color or style, drawn here from simple shapes: original artwork, nothing copied.
Writes public/app-icons/<id>-192.png, <id>-512.png, <id>-maskable-512.png and
<id>.webmanifest. Run again after changing it; do not edit the PNGs by hand.

    python crates/web/scripts/app-icons.py
"""
import json
from pathlib import Path

from PIL import Image, ImageChops, ImageDraw, ImageFilter

OUT = Path(__file__).resolve().parent.parent / "public" / "app-icons"
SS = 4  # supersampling for smooth edges

# id: (label, background (one color or a top-left to bottom-right pair),
#      bubble (one color or a pair), figure color, style)
ICONS = {
    "mint": ("Mint", "#111113", "#3ddc97", "#08170f", "solid"),
    "ocean": ("Ocean", "#0b1f3a", "#5bc0eb", "#0b1f3a", "solid"),
    "violet": ("Violet", "#1a1030", "#b48cff", "#1a1030", "solid"),
    "rose": ("Rose", "#2a0f1a", "#ff8fb1", "#2a0f1a", "solid"),
    "amber": ("Amber", "#2a1a05", "#f2b84b", "#2a1a05", "solid"),
    "graphite": ("Graphite", "#1c1c20", "#a8a8b0", "#1c1c20", "solid"),
    "snow": ("Snow", "#f2f3f5", "#06603f", "#ffffff", "solid"),
    "sunset": ("Sunset", ("#3d1a0f", "#5e1f3a"), ("#fdba74", "#fb7185"), "#2b0f14", "solid"),
    "aurora": ("Aurora", "#0b0b2a", ("#6ee7b7", "#a5b4fc"), "#0b0b2a", "solid"),
    "outline": ("Outline", "#111113", "#3ddc97", "#3ddc97", "outline"),
    "midnight": ("Midnight", "#000000", "#1e3a6b", "#93c5fd", "solid"),
    "lime": ("Lime", "#102010", "#a3e635", "#102010", "solid"),
}


def rgb(h):
    h = h.lstrip("#")
    return tuple(int(h[i : i + 2], 16) for i in (0, 2, 4))


def gradient(size, colors):
    """A top-left to bottom-right gradient, or a flat fill."""
    if isinstance(colors, str):
        return Image.new("RGBA", (size, size), rgb(colors) + (255,))
    a, b = rgb(colors[0]), rgb(colors[1])
    n = 64  # drawn small, then smoothly scaled up
    img = Image.new("RGBA", (n, n))
    px = img.load()
    for y in range(n):
        for x in range(n):
            t = (x + y) / (2 * (n - 1))
            px[x, y] = tuple(round(a[i] + (b[i] - a[i]) * t) for i in range(3)) + (255,)
    return img.resize((size, size), Image.BILINEAR)


def mark(size, inset):
    """Masks of the bubble and the figure, in a square of `size` with the mark
    scaled into the area inside `inset` (a fraction of the side)."""
    s = size
    box = s * (1 - 2 * inset)
    o = s * inset
    u = box / 112  # the mark is designed on a 112-unit grid

    def p(x, y):
        return (o + x * u, o + y * u)

    bubble = Image.new("L", (s, s), 0)
    d = ImageDraw.Draw(bubble)
    d.rounded_rectangle([p(0, 6), p(112, 92)], radius=18 * u, fill=255)
    d.polygon([p(14, 84), p(40, 84), p(16, 110)], fill=255)
    fig = Image.new("L", (s, s), 0)
    f = ImageDraw.Draw(fig)
    w = max(1, round(7 * u))
    f.ellipse([p(47, 18), p(65, 36)], fill=255)
    f.line([p(56, 36), p(56, 62)], fill=255, width=w)
    f.line([p(40, 46), p(72, 46)], fill=255, width=w)
    f.line([p(56, 60), p(44, 78)], fill=255, width=w)
    f.line([p(56, 60), p(68, 78)], fill=255, width=w)
    for x, y in [(56, 36), (56, 62), (40, 46), (72, 46), (44, 78), (68, 78)]:
        cx, cy = p(x, y)
        f.ellipse([cx - w / 2, cy - w / 2, cx + w / 2, cy + w / 2], fill=255)
    return bubble, fig


def draw(spec, size, maskable=False):
    _, bg, bubble_color, figure_color, style = spec
    s = size * SS
    img = gradient(s, bg)
    if not maskable:
        # The rounded tile, transparent outside it.
        tile = Image.new("L", (s, s), 0)
        ImageDraw.Draw(tile).rounded_rectangle([0, 0, s - 1, s - 1], radius=s * 0.22, fill=255)
        img.putalpha(tile)
    # Maskable icons keep the mark inside the 80 % safe zone.
    bubble, fig = mark(s, 0.25 if maskable else 0.19)
    if style == "outline":
        # Outline: the bubble minus an evenly eroded copy of itself (eroded at
        # a quarter of the size, which is plenty for a smooth ring).
        q = s // 4
        small = bubble.resize((q, q), Image.BILINEAR)
        k = max(3, round(q * 0.035) | 1)
        inner = small.filter(ImageFilter.MinFilter(k))
        ring = ImageChops.subtract(small, inner).resize((s, s), Image.BILINEAR)
        img.paste(gradient(s, bubble_color), mask=ring)
    else:
        img.paste(gradient(s, bubble_color), mask=bubble)
    img.paste(Image.new("RGBA", (s, s), rgb(figure_color) + (255,)), mask=fig)
    return img.resize((size, size), Image.LANCZOS)


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    base = json.loads((OUT.parent / "manifest.webmanifest").read_text(encoding="utf8"))
    for key, spec in ICONS.items():
        draw(spec, 192).save(OUT / f"{key}-192.png", optimize=True)
        draw(spec, 512).save(OUT / f"{key}-512.png", optimize=True)
        draw(spec, 512, maskable=True).save(OUT / f"{key}-maskable-512.png", optimize=True)
        manifest = dict(base)
        manifest["icons"] = [
            {"src": f"/public/app-icons/{key}-192.png", "sizes": "192x192", "type": "image/png", "purpose": "any"},
            {"src": f"/public/app-icons/{key}-512.png", "sizes": "512x512", "type": "image/png", "purpose": "any"},
            {"src": f"/public/app-icons/{key}-maskable-512.png", "sizes": "512x512", "type": "image/png", "purpose": "maskable"},
        ]
        (OUT / f"{key}.webmanifest").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf8")
    print(f"{len(ICONS)} icons in {OUT}")


if __name__ == "__main__":
    main()
