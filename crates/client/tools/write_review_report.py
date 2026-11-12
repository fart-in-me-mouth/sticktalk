from pathlib import Path
import hashlib,json
root=Path(__file__).resolve().parents[1]
exe=Path('C:/Users/dwhee/.cargo-target/sticktalk-client/release/sticktalk.exe')
hash=hashlib.sha256(exe.read_bytes()).hexdigest().upper()
read=lambda name:json.loads((root/name).read_text(encoding='utf-8-sig'))
mib=lambda n:n/1048576
cases=[('GPU · 2560×1440','gpu'),('Software fallback · 2560×1440','tiny'),('GPU · 5,000-history channel','history'),('GPU · images','images')]
rows=[]
for label,key in cases:
 d=read(f'perf-final-{key}.json');assert d['sha256']==hash,(key,'not current release')
 rows.append((label,key,d))
before=read('scroll-before.json');contended=read('scroll-before-contended.json');after=read('scroll-after-gpu.json');tiny=read('scroll-after-tiny.json')
assert after['binary_sha256']==hash and tiny['binary_sha256']==hash
perf='''# Scrolling and idle performance

Default: **wgpu / D3D12**, iced automatic tiny-skia fallback. Screenshots remain tiny-skia/headless.

## 2560×1440 / 5,000-message scroll profile

Times in milliseconds. 60 warmups + 600 measured frames per trace. Software measurements rasterize the damaged chat region into a persistent pixmap; GPU measurements render the full offscreen texture and **wait for submission completion**, including driver/synchronization latency. No PNG encoding/readback or OS window. Offscreen traces use fractional wheel-sized offsets and randomized seek offsets, not a physical 280 Hz presentation measurement. Actual iced wheel/scrollbar pointer events are covered separately by the headless input test.

| Trace | Renderer | Layout p50 / p99 | Render p50 / p99 | End-to-end p50 / p99 |
|---|---|---:|---:|---:|
'''
for label,data in [('Before (earlier)',before),('Before (load repeat)',contended),('After',after),('After software fallback',tiny)]:
 for row in data['results']:
  val=lambda key:f'{row[key]["p50_ms"]:.2f} / {row[key]["p99_ms"]:.2f}'
  perf+=f'| {label} · {row["input"]} | {data["renderer"]} | {val("layout")} | {val("render")} | {val("total")} |\n'
perf+='''
[Earlier before raw](scroll-before.json) · [Before under-load repeat](scroll-before-contended.json) · [After GPU raw](scroll-after-gpu.json) · [After software raw](scroll-after-tiny.json)

![p99 end-to-end scrolling comparison](scroll-comparison.png)

The owner continued using the PC; system load was not controlled. The first final-revision run coincided with concurrent builds and a later ~83% total CPU sample: [GPU run 1](scroll-run1-gpu.json), [software run 1](scroll-run1-tiny.json). An earlier GPU prototype measured wheel total p50/p99 1.22/2.35 ms and render p99 1.86 ms, but **that is not this executable**. Both earlier and repeated baselines are preserved; do not infer a controlled causal speedup ratio or 280 Hz certification from unlike background loads.

### Bottlenecks and changes

- Software rasterization was the dominant cost (~19 ms median at this resolution), not attachment decoding. Random cold seeks additionally recreated/shaped message text and recalculated geometry.
- Default GPU rendering substantially reduces the software fill/raster bottleneck; it does not guarantee the requested final p99 target under concurrent load. D3D12 measured less residency than Vulkan and GL on this machine. Windows defaults to D3D12 without overriding an explicitly selected backend; GPU initialization failure follows iced's built-in software fallback. No animation/render ticker was introduced.
- Persistent, bounded **256-body shaped-paragraph + layout LRU** stores iced Trees across virtualization. Normal scroll hits reuse cosmic-text paragraphs/layout; content/spoiler/mention changes or width changes invalidate appropriately. Global stream sequence/typing/presence no longer invalidate message geometry. Keyed rows preserve normal widget continuity.
- Only visible message widgets + **four-row overscan** are built. Prefix heights use binary-search range lookup; per-pixel scrolling no longer reconstructs a full-history reference vector to render or resolve images.
- Static avatar/two-bar skeletons for missing/unlaid-out rows during fast movement; no shimmer. Viewport jumps **or event-rate-independent velocity** select the fast path, including small high-Hz scrollbar deltas. Evicted shaped-body entries also use placeholders instead of re-shaping while dragging. One event-driven **60 ms** settle timer (no idle polling) defers cold text preparation and image requests. The drag-skeleton trace measures that movement path; the cold-seek trace deliberately renders real content immediately and exposes remaining cold-layout cost. Do not treat these as identical workloads or claim the cold cost vanished.
- Thumbnail network/cache reads and image decode remain on bounded async/blocking workers; decoded RGBA handles are reused. No automatic original downloads.
- Up to **20,000 compact visited-message metadata entries** preserve IDs/heights/author/time through the **1,000-body / 8 MiB** content cap. Known evicted regions request the landing page directly, not intervening pages; stale seek results are rejected. Reload/index anchoring is regression-tested; prepend uses height-delta compensation. Current protocol exposes exclusive before/after IDs, **not total count/index/offset**: arbitrary jumps into never-visited history cannot be implemented honestly in the permitted client-only scope. Unknown older history still paginates normally.
- Native iced pixel/line deltas and thin 4px overlay scrollbars are retained. Subpixel sensor noise is ignored to prevent perpetual redraw feedback.

**Acceptance:** the table reports the final measured p99s, not a promise of <4 ms under concurrent system load. Physical foreground 280 Hz cadence remains unverified; cold content preparation still has a distinct cost after landing.

## Native idle residency

Real native processes, guarded non-primary/no-activation placement, ten-second warmup then 60 one-second samples. CPU uses one logical core, not all-core-normalized. Windows counter resolution applies. Each process is closed in `finally`; no trim/suspend tricks. The owner was using the PC, so GPU/desktop load was not controlled.

| Workload | Mean / max WS (MiB) | Mean private (MiB) | One-core idle CPU | Raw |
|---|---:|---:|---:|---|
'''
for label,key,d in rows:
 perf+=f'| {label} | {mib(d["working_set_mean"]):.2f} / {mib(d["working_set_max"]):.2f} | {mib(d["private_bytes_mean"]):.2f} | {d["one_core_cpu_percent"]:.3f}% | [JSON](perf-final-{key}.json) |\n'
