from PIL import Image, ImageDraw
from pathlib import Path
root = Path(__file__).resolve().parents[2]
# Original StickTalk chat mark; vector-like construction at each native icon size.
def icon(size):
    scale = 4
    s = size * scale
    im = Image.new('RGBA', (s,s))
    d = ImageDraw.Draw(im)
    def box(values): return tuple(round(v*s/256) for v in values)
    d.rounded_rectangle(box((8,8,248,248)), radius=s*0.23, fill='#111113')
    d.rounded_rectangle(box((48,52,208,184)), radius=s*0.14, fill='#3DDC97')
    d.polygon([box((72,172)),box((72,213)),box((119,176))], fill='#3DDC97')
    for y in (98,134):
        d.rounded_rectangle(box((85,y,171,y+12)), radius=s*0.02, fill='#08170F')
    return im.resize((size,size), Image.Resampling.LANCZOS)
asset = root/'crates/client/assets'
icon(256).save(asset/'sticktalk.png')
icon(256).save(asset/'sticktalk.ico', sizes=[(s,s) for s in (16,20,24,32,40,48,64,128,256)])
