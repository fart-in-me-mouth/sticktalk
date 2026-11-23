"""Generate StickTalk's original cosmetic SVG artwork.

Usage: python crates/web/scripts/cosmetics.py crates/web/public/cosmetics
(badge icons go to the sibling public/badges directory).

Decorations: viewBox 136x136, the avatar is the circle r=50 at (68,68).
Effects: 240x240 seamless tiles (particles), moved with CSS transforms.
Badges: 32x32 icons.
Every file is static (no SVG animation): motion is CSS on the element, so
nothing moves unless the app asks for it.
"""
import math, os, random, sys

out = sys.argv[1]
os.makedirs(out, exist_ok=True)
C = 68


def svg(w, h, body, defs=""):
    d = f"<defs>{defs}</defs>" if defs else ""
    return f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}" width="{w}" height="{h}">{d}{body}</svg>\n'


def pt(r, deg):
    a = math.radians(deg - 90)
    return C + r * math.cos(a), C + r * math.sin(a)


def arc(r, a0, a1, **attrs):
    x0, y0 = pt(r, a0)
    x1, y1 = pt(r, a1)
    large = 1 if (a1 - a0) % 360 > 180 else 0
    extra = " ".join(f'{k.replace("_", "-")}="{v}"' for k, v in attrs.items())
    return f'<path d="M{x0:.2f} {y0:.2f} A{r} {r} 0 {large} 1 {x1:.2f} {y1:.2f}" fill="none" {extra}/>'


def star4(x, y, r, fill):
    k = r * 0.28
    return (f'<path d="M{x:.1f} {y - r:.1f} L{x + k:.1f} {y - k:.1f} L{x + r:.1f} {y:.1f} L{x + k:.1f} {y + k:.1f} '
            f'L{x:.1f} {y + r:.1f} L{x - k:.1f} {y + k:.1f} L{x - r:.1f} {y:.1f} L{x - k:.1f} {y - k:.1f}Z" fill="{fill}"/>')


def heart(x, y, s, fill):
    return (f'<path transform="translate({x:.1f} {y:.1f}) scale({s:.2f})" d="M0 3C-2-1-8 0-8 4.5C-8 8-3 10.5 0 13C3 10.5 8 8 8 4.5C8 0 2-1 0 3Z" '
            f'fill="{fill}"/>')


deco = {}
deco["halo"] = svg(136, 136,
    f'<circle cx="68" cy="68" r="56" fill="none" stroke="#3DDC97" stroke-opacity=".35" stroke-width="10" filter="url(#b)"/>'
    f'<circle cx="68" cy="68" r="55" fill="none" stroke="url(#g)" stroke-width="4"/>'
    f'<circle cx="68" cy="68" r="60.5" fill="none" stroke="#E8FFF4" stroke-opacity=".5" stroke-width="1"/>',
    '<linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#C9FBE3"/><stop offset=".5" stop-color="#3DDC97"/><stop offset="1" stop-color="#9EF0C9"/></linearGradient>'
    '<filter id="b" x="-20%" y="-20%" width="140%" height="140%"><feGaussianBlur stdDeviation="3"/></filter>')
mx, my = pt(58, 45)
sx, sy = pt(58, 225)
deco["orbit"] = svg(136, 136,
    f'<circle cx="68" cy="68" r="58" fill="none" stroke="#A8B4FF" stroke-opacity=".75" stroke-width="2" stroke-dasharray="2 5" stroke-linecap="round"/>'
    f'<circle cx="68" cy="68" r="54" fill="none" stroke="#6F7CFF" stroke-opacity=".45" stroke-width="1.5"/>'
    f'<circle cx="{mx:.1f}" cy="{my:.1f}" r="8" fill="url(#m)"/><circle cx="{mx - 2.5:.1f}" cy="{my + 1:.1f}" r="1.6" fill="#8E95B8"/>'
    f'<circle cx="{sx:.1f}" cy="{sy:.1f}" r="3.5" fill="#FFD37A"/>',
    '<radialGradient id="m" cx=".35" cy=".35"><stop offset="0" stop-color="#FFFFFF"/><stop offset="1" stop-color="#B9C0E8"/></radialGradient>')
