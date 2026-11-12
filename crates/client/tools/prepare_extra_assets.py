import urllib.request,zipfile
from pathlib import Path
from fontTools.ttLib import TTFont
from fontTools.varLib.instancer import instantiateVariableFont
from fontTools import subset
root=Path('crates/client/assets/fonts')
url='https://raw.githubusercontent.com/google/fonts/main/ofl/inter/Inter-Italic%5Bopsz,wght%5D.ttf'
source=root/'InterVariable-Italic.ttf';urllib.request.urlretrieve(url,source)
for weight,style in [(400,'Italic'),(600,'SemiBoldItalic')]:
 f=instantiateVariableFont(TTFont(source),{'wght':weight,'opsz':14},inplace=True)
 for n in f['name'].names:
  if n.nameID in [1,16]:v='StickTalk UI'
  elif n.nameID in [2,17]:v=style
  elif n.nameID==4:v='StickTalk UI '+style
  elif n.nameID==6:v='StickTalkUI-'+style
  else:continue
  n.string=v.encode(n.getEncoding())
 f['OS/2'].usWeightClass=weight
 opts=subset.Options();opts.layout_features=['*'];opts.name_IDs=['*'];opts.name_legacy=True;opts.name_languages=['*']
 sub=subset.Subsetter(options=opts);sub.populate(unicodes=list(range(0x250))+list(range(0x2000,0x2070))+[0xfffd]);sub.subset(f);f.save(root/f'Inter-{style}.ttf')
mono_source=root/'JetBrainsMono-source.ttf'
if not mono_source.exists():urllib.request.urlretrieve('https://raw.githubusercontent.com/JetBrains/JetBrainsMono/master/fonts/ttf/JetBrainsMono-Regular.ttf',mono_source)
f=TTFont(mono_source)
for n in f['name'].names:
 if n.nameID in [1,16]:v='StickTalk Mono'
 elif n.nameID==4:v='StickTalk Mono Regular'
 elif n.nameID==6:v='StickTalkMono-Regular'
 else:continue
 n.string=v.encode(n.getEncoding())
opts=subset.Options();opts.layout_features=['*'];opts.name_IDs=['*'];opts.name_legacy=True;opts.name_languages=['*']
sub=subset.Subsetter(options=opts);sub.populate(unicodes=list(range(0x250))+list(range(0x2000,0x2070))+[0xfffd]);sub.subset(f);f.save(root/'JetBrainsMono-Regular.ttf')

# Google's approved gradient artwork uses SVG foreignObject/conic gradients unsupported by resvg.
# Keep the official PNG fallback unmodified in a portable SVG image wrapper.
import base64,io
from PIL import Image
archive=Path('crates/client/assets/google-signin-assets.zip')
if not archive.exists():urllib.request.urlretrieve('https://developers.google.com/static/identity/images/signin-assets.zip',archive)
z=zipfile.ZipFile(archive)
i=Image.open(io.BytesIO(z.read('Android + Web/PNG @4x/Light/Theme=Light, Show text=No, Shape=Square, Platform=Android+Web@4x.png')))
b=io.BytesIO();i.crop((40,40,120,120)).save(b,format='PNG')
Path('crates/client/assets/google-g.svg').write_text('<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="20" height="20" viewBox="0 0 80 80"><image width="80" height="80" xlink:href="data:image/png;base64,'+base64.b64encode(b.getvalue()).decode()+'"/></svg>',encoding='utf8')