perf+=f'''
**The ~90 MB GPU target is not met on this PC.** GPU is retained for the measured scrolling improvement, not represented as meeting the old <60 MB budget. Preliminary D3D12/Vulkan/GL comparison records remain in `perf-*-prototype.json`; those are an earlier prototype, not this executable. Do not compare process working set to GPU VRAM.

Final executable: {exe.stat().st_size:,} bytes; SHA-256 `{hash}`.
Final idle records all match this executable. [History bounds](perf-final-history-stats.json). Placement/foreground records: [GPU](perf-final-placement-gpu.json), [fallback](perf-final-placement-tiny.json), [history](perf-final-placement-history.json), [images](perf-final-placement-images.json).

### Limitations

No certification of physical 280 Hz cadence or live-server high-latency page loading. The frame benchmark intentionally holds 5,000 mock message bodies to isolate view/render cost; the native history workload separately verifies the production 1,000-body bound. The headless input test validates actual fractional wheel and scrollbar drag processing, not OS event delivery. Live TLS/WebSocket, maximal Unicode, image churn and all adversarial resource combinations remain unmeasured.

Reproduce with the commands in [README](README.md); native process sampler is `tools/measure.ps1`. All screenshots use `--mock --screenshots`, never a native capture window.
'''
(root/'PERF.md').write_text(perf,encoding='utf-8')
# Small code-native chart (no preview window/server).
series=[('Before wheel',contended['results'][0]['total']['p99_ms']),('After wheel',after['results'][0]['total']['p99_ms']),('Before cold seek',contended['results'][1]['total']['p99_ms']),('After cold seek',after['results'][1]['total']['p99_ms']),('After fast drag',after['results'][2]['total']['p99_ms'])]
factor=500/max(value for _,value in series)
svg='<svg xmlns="http://www.w3.org/2000/svg" width="760" height="258"><rect width="760" height="258" fill="#18181b"/><g fill="#eee" font-family="Arial" font-size="15"><text x="20" y="28">p99 end-to-end offscreen frame time (ms), lower is better</text>'
for i,(label,value) in enumerate(series):
 y=48+i*37
 svg+=f'<text x="20" y="{y+18}">{label}</text><rect x="185" y="{y}" width="{value*factor:.2f}" height="24" rx="3" fill="{"#71717a" if label.startswith("Before") else "#6f9e86"}"/><text x="{192+value*factor:.2f}" y="{y+18}">{value:.2f}</text>'
