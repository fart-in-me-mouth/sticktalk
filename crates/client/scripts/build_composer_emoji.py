"""Rebuild the native composer emoji face from our already licensed Twemoji PNGs.
Requires fontTools. sbix keeps native shaping, grapheme selection, caret and IME.
The face deliberately has 22px artwork/advance at the 15px UI body size.
"""
from pathlib import Path
import re
from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.ttLib import newTable
from fontTools.ttLib.tables.sbixStrike import Strike
from fontTools.ttLib.tables.sbixGlyph import Glyph
from fontTools.feaLib.builder import addOpenTypeFeaturesFromString

root = Path(__file__).resolve().parents[1]
entries = [(key, int(start), int(length)) for key, start, length in re.findall(r'\("([0-9a-f-]+)",\s*(\d+),\s*(\d+)\)', (root/'assets/emoji/twemoji-index.rs').read_text())]
assert len(entries) > 3000
blob = (root/'assets/emoji/twemoji.bin').read_bytes()
sequences = [(tuple(int(v,16) for v in key.split('-')), start, length) for key,start,length in entries]
points = sorted({v for seq,_,_ in sequences for v in seq})
name = lambda cp: f'u{cp:X}'
order = ['.notdef'] + [name(cp) for cp in points] + [f'emoji{i}' for i in range(len(sequences))]
fb = FontBuilder(1024, isTTF=True)
fb.setupGlyphOrder(order)
single = {seq[0]: f'emoji{i}' for i,(seq,_,_) in enumerate(sequences) if len(seq)==1}
fb.setupCharacterMap({cp:single.get(cp,name(cp)) for cp in points})
empty = TTGlyphPen(None).glyph()
fb.setupGlyf({glyph:empty for glyph in order})
advance = round(1024*22/15)
fb.setupHorizontalMetrics({glyph:(advance,0) for glyph in order})
# A 72px bitmap at a 49ppem strike occupies 22.04px at body size 15.
# Its bottom is 5px below the Inter baseline, not top-aligned to the line.
fb.setupHorizontalHeader(ascent=1100, descent=-400)
fb.setupNameTable({'familyName':'StickTalk Composer Emoji','styleName':'Regular','uniqueFontIdentifier':'StickTalkComposerEmoji-v1','fullName':'StickTalk Composer Emoji','psName':'StickTalkComposerEmoji','version':'Version 1.0','copyright':'Twemoji graphics CC-BY 4.0, Twitter and contributors. Font assembly MIT.'})
fb.setupOS2(sTypoAscender=1100,sTypoDescender=-400,usWinAscent=1200,usWinDescent=450)
fb.setupPost(); fb.setupMaxp()
font=fb.font
sbix=newTable('sbix'); sbix.version=1; sbix.flags=1
strike=Strike(ppem=49,resolution=72)
ligatures=[]
for i,(seq,start,length) in enumerate(sequences):
    target=f'emoji{i}'
    image=blob[start:start+length]
    strike.glyphs[target]=Glyph(glyphName=target,originOffsetX=0,originOffsetY=-17,graphicType='png ',imageData=image)
    if len(seq)>1:
        ligatures.append('sub '+' '.join(single.get(cp,name(cp)) for cp in seq)+' by '+target+';')
sbix.strikes={49:strike}; font['sbix']=sbix
# Longest sequence wins, including flags, skin tones and ZWJ families.
addOpenTypeFeaturesFromString(font,'feature liga {\n'+'\n'.join(sorted(ligatures,key=len,reverse=True))+'\n} liga;')
font.save(root/'assets/fonts/ComposerEmoji.ttf')
print(f'{len(entries)} emoji, {(root/"assets/fonts/ComposerEmoji.ttf").stat().st_size} bytes')

# Zero-ink custom-token advances. Actual server artwork is drawn by the composer
# overlay using the exact same native editor layout; no dynamic/system fonts.
fb = FontBuilder(1024, isTTF=True)
order = ['.notdef'] + [f'ascii{cp}' for cp in range(32,127)]
fb.setupGlyphOrder(order)
fb.setupCharacterMap({cp:f'ascii{cp}' for cp in range(32,127)})
fb.setupGlyf({glyph:empty for glyph in order})
fb.setupHorizontalMetrics({glyph:(advance if glyph=='ascii60' else 0,0) for glyph in order})
fb.setupHorizontalHeader(ascent=1100,descent=-400)
fb.setupNameTable({'familyName':'StickTalk Custom Advance','styleName':'Regular','uniqueFontIdentifier':'StickTalkCustomAdvance-v1','fullName':'StickTalk Custom Advance','psName':'StickTalkCustomAdvance'})
fb.setupOS2(sTypoAscender=1100,sTypoDescender=-400,usWinAscent=1200,usWinDescent=450)
fb.setupPost(); fb.setupMaxp()
fb.font.save(root/'assets/fonts/CustomEmojiAdvance.ttf')
