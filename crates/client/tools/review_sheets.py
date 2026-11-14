import os
from pathlib import Path
from PIL import Image,ImageDraw,ImageFont
root=Path('crates/client/screenshots');out=(Path(os.environ.get('CARGO_TARGET_DIR', Path(__file__).resolve().parents[3] / 'target')) / 'review');out.mkdir(parents=True,exist_ok=True)
scenes=sorted({f.stem.rsplit('-',1)[0] for f in root.glob('*.png')})
font=ImageFont.truetype('C:/Windows/Fonts/segoeui.ttf',18)
for index in range(0,len(scenes),2):
 sheet=Image.new('RGB',(1920,1240),'#111113');d=ImageDraw.Draw(sheet)
 for row,name in enumerate(scenes[index:index+2]):
  for col,size in enumerate(['1280x800','1920x1080']):
   im=Image.open(root/f'{name}-{size}.png').convert('RGB');im.thumbnail((960,580));x=col*960;y=row*620+30;sheet.paste(im,(x,y));d.text((x+8,row*620+5),f'{name} / {size}',font=font,fill='white')
 sheet.save(out/f'sheet-{index//2+1:02}.jpg',quality=94)
print('Contact sheets:',len(scenes)//2)