leaf = lambda x, y, rot, s, c: f'<path transform="translate({x} {y}) rotate({rot}) scale({s})" d="M0 0C6-8 18-8 24 0C18 8 6 8 0 0Z" fill="{c}"/><path transform="translate({x} {y}) rotate({rot}) scale({s})" d="M1 0H22" stroke="#1F6B43" stroke-width="1" fill="none"/>'
deco["sprout"] = svg(136, 136,
    arc(57, 120, 240, stroke="#5FBF6E", stroke_width=3, stroke_linecap="round") +
    leaf(40, 108, 150, 1.0, "#7FD88A") + leaf(30, 96, 200, 0.8, "#4DB860") +
    leaf(96, 108, 30, 1.0, "#7FD88A") + leaf(106, 96, -20, 0.8, "#4DB860") +
    leaf(68, 124, 90, 0.7, "#9BE6A4") +
    '<circle cx="54" cy="122" r="3" fill="#FFE08A"/><circle cx="82" cy="122" r="3" fill="#FFB3C7"/>')
deco["crown"] = svg(136, 136,
    '<g transform="rotate(-14 68 18)">'
    '<path d="M42 30L46 6L58 20L68 2L78 20L90 6L94 30Z" fill="url(#gold)" stroke="#9A6B12" stroke-width="1.5" stroke-linejoin="round"/>'
    '<rect x="42" y="28" width="52" height="7" rx="2" fill="#E3A82B" stroke="#9A6B12" stroke-width="1.5"/>'
    '<circle cx="68" cy="22" r="3.5" fill="#F25F5C"/><circle cx="53" cy="25" r="2.5" fill="#5BB8F2"/><circle cx="83" cy="25" r="2.5" fill="#3DDC97"/>'
    '<circle cx="46" cy="6" r="2.5" fill="#FFF1C2"/><circle cx="68" cy="2.5" r="2.5" fill="#FFF1C2"/><circle cx="90" cy="6" r="2.5" fill="#FFF1C2"/></g>',
    '<linearGradient id="gold" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#FFE08A"/><stop offset="1" stop-color="#E3A82B"/></linearGradient>')
ear = lambda flip: (f'<g transform="translate(68 0) scale({flip} 1) translate(-68 0)">'
                    '<path d="M30 40L24 4L58 22Z" fill="#3A3A41" stroke="#1C1C20" stroke-width="2" stroke-linejoin="round"/>'
                    '<path d="M33 33L30 13L50 23Z" fill="#F7A8C4"/></g>')
deco["cat_ears"] = svg(136, 136, ear(1) + ear(-1))
random.seed(7)
stars = []
for i, a in enumerate(range(0, 360, 30)):
    r = 60 + random.uniform(-3, 4)
    x, y = pt(r, a + random.uniform(-8, 8))
    s = random.choice([3, 4, 5, 6.5])
    stars.append(star4(x, y, s, random.choice(["#FFF4C7", "#FFFFFF", "#FFD37A", "#C9D2FF"])))
deco["starfield"] = svg(136, 136, f'<circle cx="68" cy="68" r="55" fill="none" stroke="#2E3A78" stroke-opacity=".8" stroke-width="5"/>' + "".join(stars))
flames = ""
for a, h in [(-40, 14), (-20, 20), (0, 26), (20, 20), (40, 14)]:
    x, y = pt(54, a)
    flames += f'<path transform="translate({x:.1f} {y:.1f}) rotate({a})" d="M-6 2C-6-6 0-{h} 0-{h}C0-{h} 6-6 6 2C6 6-6 6-6 2Z" fill="url(#fl)"/>'
