"""Draw the server tag badge glyphs (docs/features/identity.md).

Original, simple geometric art made for StickTalk: one 16x16 SVG per glyph in
public/tags/, black on transparent. The web app uses each file as a CSS mask,
so a badge is the glyph in the tag's second color on a tile of its first
color. The IDs must match `TAG_GLYPHS` in crates/proto/src/identity.rs.
Change this script and run it again rather than editing the files by hand:

    python crates/web/scripts/tag_glyphs.py
"""
from pathlib import Path
import math

OUT = Path(__file__).resolve().parent.parent / "public" / "tags"


def circle(cx, cy, r):
    return f"M{cx - r:g} {cy:g}a{r:g} {r:g} 0 1 0 {2 * r:g} 0a{r:g} {r:g} 0 1 0 {-2 * r:g} 0Z"


def ellipse(cx, cy, rx, ry):
    return f"M{cx - rx:g} {cy:g}a{rx:g} {ry:g} 0 1 0 {2 * rx:g} 0a{rx:g} {ry:g} 0 1 0 {-2 * rx:g} 0Z"


def rays(n, r0, r1, w):
    out = []
    for i in range(n):
        a = 2 * math.pi * i / n
        dx, dy = math.cos(a), math.sin(a)
        px, py = -dy * w / 2, dx * w / 2
        x0, y0, x1, y1 = 8 + dx * r0, 8 + dy * r0, 8 + dx * r1, 8 + dy * r1
        out.append(
            f"M{x0 + px:.2f} {y0 + py:.2f}L{x1 + px:.2f} {y1 + py:.2f}L{x1 - px:.2f} {y1 - py:.2f}L{x0 - px:.2f} {y0 - py:.2f}Z"
        )
    return "".join(out)


def petals():
    out = []
    for i in range(5):
        a = 2 * math.pi * i / 5 - math.pi / 2
        out.append(circle(round(8 + 3.6 * math.cos(a), 2), round(8 + 3.6 * math.sin(a), 2), 2.9))
    return "".join(out)


def path(d, evenodd=False):
    rule = ' fill-rule="evenodd"' if evenodd else ""
    return f'<path{rule} d="{d}"/>'


def stroke(d, width=1.6):
    return f'<path fill="none" stroke="#000" stroke-width="{width}" stroke-linecap="round" stroke-linejoin="round" d="{d}"/>'


