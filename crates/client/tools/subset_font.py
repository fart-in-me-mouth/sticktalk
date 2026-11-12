"""Subset Inter (SIL OFL) to compact static Latin UI faces; requires fonttools."""
from pathlib import Path
from fontTools.ttLib import TTFont
from fontTools.varLib.instancer import instantiateVariableFont
from fontTools import subset
root=Path(__file__).resolve().parents[1]/'assets/fonts'
for weight,style in [(400,'Regular'),(500,'Medium'),(600,'SemiBold')]:
 f=instantiateVariableFont(TTFont(root/'InterVariable.ttf'),{'wght':weight,'opsz':14},inplace=True)
 for n in f['name'].names:
  if n.nameID in [1,16]:v='StickTalk UI'
  elif n.nameID in [2,17]:v=style
  elif n.nameID==4:v='StickTalk UI '+style
  elif n.nameID==6:v='StickTalkUI-'+style
  else:continue
  n.string=v.encode(n.getEncoding())
 f['OS/2'].usWeightClass=weight
 opts=subset.Options();opts.layout_features=['*'];opts.name_IDs=['*'];opts.name_legacy=True;opts.name_languages=['*']
 sub=subset.Subsetter(options=opts);sub.populate(unicodes=list(range(0x250))+list(range(0x2000,0x2070))+list(range(0x20a0,0x20d0))+[0xfffd]);sub.subset(f);f.save(root/f'Inter-{style}.ttf')