deco["ember"] = svg(136, 136,
    '<circle cx="68" cy="68" r="55" fill="none" stroke="url(#er)" stroke-width="5"/>' + flames,
    '<linearGradient id="er" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#FFB03A"/><stop offset="1" stop-color="#F25F5C"/></linearGradient>'
    '<linearGradient id="fl" x1="0" y1="1" x2="0" y2="0"><stop offset="0" stop-color="#F25F5C"/><stop offset=".6" stop-color="#FF9A3C"/><stop offset="1" stop-color="#FFE08A"/></linearGradient>')
flake = lambda x, y: (f'<g transform="translate({x:.1f} {y:.1f})" stroke="#E6F7FF" stroke-width="1.6" stroke-linecap="round">'
                      + "".join(f'<path transform="rotate({r})" d="M0-6V6M-2.5-4L0-6L2.5-4M-2.5 4L0 6L2.5 4"/>' for r in (0, 60, 120)) + '</g>')
deco["frost"] = svg(136, 136,
    '<circle cx="68" cy="68" r="56" fill="none" stroke="#BFE9FF" stroke-width="3" stroke-dasharray="10 4 2 4" stroke-linecap="round"/>'
    '<circle cx="68" cy="68" r="60" fill="none" stroke="#7FCBF2" stroke-opacity=".45" stroke-width="1.5"/>'
    + "".join(flake(*pt(60, a)) for a in (20, 110, 200, 290)))
px = ""
for a in range(0, 360, 6):
    x, y = pt(57, a)
    px += f'<rect x="{round(x / 4) * 4 - 2}" y="{round(y / 4) * 4 - 2}" width="5" height="5" fill="{"#7CF29C" if (a // 6) % 3 else "#3FBF62"}"/>'
deco["pixel"] = svg(136, 136, px + '<rect x="96" y="10" width="6" height="6" fill="#FFE08A"/><rect x="104" y="18" width="4" height="4" fill="#FFE08A"/>')
seg = ""
for i in range(24):
    seg += arc(56, i * 15, i * 15 + 16, stroke=f"hsl({i * 15} 85% 65%)", stroke_width=5)
deco["prism"] = svg(136, 136, seg + '<circle cx="68" cy="68" r="60" fill="none" stroke="#FFFFFF" stroke-opacity=".35" stroke-width="1"/>')
bub = ""
for a, r in [(100, 7), (125, 5), (145, 9), (170, 4), (195, 6), (60, 4), (300, 5)]:
    x, y = pt(61, a)
    bub += (f'<circle cx="{x:.1f}" cy="{y:.1f}" r="{r}" fill="#7FD4FF" fill-opacity=".18" stroke="#BDEBFF" stroke-width="1.5"/>'
            f'<circle cx="{x - r * .35:.1f}" cy="{y - r * .35:.1f}" r="{max(1, r * .25):.1f}" fill="#FFFFFF" fill-opacity=".8"/>')
deco["bubbles"] = svg(136, 136, '<circle cx="68" cy="68" r="54" fill="none" stroke="#5BB8F2" stroke-opacity=".4" stroke-width="2"/>' + bub)
deco["headset"] = svg(136, 136,
    arc(60, -68, 68, stroke="#2B2B31", stroke_width=9, stroke_linecap="round") +
    arc(60, -68, 68, stroke="#3DDC97", stroke_width=2.5, stroke_linecap="round") +
    '<rect x="2" y="54" width="16" height="30" rx="7" fill="#2B2B31" stroke="#3DDC97" stroke-width="2"/>'
    '<rect x="118" y="54" width="16" height="30" rx="7" fill="#2B2B31" stroke="#3DDC97" stroke-width="2"/>'
    '<path d="M12 84C12 102 26 112 42 114" fill="none" stroke="#2B2B31" stroke-width="3" stroke-linecap="round"/>'
    '<circle cx="44" cy="114" r="4" fill="#3DDC97"/>')
