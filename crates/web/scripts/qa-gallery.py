"""Contact sheets from unmodified real-browser PNGs; no fixture rendering."""
from pathlib import Path
from math import ceil
import json
from PIL import Image, ImageDraw, ImageFont

root = Path(__file__).resolve().parents[1] / "screenshots" / "qa"
font = ImageFont.truetype("C:/Windows/Fonts/segoeui.ttf", 13)
index = {}


def sheet(paths, name, columns=4, width=210, height=455):
    cell_width, cell_height = width + 24, height + 56
    out = Image.new("RGB", (columns * cell_width, ceil(len(paths) / columns) * cell_height), "#111113")
    draw = ImageDraw.Draw(out)
    for i, path in enumerate(paths):
        screenshot = Image.open(path).convert("RGB")
        screenshot.thumbnail((width, height), Image.Resampling.LANCZOS)
        x, y = (i % columns) * cell_width + 12, (i // columns) * cell_height + 40
        out.paste(screenshot, (x, y))
        label = str(path.relative_to(root)).replace("\\", "/").removesuffix(".png")
        draw.text((x, y - 34), label.rsplit("-", 1)[0][:32], font=font, fill="#E8EAED")
        draw.text((x, y - 17), label.rsplit("-", 1)[-1], font=font, fill="#A8A8B0")
    out.save(root / name, optimize=True)
    index[name] = [str(p.relative_to(root)).replace("\\", "/") for p in paths]


for size in ["390x844", "412x915", "820x1180", "1440x900", "1280x800", "1920x1080"]:
    paths = sorted([*root.glob(f"*-{size}.png"), *(root / "live").glob(f"*-{size}.png")])
    if size.startswith(("390", "412")):
        paths += sorted(root.glob(f"*-{size.split(chr(120))[0]}x440.png"))
    paths = [p for p in paths if "failure" not in p.name]
    for start in range(0, len(paths), 12):
        sheet(paths[start:start + 12], f"contact-{size}-{start // 12 + 1:02}.png",
              width=210 if size.startswith(("390", "412")) else 300,
              height=455 if size.startswith(("390", "412")) else 400 if size.startswith("820") else 220)

featured = [
    "login-390x844.png", "live/grouped-chat-390x844.png", "message-actions-390x844.png",
    "screen-share-phone-390x844.png", "settings-390x844.png", "profile-edit-390x844.png",
    "appearance-390x844.png", "notifications-390x844.png", "server-profile-saved-390x844.png",
    "roles-reordered-390x844.png", "bans-populated-390x844.png", "offline-shell-390x844.png",
]
sheet([root / p for p in featured if (root / p).exists()], "contact-phone.png")
(root / "contact-index.json").write_text(json.dumps(index, indent=2), encoding="utf-8")
print(f"{sum(len(v) for k, v in index.items() if k != 'contact-phone.png')} screenshots, {len(index)} sheets")

# Reproducible comparison chart; raw values remain in the paired JSON files.
if (root / "lighthouse-summary.json").exists() and (root / "bundle-sizes.json").exists():
    before = json.loads((root / "baseline/lighthouse-summary.json").read_text(encoding="utf-8"))
    after = json.loads((root / "lighthouse-summary.json").read_text(encoding="utf-8"))
    old_sizes = json.loads((root / "baseline/bundle-sizes.json").read_text(encoding="utf-8"))
    new_sizes = json.loads((root / "bundle-sizes.json").read_text(encoding="utf-8"))
    def core(sizes):
        return next(a["brotli"] for a in sizes["assets"] if a["file"].endswith("_bg.wasm"))
    rows = [(label, before[key]["metrics"]["interactive"]["value"], after[key]["metrics"]["interactive"]["value"], "ms", 6000)
            for key, label in [("mobile", "Cold progressive login"), ("authenticatedChatMobile", "Cold authenticated chat"),
                               ("warmAuthenticatedChatMobile", "Warm authenticated chat"), ("desktop", "Desktop progressive login")]]
    rows += [("Core WASM Brotli", core(old_sizes), core(new_sizes), "B", 900000),
             ("Installable shell cache", old_sizes["firstLoadBytes"], new_sizes["firstLoadBytes"], "B", 900000),
             ("All assets Brotli", old_sizes["compressed"], new_sizes["compressed"], "B", 900000)]
    chart = Image.new("RGB", (1120, 870), "#19191C"); draw = ImageDraw.Draw(chart)
    title = ImageFont.truetype("C:/Windows/Fonts/segoeuib.ttf", 26)
    body = ImageFont.truetype("C:/Windows/Fonts/segoeui.ttf", 18)
    draw.text((28, 20), "StickTalk web: before / after QA", font=title, fill="#E8EAED")
    draw.text((28, 58), "Same-method Lighthouse runs on a shared Windows host; not physical-phone certification.", font=body, fill="#A0A0AA")
    for i, (label, old, new, unit, maximum) in enumerate(rows):
        y = 115 + i * 99
        draw.text((28, y), label, font=body, fill="#E8EAED")
        for value, offset, color, prefix in [(old, 28, "#71717B", "Before"), (new, 55, "#3DDC97", "After")]:
            width = min(650, value / maximum * 650)
            draw.rounded_rectangle((325, y + offset, 325 + width, y + offset + 18), radius=3, fill=color)
            value_label = f"{value:,.0f} {unit}"
            draw.text((990, y + offset - 4), value_label, font=body, fill="#E8EAED", anchor="ra")
            draw.text((1030, y + offset - 4), prefix, font=body, fill=color)
    draw.text((28, 830), "Shell cache bytes are NOT cold-login network transfer. Byte counts include lazy assets.", font=body, fill="#A0A0AA")
    chart.save(root / "performance.png", optimize=True)
