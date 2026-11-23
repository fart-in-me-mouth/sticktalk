"""Rebuild local fonts, Lucide sprite and PWA marks (no application network IO)."""
from pathlib import Path
from xml.etree import ElementTree as ET
from fontTools import subset
from fontTools.ttLib import TTFont
from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[1]
CLIENT = ROOT.parent / 'client' / 'assets'
PUBLIC = ROOT / 'public'
for name in ['Inter-Regular', 'Inter-Medium', 'Inter-SemiBold', 'JetBrainsMono-Regular']:
    font = TTFont(CLIENT / 'fonts' / (name + '.ttf'))
    options = subset.Options()
    options.flavor = 'woff2'
    sub = subset.Subsetter(options=options)
    sub.populate(unicodes=list(range(0x20, 0x100)) + list(range(0x2000, 0x2070)) + [0x2190, 0x2192, 0x2212])
    sub.subset(font)
    font.flavor = 'woff2'
    font.save(PUBLIC / 'fonts' / (name + '.woff2'))
    (PUBLIC / 'fonts' / (name + '.ttf')).unlink(missing_ok=True)

extra = ['volume-2','volume-x','mic','mic-off','headphones','headphone-off','phone','phone-incoming','phone-missed','phone-off','video','video-off','monitor','signal','pin','crown','lock','maximize','arrow-up-right','smartphone','key-round','eye','shield-check','check-check','bell','user-round','message-square','wifi-off']
vendored = ET.parse(ROOT / 'assets' / 'lucide-extra.svg').getroot()
extra_symbols = {node.attrib['id'][2:]: node for node in vendored.iter() if 'id' in node.attrib}
icons = {p.stem: ET.fromstring(p.read_text()) for p in (CLIENT / 'icons').glob('*.svg')}
for name in extra:
    if name not in icons:
        icons[name] = extra_symbols[name]
ET.register_namespace('', 'http://www.w3.org/2000/svg')
symbols = []
for name, svg in sorted(icons.items()):
    children = ''.join(ET.tostring(child, encoding='unicode').replace(' xmlns="http://www.w3.org/2000/svg"','') for child in svg)
    symbols.append(f'<symbol id="i-{name}" viewBox="0 0 24 24">{children}</symbol>')
sprite = '<svg xmlns="http://www.w3.org/2000/svg" class="svg-sprite" aria-hidden="true"><defs>' + ''.join(symbols) + '</defs></svg>'
(PUBLIC / 'icons' / 'sprite.svg').write_text(sprite)

for size, name in [(192,'icon-192.png'),(512,'icon-512.png'),(512,'maskable-512.png'),(180,'apple-touch-icon.png')]:
    im = Image.new('RGB', (size*3, size*3), '#111113')
    d = ImageDraw.Draw(im)
    s = size*3/512
    box = lambda xy: tuple(round(x*s) for x in xy)
    d.rounded_rectangle(box((92,102,420,358)), radius=round(68*s), fill='#3DDC97')
    d.polygon([box((150,338)),box((150,410)),box((230,348))],fill='#3DDC97')
    d.ellipse(box((229,148,283,202)), fill='#08170F')
    d.line([box((256,216)),box((256,284))],fill='#08170F',width=round(18*s))
    d.line([box((214,240)),box((298,240))],fill='#08170F',width=round(18*s))
    d.line([box((256,278)),box((223,316))],fill='#08170F',width=round(18*s))
    d.line([box((256,278)),box((289,316))],fill='#08170F',width=round(18*s))
    im.resize((size,size),Image.Resampling.LANCZOS).save(PUBLIC / 'icons' / name, optimize=True)