svg+='</g></svg>'; (root/'scroll-comparison.svg').write_text(svg,encoding='utf-8')

from PIL import Image,ImageDraw,ImageFont
chart=Image.new('RGB',(760,258),'#18181b');draw=ImageDraw.Draw(chart);font=ImageFont.truetype('C:/Windows/Fonts/segoeui.ttf',15)
draw.text((20,10),'p99 end-to-end offscreen frame time (ms), lower is better',fill='#eeeeee',font=font)
for i,(label,value) in enumerate(series):
 y=48+i*37
 draw.text((20,y+2),label,fill='#eeeeee',font=font)
 draw.rounded_rectangle((185,y,185+value*factor,y+24),radius=3,fill='#71717a' if label.startswith('Before') else '#6f9e86')
 draw.text((192+value*factor,y+2),f'{value:.2f}',fill='#eeeeee',font=font)
chart.save(root/'scroll-comparison.png')

report=(root/'tools/review_items.md').read_text(encoding='utf-8')
report+='''## Scrolling priority

Implemented GPU default/fallback, persistent shaped body/layout cache, keyed visible rows + overscan, indexed heights, static fast-drag/missing-history skeletons, landing-region keyset fetches, stale-response suppression and anchored replacements/prepends. Added the 5,000-message `scroll-5000` mock channel and window-free frame profiler. **Final measured render p99 remains above 4 ms; GPU RAM remains above ~90 MB; physical 280 Hz cadence is not certified. Unvisited arbitrary-history jumps need a server count/index API.** [Measurements and limitations](PERF.md).

## Proof

- **96 PNGs**, 48 states at 1280×800 and 1920×1080. All regenerated from the actual iced view tree with offscreen tiny-skia, no OS window. [Review / full list](UI_REVIEW.md), [dimension/hash manifest](screenshots/manifest.json).
- New requested states: google-login, pick-username, sign-in-methods, markdown, masked-link-confirm, context-own-message. Added scroll-5000, scroll-skeleton and expanded account/hover/settings cases.
- Passed: `cargo build -p sticktalk --release`; `cargo test -p sticktalk` (**35 passed**); `cargo clippy -p sticktalk --all-targets -- -D warnings`; `cargo fmt --all -- --check`. [Exact final commands](VALIDATION.txt). Earlier workspace formatting failures were resolved by the concurrent owners of those files; this task did not edit them.
'''
wheel=after['results'][0];drag=after['results'][2];gpu=rows[0][2]
report+=f'''- GPU wheel total p50/p99 **{wheel['total']['p50_ms']:.2f}/{wheel['total']['p99_ms']:.2f} ms**; fast drag **{drag['total']['p50_ms']:.2f}/{drag['total']['p99_ms']:.2f} ms**. Native GPU idle **{mib(gpu['working_set_mean']):.2f} MiB WS**, **{gpu['one_core_cpu_percent']:.3f}%** one-core CPU. [All before/after and fallback numbers](PERF.md).

## Remaining integration / deviations

The reaction picker is limited to nine bundled entries, not the requested full Unicode catalog/search. No live Google credentials/browser or real-server end-to-end certification. Mark Unread is session-local because v1 only supports monotonic read receipts. Google's approved gradient mark uses its official PNG fallback inside an SVG wrapper because resvg cannot render the source foreignObject/conic gradient. Arbitrary Unicode/assistive-technology coverage and OS picker/drop integration are not exhaustively tested. Compact continuation hover timestamps use HH:MM to avoid wrapping in the 40px gutter; tight-row action bars sit fully above text. [Asset provenance](ASSETS.md).

All automated screenshots and OAuth tests were window/browser-free. Native residency tests used only non-primary, no-activation mode and closed their own processes immediately after sampling; the owner's running preview was not touched.
'''
(root/'REPORT.md').write_text(report,encoding='utf-8')
p=root/'UI_REVIEW.md';s=p.read_text(encoding='utf-8');s=s[:s.index('| State |')].replace('46 states','48 states').replace('92 PNGs','96 PNGs')
s+='| State | 1280×800 | 1920×1080 |\n|---|---|---|\n'
for name in sorted({f.stem.rsplit('-',1)[0] for f in (root/'screenshots').glob('*.png')}):s+=f'| {name} | [PNG](screenshots/{name}-1280x800.png) | [PNG](screenshots/{name}-1920x1080.png) |\n'
p.write_text(s,encoding='utf-8')
print('Wrote final reports',hash)
