"""Plot present-return cadence and wheel trajectories (not physical scanout).
Usage: python scroll_timeline.py before.json after.json timeline.png
Requires Pillow; no client dependency or visible preview window.
"""
import json
import math
import sys
from pathlib import Path
from PIL import Image, ImageDraw, ImageFont

before, after = (json.loads(Path(p).read_text(encoding="utf-8-sig")) for p in sys.argv[1:3])
image = Image.new("RGB", (1600, 920), "#f8fafb")
draw = ImageDraw.Draw(image)
font_path = Path("C:/Windows/Fonts/segoeui.ttf")
font = lambda size: ImageFont.truetype(str(font_path), size) if font_path.exists() else ImageFont.load_default()
text = lambda xy, s, size=21, color="#263238": draw.text(xy, s, font=font(size), fill=color)
text((85, 24), "StickTalk | secondary-window wheel A/B", 34)
text((85, 75), "Present-return timing only: these are not physical displayed-frame/ETW timestamps.", 20)
colors = ["#919aa4", "#158861"]
for x, color, label in [(90, colors[0], "Before: native wheel jumps"), (620, colors[1], "After: additive 140 ms ease-out")]:
    draw.line((x, 120, x + 40, 120), fill=color, width=4)
    text((x + 50, 104), label)

def panel(rect, xlim, ylim, title, ylabel):
    left, top, right, bottom = rect
    text((left, top - 37), title, 23)
    def point(x, y):
        return (left + (x - xlim[0]) / (xlim[1] - xlim[0]) * (right - left), bottom - (y - ylim[0]) / (ylim[1] - ylim[0]) * (bottom - top))
    for n in range(6):
        y = ylim[0] + (ylim[1] - ylim[0]) * n / 5
        yy = point(xlim[0], y)[1]
        draw.line((left, yy, right, yy), fill="#dce3e7")
        text((15, yy - 13), f"{y:.1f}", 18)
        x = xlim[0] + (xlim[1] - xlim[0]) * n / 5
        xx = point(x, ylim[0])[0]
        text((xx - 15, bottom + 8), f"{x:.1f}", 18)
    text((left, bottom + 35), "Benchmark time (seconds)", 18)
    text((right - 240, top - 30), ylabel, 18)
    return point

series = []
for data in [before, after]:
    times = data["present_return_timestamps_ms"]
    series.append([(b / 1000, b - a) for a, b in zip(times, times[1:])])
interval = after["refresh_interval_ms"]
ymax = max(interval * 1.4, max(y for points in series for _, y in points)) * 1.05
p = panel((100, 208, 1520, 465), (1, 11), (0, ymax), "Frame-interval timeline", "Interval (ms)")
for value, color in [(interval, "#30414c"), (interval * 1.2, "#ce5a53")]:
    yy = p(1, value)[1]
    for xx in range(100, 1520, 14):
        draw.line((xx, yy, xx + 8, yy), fill=color, width=1)
for points, color in zip(series, colors):
    draw.line([p(x, y) for x, y in points], fill=color, width=2)
text((100, 540), f'Monitor: {after["refresh_hz"]:g} Hz | refresh {interval:.2f} ms | red dashed line = 1.2x target', 20)
trajectories = [[(t / 1000, y) for t, y in zip(d["present_return_timestamps_ms"], d["scroll_offsets"]) if 1.2 <= t / 1000 <= 2.2] for d in [before, after]]
ymin = min(y for points in trajectories for _, y in points)
ymax = max(y for points in trajectories for _, y in points)
p = panel((100, 655, 1520, 840), (1.2, 2.2), (ymin - 10, ymax + 10), "Wheel trajectory: the missing animation that frame-cost benchmarks cannot reveal", "Offset (logical pixels)")
for points, color in zip(trajectories, colors):
    draw.line([p(x, y) for x, y in points], fill=color, width=3)
image.save(sys.argv[3])

