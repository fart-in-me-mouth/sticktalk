"""Recalculate qualification from saved per-second, per-process measurements."""
import json
import re
from pathlib import Path

root = Path(__file__).resolve().parent
rows = []
for path in sorted(root.glob("*/*.metrics.json")):
    data = json.loads(path.read_text())
    fps = data["frames"] / data["seconds"]
    minimum = min(data["fps_by_second"])
    sharer = data["role"] == "sharer"
    passed = fps >= 58 and minimum >= 50 and abs(data["drift_ms"]) < 20
    if sharer:
        passed &= data["cpu_machine_pct"] < 5 and data["rss_delta_bytes"] < 150_000_000
    rows.append({
        "case": path.parent.name, "role": data["role"],
        "duration": len(data["fps_by_second"]), "fps": fps, "minimum_1s_fps": minimum,
        "drift_ms": data["drift_ms"], "cpu_machine_pct": data["cpu_machine_pct"],
        "working_set_delta_mib": data["rss_delta_bytes"] / 1048576,
        "private_delta_mib": data["private_delta_bytes"] / 1048576,
        "payload_mbps": data["payload_bytes"] * 8 / data["seconds"] / 1_000_000,
        "pass": bool(passed),
    })

diagnostics = {}
for path in sorted(root.glob("*.log")):
    if path.stem.endswith((".stdout", ".trace")):
        continue
    content = path.read_text(errors="replace")
    samples = re.findall(r"VIDEO_TRACE sfu_peer .*?cap_drops=(\d+) (?:media_cap_drops=\d+ )?queue_drops=(\d+) mtu_drops=(\d+)", content)
    if samples:
        diagnostics[path.stem] = {
            "max_cap_drops": max(int(row[0]) for row in samples),
            "max_queue_drops": max(int(row[1]) for row in samples),
            "max_mtu_drops": max(int(row[2]) for row in samples),
        }
        peers = {}
        for peer, seconds, fields in re.findall(r"VIDEO_TRACE sfu_peer peer=(.*?) seconds=(\S+) (.*)", content):
            values = dict(re.findall(r"(\w+)=(\S+)", fields))
            record = peers.setdefault(peer, {"bwe_min_after_10s": None, "bwe_max_after_10s": None})
            record["last"] = {key: float(value) for key, value in values.items()}
            if float(seconds) > 10:
                bwe = int(values["bwe"])
                record["bwe_min_after_10s"] = min(record["bwe_min_after_10s"] or bwe, bwe)
                record["bwe_max_after_10s"] = max(record["bwe_max_after_10s"] or bwe, bwe)
        diagnostics[path.stem]["peers"] = peers
        hops = {}
        for hop, peer, mid, fields in re.findall(r"VIDEO_TRACE (sfu_in|sfu_out) peer=(.*?) mid=(\S+) seconds=\S+ (.*)", content):
            hops[f"{hop} {peer} mid={mid}"] = {key: int(value) for key, value in re.findall(r"(\w+)=(\d+)", fields)}
        diagnostics[path.stem]["last_per_hop_counters"] = hops

result = {"endpoints": rows, "sfu_diagnostics": diagnostics}
(root / "summary.json").write_text(json.dumps(result, indent=2) + "\n")
for row in rows:
    print(f"{row['case']:30} {row['role']:8} {row['fps']:7.3f} fps min={row['minimum_1s_fps']:2} drift={row['drift_ms']:+8.3f}ms CPU={row['cpu_machine_pct']:.3f}% WS+={row['working_set_delta_mib']:.2f}MiB private+={row['private_delta_mib']:.2f}MiB {'PASS' if row['pass'] else 'FAIL'}")
