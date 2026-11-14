"""Summarize measured decoder output; never substitute sender FPS for a viewer."""
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent


def read(path):
    return json.loads(path.read_text(encoding="utf-8-sig"))


def summarize():
    runs = read(ROOT / "matrix-results.json")
    if isinstance(runs, dict):
        runs = [runs]
    rows = []
    for run in runs:
        folder = ROOT / run["Case"]
        spec = read(folder / "run.json")
        row = dict(case=run["Case"], exit_code=run["ExitCode"], **spec)
        for role in ("sharer", "viewer_b", "viewer_c"):
            path = folder / f"{role}.metrics.json"
            if not path.exists():
                row[f"{role}_missing"] = True
                continue
            data = read(path)
            row[role] = dict(
                avg_fps=data["frames"] / data["seconds"],
                min_fps=min(data["fps_by_second"]),
                drift_ms=data["drift_ms"],
                reconfigurations=data["encoder_reconfigurations"],
                decode_errors=data["decode_errors"],
                cpu_machine_pct=data["cpu_machine_pct"],
                working_set_mib=data["rss_bytes"] / 1048576,
                private_mib=data["private_bytes"] / 1048576,
                payload_mbps=data["payload_bytes"] * 8 / data["seconds"] / 1e6,
            )
        # A PowerShell wrapper losing a native ExitCode must never turn a
        # failed media assertion into a green report. Validate every metric.
        failures = []
        for role in ("sharer", "viewer_b", "viewer_c"):
            data = row.get(role)
            if not data:
                failures.append(f"{role}: missing metrics")
            elif (data["avg_fps"] < spec["fps"] * 58 / 60
                  or data["min_fps"] < spec["fps"] * 45 / 60
                  or abs(data["drift_ms"]) >= 20):
                failures.append(f"{role}: FPS/drift gate")
        if row.get("sharer", {}).get("reconfigurations") != 0:
            failures.append("encoder reconfigurations")
        row["metric_failures"] = failures
        row["passed"] = run["ExitCode"] == 0 and not failures
        rows.append(row)
    (ROOT / "acceptance-summary.json").write_text(
        json.dumps(rows, indent=2) + "\n", encoding="utf-8"
    )
    lines = [
        "| Source | Preset | Seconds | B avg/min fps | C avg/min fps | B/C drift ms | Reconfigs | Result |",
        "|---|---|---:|---:|---:|---:|---:|---|",
    ]
    for row in rows:
        b, c, s = [row.get(role, {}) for role in ("viewer_b", "viewer_c", "sharer")]
        def fps(d):
            return f'{d["avg_fps"]:.3f}/{d["min_fps"]}' if d else "missing"
        def drift(d):
            return f'{d["drift_ms"]:+.3f}' if d else "missing"
        lines.append(
            f'| {row["source"]} | {row["quality"]}p/{row["fps"]} | {row["seconds"]} '
            f'| {fps(b)} | {fps(c)} | {drift(b)}/{drift(c)} '
            f'| {s.get("reconfigurations", "missing")} | {"PASS" if row["passed"] else "FAIL"} |'
        )
    (ROOT / "acceptance-table.md").write_text("\n".join(lines) + "\n", encoding="utf-8")
    print("\n".join(lines))


if __name__ == "__main__":
    summarize()