hs = ""
for a, s, c in [(-30, 1.0, "#FF6B8A"), (0, 1.25, "#F25F5C"), (30, 1.0, "#FF8FB1"), (150, 0.7, "#FF8FB1"), (210, 0.7, "#FF6B8A"), (90, .6, "#FFB3C7"), (270, .6, "#FFB3C7")]:
    x, y = pt(60, a)
    hs += heart(x, y - 6 * s, s, c)
deco["hearts"] = svg(136, 136, '<circle cx="68" cy="68" r="55" fill="none" stroke="#FF8FB1" stroke-opacity=".5" stroke-width="2.5"/>' + hs)
bolt = lambda x, y, rot, s: f'<path transform="translate({x} {y}) rotate({rot}) scale({s})" d="M2-16L-8 2H0L-3 16L8-3H0Z" fill="#FFE14D" stroke="#B8860B" stroke-width="1" stroke-linejoin="round"/>'
deco["lightning"] = svg(136, 136,
    '<circle cx="68" cy="68" r="55" fill="none" stroke="#FFE14D" stroke-opacity=".6" stroke-width="2.5" stroke-dasharray="14 6"/>'
    + bolt(112, 22, 20, 1.3) + bolt(22, 114, 200, 1.1) + bolt(120, 92, 40, .7))
for k, v in deco.items():
    open(os.path.join(out, f"deco-{k}.svg"), "w", newline="\n").write(v)

# Effects: seamless 240x240 tiles. Each particle is drawn again one tile
# up and to the side where it would cross an edge, so tiling has no seams.
def tile(parts):
    body = ""
    for p in parts:
        for dx in (-240, 0, 240):
            for dy in (-240, 0, 240):
                body += p(dx, dy)
    return svg(240, 240, body)


random.seed(11)
eff = {}
ff = []
for _ in range(14):
    x, y, r = random.uniform(0, 240), random.uniform(0, 240), random.uniform(1.5, 3)
    ff.append(lambda dx, dy, x=x, y=y, r=r: f'<circle cx="{x + dx:.1f}" cy="{y + dy:.1f}" r="{r * 3:.1f}" fill="#FFE98A" fill-opacity=".14"/><circle cx="{x + dx:.1f}" cy="{y + dy:.1f}" r="{r:.1f}" fill="#FFF3B0"/>')
eff["fireflies"] = tile(ff)
sn = []
for _ in range(26):
    x, y, r = random.uniform(0, 240), random.uniform(0, 240), random.choice([1.2, 1.6, 2.2, 3])
    sn.append(lambda dx, dy, x=x, y=y, r=r: f'<circle cx="{x + dx:.1f}" cy="{y + dy:.1f}" r="{r}" fill="#FFFFFF" fill-opacity=".85"/>')
eff["snowfall"] = tile(sn)
eff["aurora"] = svg(480, 240,
    '<path d="M0 70C80 30 160 110 240 70S400 30 480 70V120C400 80 320 160 240 120S80 80 0 120Z" fill="url(#a1)" filter="url(#bl)"/>'
    '<path d="M0 40C100 80 140 10 240 40S380 80 480 40V70C380 110 340 40 240 70S100 110 0 70Z" fill="url(#a2)" filter="url(#bl)"/>',
    '<linearGradient id="a1" x1="0" x2="1"><stop offset="0" stop-color="#3DDC97" stop-opacity=".55"/><stop offset=".5" stop-color="#5BE5D3" stop-opacity=".35"/><stop offset="1" stop-color="#3DDC97" stop-opacity=".55"/></linearGradient>'
    '<linearGradient id="a2" x1="0" x2="1"><stop offset="0" stop-color="#9B7BFF" stop-opacity=".45"/><stop offset=".5" stop-color="#FF7BD5" stop-opacity=".3"/><stop offset="1" stop-color="#9B7BFF" stop-opacity=".45"/></linearGradient>'
    '<filter id="bl" x="-10%" y="-50%" width="120%" height="200%"><feGaussianBlur stdDeviation="8"/></filter>')
