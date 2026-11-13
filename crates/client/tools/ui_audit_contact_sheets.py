from pathlib import Path
from PIL import Image,ImageDraw,ImageFont
import argparse
parser=argparse.ArgumentParser(description='Contact sheets for native iced UI audit PNGs (inspection only).')
parser.add_argument('source',type=Path);parser.add_argument('destination',type=Path)
parser.add_argument('--pattern',default='*.png')
args=parser.parse_args();args.destination.mkdir(parents=True,exist_ok=True)
font=ImageFont.truetype(str(Path(__file__).resolve().parents[1]/'assets/fonts/Inter-Regular.ttf'),14)
files=sorted(args.source.glob(args.pattern),key=lambda p:(p.name.split('-')[-1],p.name))
for start in range(0,len(files),12):
 sheet=Image.new('RGB',(1920,984),'#38383e');draw=ImageDraw.Draw(sheet)
 for i,path in enumerate(files[start:start+12]):
  with Image.open(path) as source:
   source=source.convert('RGB');source.thumbnail((480,300))
   x=i%4*480;y=i//4*328
   sheet.paste(source,(x+(480-source.width)//2,y+28))
   draw.text((x+8,y+6),path.name,font=font,fill='white')
 sheet.save(args.destination/f'sheet-{start//12:02}.png')
print(f'{len(files)} captures in {(len(files)+11)//12} sheets')
