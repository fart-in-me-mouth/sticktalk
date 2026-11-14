"""Deterministic contact sheets for inspecting every original headless PNG."""
from pathlib import Path
from PIL import Image, ImageDraw, ImageFont

root = Path(__file__).resolve().parents[2] / "screenshots" / "qa-calls"
destination = root / "review"
destination.mkdir(exist_ok=True)
paths = sorted(p for p in root.rglob("*.png") if destination not in p.parents)
font = ImageFont.truetype("C:/Windows/Fonts/segoeui.ttf", 16)
for start in range(0, len(paths), 6):
    sheet = Image.new("RGB", (1920, 1050), "#151519")
    draw = ImageDraw.Draw(sheet)
    for i, path in enumerate(paths[start:start + 6]):
        x, y = (i % 3) * 640, (i // 3) * 525
        image = Image.open(path).convert("RGB")
        image.thumbnail((630, 485))
        sheet.paste(image, (x + (640-image.width)//2, y + 28))
        draw.text((x + 5, y + 4), str(path.relative_to(root)), font=font, fill="white")
    sheet.save(destination / f"sheet-{start//6 + 1:02}.jpg", quality=92)
print(f"{len(paths)} images, {(len(paths)+5)//6} sheets: {destination}")
