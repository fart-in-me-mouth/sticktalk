"""Index an inspected capture matrix and retain ten full-resolution comparisons."""

import hashlib
import json
from pathlib import Path
import re
import shutil

from PIL import Image


ROOT = Path(__file__).resolve().parents[1] / "screenshots" / "ui-audit"
PAIRS = [
    ("audit-long-names-940x600", "Measured ellipsis in dense desktop chrome"),
    ("audit-many-reactions-940x600", "Wrapped reactions without composer overflow"),
    ("audit-quick-long-940x600", "Bounded, two-line quick-switcher results"),
    ("emoji-picker-940x600", "Picker controls survive minimum window height"),
    ("change-server-940x600", "Server configuration stays inside the login card"),
    ("guild-settings-role-display-940x600", "Role navigation and tabs fit narrow settings"),
    ("audit-image-error-1280x800", "Centered image failure and recovery"),
    ("audit-search-empty-1280x800", "Honest search scope and actionable empty result"),
    ("notification-settings-muted-940x600", "Mute duration and Done remain visible"),
    ("audit-many-servers-1280x800", "Home and Add Server remain reachable"),
]


def metadata(path):
    with Image.open(path) as source:
        width, height = source.size
    return {
        "file": path.relative_to(ROOT).as_posix(),
        "width": width,
        "height": height,
        "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
    }


captures = []
states = {}
for directory in ["final", "scale-150"]:
    for path in sorted((ROOT / directory).glob("*.png")):
        match = re.fullmatch(r"(.+)-(\d+)x(\d+)(?:@(\d+)pct)?", path.stem)
        if not match:
            raise ValueError(f"Unrecognized capture filename: {path.name}")
        screen, width, height, scale = match.groups()
        scale = int(scale or 100) / 100
        item = metadata(path)
        item.update(
            screen=screen,
            logical_width=int(width),
            logical_height=int(height),
            scale_factor=scale,
            renderer="iced-tiny-skia",
        )
        assert (item["width"], item["height"]) == (
            round(int(width) * scale), round(int(height) * scale)
        ), path.name
        captures.append(item)
        if directory == "final":
            states.setdefault(screen, set()).add((int(width), int(height)))

sizes = {(940, 600), (1280, 800), (1920, 1080)}
assert len(states) == 190, len(states)
assert all(found == sizes for found in states.values())
for scale in [100, 150]:
    item = metadata(ROOT / f"native-{scale}.png")
    item.update(renderer="iced-wgpu", application_scale_factor=scale / 100)
    captures.append(item)

pairs = []
for number, (stem, issue) in enumerate(PAIRS, 1):
    filename = f"{number:02}-{stem}.png"
    pair = {"issue": issue, "source": f"{stem}.png"}
    for target, source in [("before", "baseline"), ("after", "final")]:
        (ROOT / target).mkdir(exist_ok=True)
        destination = ROOT / target / filename
        shutil.copyfile(ROOT / source / f"{stem}.png", destination)
        pair[target] = metadata(destination)
    pairs.append(pair)

inventory = {
    "screen_count": len(states),
    "logical_sizes": sorted(sizes),
    "offscreen_captures": len(captures) - 2,
    "native_captures": 2,
    "hash": "SHA-256 of PNG bytes",
    "captures": captures,
    "comparisons": pairs,
}
(ROOT / "capture-inventory.json").write_text(
    json.dumps(inventory, indent=2) + "\n", encoding="utf-8"
)
print(f"Indexed {len(captures)} captures and retained {len(pairs)} comparison pairs")
