"""Validate every proof PNG and create an inspectable index + review sheets."""
from collections import defaultdict
from pathlib import Path
import html
import json
from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'screenshots'
result = json.loads((OUT / 'browser-results.json').read_text(encoding='utf8'))
groups = defaultdict(list)
for item in result['screenshots']:
    file = ROOT / item['path']
    with Image.open(file) as image:
        image.verify()
    with Image.open(file) as image:
        assert image.size == (item['width'], item['height']), file
    groups[item['name']].append(item)

font = ImageFont.truetype(str(ROOT.parent / 'client/assets/fonts/Inter-Regular.ttf'), 18)
names = list(groups)
for start in range(0, len(names), 2):
    sheet = Image.new('RGB', (1536, 1280), '#111113')
    draw = ImageDraw.Draw(sheet)
    for row, name in enumerate(names[start:start+2]):
        draw.text((12,row*640+8),name,font=font,fill='#E8EAED')
        x = 12
        for item in groups[name]:
            with Image.open(ROOT / item['path']) as image:
                width = {1440:640,390:195,412:206,820:410}[item['width']]
                image.thumbnail((width,590),Image.Resampling.LANCZOS)
                sheet.paste(image,(x,row*640+40))
                x += width+12
    sheet.save(OUT / f'review-{start//2+1:02}.jpg',quality=92)

sections=[]
for name,items in groups.items():
    links=''.join(f'<a href="{Path(i["path"]).name}"><figure><img loading="lazy" src="{Path(i["path"]).name}" alt="{html.escape(name)} at {i["width"]} by {i["height"]}"><figcaption>{i["width"]} × {i["height"]}</figcaption></figure></a>' for i in items)
    sections.append(f'<section><h2>{html.escape(name)}</h2><div>{links}</div></section>')
(OUT / 'index.html').write_text('''<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>StickTalk UI proof</title><style>body{font:15px system-ui;background:#111113;color:#E8EAED;margin:24px}h1{font-size:20px}h2{font-size:16px}section{margin:32px 0}section>div{display:flex;gap:16px;align-items:flex-start;flex-wrap:wrap}a{color:#3DDC97}figure{margin:0}img{display:block;width:auto;max-width:360px;max-height:540px;border:1px solid #2A2A2F;border-radius:8px}figcaption{font-size:12px;margin:8px 0}p{max-width:700px;color:#A8A8B0}</style><h1>StickTalk · responsive UI proof</h1><p>Headless Chromium. Click a screenshot for its original PNG. Phone keyboard images use a labeled visualViewport / keyboard emulation; no OS keyboard or visible window was opened.</p>'''+''.join(sections)+'</html>',encoding='utf8')
print(f'Validated {len(result["screenshots"])} PNGs; wrote {len(range(0,len(names),2))} review sheets and screenshots/index.html')
