"""Retain raw diagnostics without committing duplicate uncompressed traces."""
from pathlib import Path
from zipfile import ZipFile, ZIP_DEFLATED

root = Path(__file__).resolve().parent
paths = sorted(p for p in root.rglob("*.log")
               if "web-probe" not in p.parts and p.name != "devices.log")
probe = root / "web-probe" / "calls.mjs"
if probe.exists():
    paths.append(probe)
with ZipFile(root / "trace-logs.zip", "w", ZIP_DEFLATED, compresslevel=6) as archive:
    for path in paths:
        archive.write(path, path.relative_to(root))
print(f"Archived {len(paths)} files; device endpoint names intentionally excluded")