cf = []
for _ in range(22):
    x, y, rot = random.uniform(0, 240), random.uniform(0, 240), random.uniform(0, 180)
    col = random.choice(["#F25F5C", "#F2B84B", "#3DDC97", "#5BB8F2", "#B48CFF", "#FF8FB1"])
    cf.append(lambda dx, dy, x=x, y=y, rot=rot, col=col: f'<rect x="-3" y="-1.5" width="6" height="3" rx=".8" fill="{col}" transform="translate({x + dx:.1f} {y + dy:.1f}) rotate({rot:.0f})"/>')
eff["confetti"] = tile(cf)
rn = []
for _ in range(30):
    x, y, l = random.uniform(0, 240), random.uniform(0, 240), random.uniform(8, 16)
    rn.append(lambda dx, dy, x=x, y=y, l=l: f'<path d="M{x + dx:.1f} {y + dy:.1f}l-2 {l:.1f}" stroke="#A9DBFF" stroke-opacity=".55" stroke-width="1.2" stroke-linecap="round"/>')
eff["rain"] = tile(rn)
sp = []
for _ in range(12):
    x, y, r = random.uniform(0, 240), random.uniform(0, 240), random.uniform(3, 7)
    sp.append(lambda dx, dy, x=x, y=y, r=r: star4(x + dx, y + dy, r, "#FFF6D6"))
eff["sparkle"] = tile(sp)
bb = []
for _ in range(14):
    x, y, r = random.uniform(0, 240), random.uniform(0, 240), random.uniform(3, 8)
    bb.append(lambda dx, dy, x=x, y=y, r=r: f'<circle cx="{x + dx:.1f}" cy="{y + dy:.1f}" r="{r:.1f}" fill="#9FDcff" fill-opacity=".1" stroke="#CDEFFF" stroke-opacity=".7" stroke-width="1.2"/>')
eff["bubbles"] = tile(bb)
for k, v in eff.items():
    open(os.path.join(out, f"effect-{k}.svg"), "w", newline="\n").write(v)

# Nameplate patterns (the rest are CSS gradients).
plates = {}
plates["circuit"] = svg(64, 64,
    '<path d="M0 16H20L28 24H44M44 24V40L52 48H64M16 64V48L24 40H36M64 8H52L48 4" fill="none" stroke="#3DDC97" stroke-opacity=".5" stroke-width="1.5"/>'
    '<circle cx="44" cy="24" r="2.5" fill="#3DDC97" fill-opacity=".7"/><circle cx="24" cy="40" r="2.5" fill="#3DDC97" fill-opacity=".7"/><circle cx="48" cy="4" r="2" fill="#3DDC97" fill-opacity=".7"/>')
random.seed(3)
plates["stars"] = svg(96, 48, "".join(star4(random.uniform(4, 92), random.uniform(4, 44), random.uniform(1.5, 3.5), "#FFFFFF") for _ in range(9)))
plates["waves"] = svg(80, 24,
    '<path d="M0 12C10 4 30 4 40 12S70 20 80 12" fill="none" stroke="#5BB8F2" stroke-opacity=".55" stroke-width="2"/>'
    '<path d="M0 20C10 12 30 12 40 20S70 28 80 20" fill="none" stroke="#3DDC97" stroke-opacity=".35" stroke-width="2"/>')
for k, v in plates.items():
    open(os.path.join(out, f"plate-{k}.svg"), "w", newline="\n").write(v)

# Badges, 32x32.
badges = {}
badges["instance_admin"] = svg(32, 32,
    '<path d="M16 2L28 6V15C28 22.5 22.8 27.6 16 30C9.2 27.6 4 22.5 4 15V6Z" fill="#1F6B4E" stroke="#3DDC97" stroke-width="2" stroke-linejoin="round"/>'
    '<rect x="10" y="10" width="12" height="4" rx="1" fill="#3DDC97"/><rect x="10" y="16" width="12" height="4" rx="1" fill="#3DDC97"/>'
    '<circle cx="12.5" cy="12" r=".9" fill="#1F6B4E"/><circle cx="12.5" cy="18" r=".9" fill="#1F6B4E"/>')
