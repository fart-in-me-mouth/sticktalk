"""Resolve stripped release allocation stacks using the build's /MAP and ASLR base."""
import argparse
from bisect import bisect_right
import json
from pathlib import Path
import re
import subprocess

parser = argparse.ArgumentParser()
parser.add_argument("profile", type=Path)
parser.add_argument("map", type=Path)
parser.add_argument("output", type=Path)
parser.add_argument("--demangler", type=Path)
parser.add_argument("--method", default="DHAT Rust allocations; checkpoint retains App/view-cache/GPU renderer. Native driver allocations and mapped image pages are NOT included. Instrumented process RAM is NOT shipping RAM.")
args = parser.parse_args()
profile = json.loads(args.profile.read_text())
image = json.loads(Path(str(args.profile) + ".image-base.json").read_text())["image_base"]
text = args.map.read_text(errors="replace")
preferred = int(re.search(r"Preferred load address is\s+([0-9A-Fa-f]+)", text)[1], 16)
symbols = []
for match in re.finditer(r"^\s+[0-9A-Fa-f]{4}:[0-9A-Fa-f]+\s+(\S+)\s+([0-9A-Fa-f]{16})\s+f\s+", text, re.M):
    symbols.append((int(match[2], 16), match[1]))
symbols.sort()
if args.demangler:
    result = subprocess.run([str(args.demangler)], input="\n".join(s[1] for s in symbols), text=True, capture_output=True, check=True)
    names = result.stdout.splitlines()
    assert len(names) == len(symbols)
    symbols = [(address, name) for (address, _), name in zip(symbols, names)]
addresses = [s[0] for s in symbols]

def resolve(frame):
    match = re.search(r"0x([0-9A-Fa-f]+)", frame)
    if match:
        address = int(match[1], 16) - image + preferred
        index = bisect_right(addresses, address) - 1
        if 0 <= index < len(symbols) and address - addresses[index] < 0x100000:
            return f"{symbols[index][1]}+0x{address-addresses[index]:x}"
    return frame

frames = [resolve(frame) for frame in profile["ftbl"]]
points = [{"live_bytes": p.get("eb", 0), "peak_live_bytes": p.get("gb", 0), "total_bytes": p["tb"],
           "live_blocks": p.get("ebk", 0), "stack": [frames[i] for i in p["fs"]]} for p in profile["pps"]]
profile["ftbl"] = frames
args.profile.with_name(args.profile.stem + "-symbolized.json").write_text(json.dumps(profile))
summary = {"method": args.method,
           "live_bytes": sum(p["live_bytes"] for p in points), "peak_live_bytes": sum(p["peak_live_bytes"] for p in points),
           "allocation_sites": len(points), "largest_live_sites": sorted(points, key=lambda p: p["live_bytes"], reverse=True)[:40],
           "largest_peak_sites": sorted(points, key=lambda p: p["peak_live_bytes"], reverse=True)[:20]}
args.output.write_text(json.dumps(summary, indent=2))
print(json.dumps({k:v for k,v in summary.items() if not isinstance(v,list)}, indent=2))