GLYPHS = {
    "leaf": path("M2.5 13.5C2.5 6.5 6.5 2.5 13.5 2.5c0 7-4 11-11 11Z") + stroke("M3.5 12.5 9 7", 1.2),
    "flame": path("M8 1c1.2 3 4.5 4.5 4.5 8.8A4.5 4.5 0 0 1 3.5 9.8C3.5 7.2 5.5 6 5.8 3.8c1.3.9 1.8 2.2 1.8 3.4C8.8 5.6 9 3.4 8 1Z"),
    "star": path("M8 1.2l2 4.3 4.7.6-3.5 3.2.9 4.7L8 11.6 3.9 14l.9-4.7L1.3 6.1 6 5.5Z"),
    "heart": path("M8 14.3S1.2 10.2 1.2 5.6A3.4 3.4 0 0 1 8 4.1a3.4 3.4 0 0 1 6.8 1.5C14.8 10.2 8 14.3 8 14.3Z"),
    "bolt": path("M9.8.8 2.8 9.2h4.4l-1 6 7-8.6H8.8Z"),
    "moon": path("M10.2 1.2A6.8 6.8 0 1 0 14.8 11a5.6 5.6 0 0 1-4.6-9.8Z"),
    "sun": path(circle(8, 8, 3.2) + rays(8, 4.6, 7.2, 1.5)),
    "drop": path("M8 1.2S3 7 3 10.2a5 5 0 0 0 10 0C13 7 8 1.2 8 1.2Z"),
    "shield": path("M8 .9 2.2 3v4.6c0 3.6 2.5 6.2 5.8 7.6 3.3-1.4 5.8-4 5.8-7.6V3Z"),
    "crown": path("M1.2 4.2 4.7 7.4 8 2.4l3.3 5 3.5-3.2-1.2 7.4H2.4ZM2.4 12.6h11.2v1.8H2.4Z"),
    "gem": path("M4.4 1.8h7.2L15 6l-7 8.6L1 6Z" + "M4.9 3.3 3 6h3.1Zm6.2 0L13 6H9.9ZM7.3 3.3 6.3 6h3.4l-1-2.7Z", True),
    "paw": path(ellipse(8, 11, 3.6, 3) + circle(3.4, 6.8, 1.6) + circle(6.2, 3.8, 1.6) + circle(9.8, 3.8, 1.6) + circle(12.6, 6.8, 1.6)),
    "wave": path("M.8 9.2c1.8-2.6 3.8-2.6 5.6 0s3.8 2.6 5.6 0c.9-1.2 2-1.8 3.2-1.8v7.4H.8Z"),
    "peak": path("M.8 14.2 5.8 4.6 8.4 9l2-3.1 4.8 8.3Z"),
    "pine": path("M8 .8 3.4 7h2.4l-3 4.2h4v3.6h2.4v-3.6h4l-3-4.2h2.4Z"),
    "bloom": path(petals() + circle(8, 8, 1.8), True),
    "note": path("M6 2.6v7.6a2.4 2.4 0 1 0 1.6 2.3V6.1l5.2-1.3v4a2.4 2.4 0 1 0 1.6 2.3V.9Z"),
    "pad": path("M4.5 4h7a3.8 3.8 0 0 1 3.7 4.4l-.5 3a2 2 0 0 1-3.5.9L10.1 11H5.9l-1.1 1.3a2 2 0 0 1-3.5-.9l-.5-3A3.8 3.8 0 0 1 4.5 4Z"
                + "M3.8 6.4h1v1.1h1.1v1H4.8v1.1h-1V8.5H2.7v-1h1.1Z" + circle(11.3, 6.9, .75) + circle(12.6, 8.4, .75), True),
    "blade": path("M14.6 1.4 14.2 4 7.4 10.8 5.2 8.6 12 1.8ZM3.6 8.4l4 4-1.1 1.1-1.3-1.3-1.9 1.9a1 1 0 0 1-1.4-1.4l1.9-1.9-1.3-1.3Z"),
    "rocket": path("M8 .8c2.6 1.9 3.7 4.7 3.7 7.8l1.6 2.4v2.4l-3-1.2H5.7l-3 1.2V11l1.6-2.4C4.3 5.5 5.4 2.7 8 .8Z" + circle(8, 6.2, 1.5) + "M6.8 13h2.4L8 15.4Z", True),
    "ring": path(circle(8, 8, 4.2)) + '<rect x=".4" y="7.3" width="15.2" height="1.6" rx=".8" transform="rotate(-24 8 8)"/>',
    "ghost": path("M2.8 14.6V7.2a5.2 5.2 0 0 1 10.4 0v7.4l-1.7-1.3-1.8 1.3L8 13.3l-1.7 1.3-1.8-1.3Z" + circle(6.1, 7.4, 1.1) + circle(9.9, 7.4, 1.1), True),
    "cat": path("M1.8 2.2 5 5.4h6l3.2-3.2v7.3a6.2 5.3 0 0 1-12.4 0Z" + ellipse(5.7, 9, .9, 1.2) + ellipse(10.3, 9, .9, 1.2), True),
    "fish": path("M.8 8c2.2-3.6 6.7-4.6 10.2-1.6L15.2 4v8L11 9.6C7.5 12.6 3 11.6.8 8Z" + circle(4.4, 7.3, .9), True),
    "bird": path("M1.4 9.3C2.5 6 5.6 4.9 8.4 6L10.8 3l1.6.9-.5 2.4c1.6.9 2.3 2.4 1.8 4H9.2L6 13.9v-3.4c-2 .5-3.8.2-4.6-1.2Z"),
    "cap": path("M1.2 8.6a6.8 5.9 0 0 1 13.6 0ZM5.9 9.6h4.2v3.6a2.1 2.1 0 0 1-4.2 0Z"),
    "anchor": stroke("M8 5.2v9.2M2.8 9.6c0 2.6 2.3 4.8 5.2 4.8s5.2-2.2 5.2-4.8M5.4 7h5.2")
    + f'<path fill="none" stroke="#000" stroke-width="1.5" d="{circle(8, 3.2, 1.7)}"/>',
    "key": path(circle(5, 8, 3.8) + circle(5, 8, 1.4) + "M8.2 7.1h7v1.8h-1.4v2.2H12V8.9h-1v1.7H9.2V8.9h-1Z", True),
    "cup": path("M2.2 3.8h9.6V9a4.2 4.2 0 0 1-4.2 4.2H6.4A4.2 4.2 0 0 1 2.2 9Z"
                + "M11.8 5.2h.9a2.3 2.3 0 0 1 0 4.6h-1.1V8.2h1.1a.7.7 0 0 0 0-1.4h-.9ZM1.4 14h11.2v1.2H1.4Z", True),
    "book": path("M1.6 2.4h4.8A1.6 1.6 0 0 1 8 4v10.6a1.6 1.6 0 0 0-1.6-1.6H1.6ZM14.4 2.4H9.6A1.6 1.6 0 0 0 8 4v10.6A1.6 1.6 0 0 1 9.6 13h4.8Z"),
    "die": path("M3.6 1.6h8.8a2 2 0 0 1 2 2v8.8a2 2 0 0 1-2 2H3.6a2 2 0 0 1-2-2V3.6a2 2 0 0 1 2-2Z"
                + circle(5, 5, 1.2) + circle(8, 8, 1.2) + circle(11, 11, 1.2), True),
    "eye": path("M.6 8S3.2 2.8 8 2.8 15.4 8 15.4 8 12.8 13.2 8 13.2.6 8 .6 8Z" + circle(8, 8, 3) + circle(8, 8, 1.4), True),
}


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    for name, body in GLYPHS.items():
        svg = f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16">{body}</svg>\n'
        (OUT / f"{name}.svg").write_text(svg, encoding="utf-8", newline="\n")
    (OUT / "NOTICE.md").write_text(
        "Server tag badge glyphs, drawn for StickTalk by `crates/web/scripts/tag_glyphs.py`.\n"
        "Original artwork; regenerate with the script instead of editing these files.\n",
        encoding="utf-8",
        newline="\n",
    )
    print(f"{len(GLYPHS)} glyphs in {OUT}")


if __name__ == "__main__":
    main()