badges["server_owner"] = svg(32, 32,
    '<path d="M4 24L6 9L12 15L16 6L20 15L26 9L28 24Z" fill="#F2B84B" stroke="#9A6B12" stroke-width="1.5" stroke-linejoin="round"/>'
    '<rect x="4" y="23" width="24" height="4" rx="1.5" fill="#E3A82B" stroke="#9A6B12" stroke-width="1.5"/><circle cx="16" cy="17" r="2" fill="#F25F5C"/>')
badges["early_member"] = svg(32, 32,
    '<circle cx="16" cy="20" r="8" fill="#FFB03A"/><path d="M2 22H30" stroke="#7CC4FF" stroke-width="2.5" stroke-linecap="round"/>'
    '<rect x="2" y="22.5" width="28" height="8" fill="#16161A"/>'
    + "".join(f'<path d="M{16 + 11 * math.cos(math.radians(a)):.1f} {20 - 11 * math.sin(math.radians(a)):.1f}L{16 + 14 * math.cos(math.radians(a)):.1f} {20 - 14 * math.sin(math.radians(a)):.1f}" stroke="#FFD37A" stroke-width="2" stroke-linecap="round"/>' for a in (30, 60, 90, 120, 150))
    + '<path d="M8 27H24" stroke="#7CC4FF" stroke-opacity=".6" stroke-width="1.5" stroke-linecap="round"/>')
hexagon = "M16 2L28 9V23L16 30L4 23V9Z"
for tier, (fill, edge, pips) in {1: ("#B8733E", "#E2A574", 1), 2: ("#8E98A8", "#D7DEE8", 2), 3: ("#D4A12A", "#FFE08A", 3), 5: ("#4CB8E6", "#C9F1FF", 5)}.items():
    marks = ""
    if tier == 5:
        marks = '<path d="M16 9L22 14L16 24L10 14Z" fill="#E8FBFF" stroke="#1C6E93" stroke-width="1"/><path d="M10 14H22M16 9L14 14L16 24L18 14Z" fill="none" stroke="#1C6E93" stroke-width=".8"/>'
    else:
        xs = {1: [16], 2: [12.5, 19.5], 3: [10, 16, 22]}[pips]
        marks = "".join(star4(x, 16, 4.2, "#FFFFFF") for x in xs)
    badges[f"years_{tier}"] = svg(32, 32, f'<path d="{hexagon}" fill="{fill}" stroke="{edge}" stroke-width="2" stroke-linejoin="round"/>' + marks)
badges["bug_reporter"] = svg(32, 32,
    '<ellipse cx="16" cy="18" rx="7" ry="9" fill="#F25F5C" stroke="#8C2A28" stroke-width="1.5"/><path d="M16 10V27" stroke="#8C2A28" stroke-width="1.5"/>'
    '<circle cx="16" cy="8" r="4" fill="#3A3A41"/><path d="M13 5L10 2M19 5L22 2M9 15H4M9 20H3M23 15H28M23 20H29M10 25L6 28M22 25L26 28" stroke="#3A3A41" stroke-width="2" stroke-linecap="round"/>'
    '<circle cx="13" cy="16" r="1.6" fill="#16161A"/><circle cx="19" cy="21" r="1.6" fill="#16161A"/>')
badges["contributor"] = svg(32, 32,
    '<rect x="2" y="4" width="28" height="24" rx="6" fill="#3B2A66" stroke="#B48CFF" stroke-width="2"/>'
    '<path d="M11 11L6 16L11 21M21 11L26 16L21 21M18 9L14 23" fill="none" stroke="#E2D4FF" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"/>')
os.makedirs(os.path.join(out, "..", "badges"), exist_ok=True)
for k, v in badges.items():
    open(os.path.join(out, "..", "badges", f"{k}.svg"), "w", newline="\n").write(v)
print(len(deco), "decorations,", len(eff), "effects,", len(plates), "plate patterns,", len(badges), "badges")
