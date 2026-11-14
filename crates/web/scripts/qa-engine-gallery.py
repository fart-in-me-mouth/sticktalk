"""Review sheets from the current run's result-listed real-browser screenshots."""
from pathlib import Path
from math import ceil
import json
import sys
from PIL import Image, ImageDraw, ImageFont

web = Path(__file__).resolve().parents[1]
root = Path(sys.argv[1] if len(sys.argv) > 1 else web / "screenshots/engines/latest").resolve()
font = next((ImageFont.truetype(str(p), 13) for p in [Path("C:/Windows/Fonts/segoeui.ttf"),
    Path("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf")] if p.exists()), ImageFont.load_default())
index = {}
for engine in ["webkit", "chromium", "firefox"]:
    paths = []
    for result in sorted((root / engine / "run-1").glob("*/*-results.json")):
        data = json.loads(result.read_text(encoding="utf-8"))
        for filename in data.get("screenshots", []):
            path = Path(filename)
            if not path.is_absolute():
                path = web / path
            path = path.resolve()
            if path.is_relative_to(root) and path.is_file():
                paths.append(path)
    paths = sorted(set(paths))
    for start in range(0, len(paths), 12):
        group = paths[start:start + 12]
        sheet = Image.new("RGB", (1000, ceil(len(group) / 4) * 470), "#111113")
        draw = ImageDraw.Draw(sheet)
        for i, path in enumerate(group):
            screenshot = Image.open(path).convert("RGB")
            screenshot.thumbnail((235, 430), Image.Resampling.LANCZOS)
            x, y = i % 4 * 250 + 7, i // 4 * 470 + 32
            sheet.paste(screenshot, (x, y))
            draw.text((x, y - 26), path.stem[:32], font=font, fill="#E8EAED")
            draw.text((x, y + 432), path.parent.name, font=font, fill="#A8A8B0")
        name = f"contact-{engine}-{start // 12 + 1:02}.png"
        sheet.save(root / name, optimize=True)
        index[name] = [str(p.relative_to(root)).replace("\\", "/") for p in group]
    print(f"{engine}: {len(paths)} screenshots")
(root / "contact-index.json").write_text(json.dumps(index, indent=2), encoding="utf-8")
