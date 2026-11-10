from geom import *
from wordmark import word_path
from common import *
CAP = 100  # design size: wordmark cap height (= 'b' ascender) in units
FS = CAP/0.71   # font size giving cap 100 (Geist cap = 0.71 em)

VARIANTS = {
  'color':      dict(ink=INK, sig=SIGNAL, word=INK, bg=None),
  'on-paper':   dict(ink=INK, sig=SIGNAL, word=INK, bg=PAPER),
  'on-ink':     dict(ink='#EDEBE5', sig='#FF6A33', word='#EDEBE5', bg=INK),
  'mono-black': dict(ink='#000000', sig='#000000', word='#000000', bg=None),
  'mono-white': dict(ink='#FFFFFF', sig='#FFFFFF', word='#FFFFFF', bg=None),
}
def body_scale(h):  # body height h -> s
    return h/(G['stem'] + G['amp']*1.2)
RIPX = G['rip_r'] + G['rip_w']/2   # ripple reach, unit space

def compose(kind, v):
    """Returns (inner svg, width, height). Everything includes clear space of one eye width."""
    if kind == 'icon':
        s = body_scale(200); e = 2*G['eye_r']*s
        x0 = -RIPX*s; y0 = -RIPX*s; bw = G['shoulder']*s; bh = (G['stem']+G['amp']*1.2)*s
        W = bw - x0 + 2*e; H = bh - y0 + 2*e
        ox, oy = e - x0, e - y0
        return mark(ox, oy, s, v['ink'], v['sig'], soft=0.035), W, H, e
    d, ww, m = word_path('booya', FS, 0, 0)
    if kind == 'wordmark':
        e = 2*G['eye_r']*body_scale(CAP*1.2)
        top = m['b_top']; bot = m['y_bot']
        W = ww + 2*e; H = top + bot + 2*e
        return f'<path transform="translate({e:.2f} {e+top:.2f})" d="{d}" fill="{v["word"]}"/>', W, H, e
    if kind == 'horizontal':
        bh = CAP*1.58; s = body_scale(bh); e = 2*G['eye_r']*s
        gap = CAP*0.42
        x0 = -RIPX*s
        # mark: hem bottom sits a hair under the baseline, tip rises above cap height
        base = 0
        oy_tip = base + CAP*0.10 - bh
        top = min(oy_tip - RIPX*s, -m['b_top'])
        ox_tip = -x0
        wx = ox_tip + G['shoulder']*s + gap
        bottom = max(m['y_bot'], CAP*0.10)
        W = wx + ww + 2*e; H = bottom - top + 2*e
        tx, ty = e, e - top
        inner = f'<g transform="translate({tx:.2f} {ty:.2f})">{mark(ox_tip, oy_tip, s, v["ink"], v["sig"], soft=0.035)}<path transform="translate({wx:.2f} 0)" d="{d}" fill="{v["word"]}"/></g>'
        return inner, W, H, e
    if kind == 'stacked':
        bh = CAP*2.1; s = body_scale(bh); e = 2*G['eye_r']*s
        bw = G['shoulder']*s
        gap = CAP*0.55
        markw = bw + RIPX*s
        W0 = max(ww, markw)
        # centre the body (not the ripple) over the word, nudged left so the ripple balances
        ox_tip = W0/2 - bw/2 + RIPX*s*0.25
        oy_tip = RIPX*s
        wy = oy_tip + bh + gap + m['b_top']
        wx = (W0 - ww)/2
        H0 = wy + m['y_bot']
        x_min = min(0, ox_tip - RIPX*s)
        W = W0 - x_min + 2*e; H = H0 + 2*e
        inner = f'<g transform="translate({e - x_min:.2f} {e:.2f})">{mark(ox_tip, oy_tip, s, v["ink"], v["sig"], soft=0.035)}<path transform="translate({wx:.2f} {wy:.2f})" d="{d}" fill="{v["word"]}"/></g>'
        return inner, W, H, e

BASE_W = {'icon': 256, 'wordmark': 480, 'horizontal': 640, 'stacked': 360}
def build():
    out = []
    for kind in ('icon', 'wordmark', 'horizontal', 'stacked'):
        for vn, v in VARIANTS.items():
            inner, W, H, e = compose(kind, v)
            if v['bg']:   # backed files get a roomier frame: clear space x4
                p = 3*e; inner = f'<g transform="translate({p:.2f} {p:.2f})">{inner}</g>'; W += 2*p; H += 2*p
            bg = f'<rect width="{W:.2f}" height="{H:.2f}" fill="{v["bg"]}"/>' if v['bg'] else ''
            name = f'booya-{"logo-" + kind if kind not in ("icon",) else "mark"}-{vn}'
            name = name.replace('logo-wordmark', 'wordmark').replace('logo-horizontal', 'lockup-horizontal').replace('logo-stacked', 'lockup-stacked')
            sv = f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W:.2f} {H:.2f}" width="{W:.0f}" height="{H:.0f}">{bg}{inner}</svg>'
            w(f'{A}/logo/svg/{name}.svg', sv)
            pw = BASE_W[kind]; ph = round(pw*H/W)
            png(sv, f'{A}/logo/png/{name}.png', pw, ph)
            png(sv, f'{A}/logo/png/{name}@2x.png', pw*2, ph*2)
            out.append(name)
    return out
if __name__ == '__main__':
    print(len(build()))
