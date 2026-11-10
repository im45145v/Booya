"""Outline the 'booya' wordmark from Geist SemiBold (wght 600) so logo files never depend on installed fonts."""
from fontTools.ttLib import TTFont
from fontTools.varLib.instancer import instantiateVariableFont
from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen
import functools
FONT = '/workspace/booya-brand/fonts/Geist-Variable.ttf'

@functools.lru_cache()
def font():
    f = TTFont(FONT)
    return instantiateVariableFont(f, {'wght': 600})

TRACK = -0.035          # em, matches the CSS letter-spacing of the wordmark
KERN = {('y', 'a'): -0.012, ('o', 'y'): -0.004}  # small optical tweaks, em

def word_path(text='booya', size=100, x=0, y=0):
    """Return (svg path d, advance width, metrics) with the baseline at y."""
    f = font(); gs = f.getGlyphSet(); cmap = f.getBestCmap(); upm = f['head'].unitsPerEm
    sc = size/upm; pen = SVGPathPen(gs); cx = 0; prev = None
    for ch in text:
        gn = cmap[ord(ch)]
        if prev: cx += KERN.get((prev, ch), 0)*upm
        tp = TransformPen(pen, (sc, 0, 0, -sc, x + cx*sc, y))
        gs[gn].draw(tp)
        cx += gs[gn].width + TRACK*upm; prev = ch
    cx -= TRACK*upm
    os2 = f['OS/2']
    m = dict(xh=os2.sxHeight*sc, cap=os2.sCapHeight*sc, asc=f['hhea'].ascent*sc, desc=-f['hhea'].descent*sc)
    # ascender of 'b'
    from fontTools.pens.boundsPen import BoundsPen
    bp = BoundsPen(gs); gs[cmap[ord('b')]].draw(bp); m['b_top'] = bp.bounds[3]*sc
    bp = BoundsPen(gs); gs[cmap[ord('y')]].draw(bp); m['y_bot'] = -bp.bounds[1]*sc
    bp = BoundsPen(gs); gs[cmap[ord('b')]].draw(bp); m['b_left'] = bp.bounds[0]*sc
    return pen.getCommands(), cx*sc, m

if __name__ == '__main__':
    d, w, m = word_path(size=200, y=200)
    print(w, m)
