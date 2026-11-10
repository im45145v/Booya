"""Booya poses for docs and empty states. Same silhouette, only the eyes and the ripple change."""
import math
from geom import *
from common import *
GRAPHITE = '#6B6963'
def rrect_d(cx, cy, w, h, r):
    x, y = cx - w/2, cy - h/2; r = min(r, h/2, w/2)
    return (f'M{x+r:.2f} {y:.2f} H{x+w-r:.2f} A{r:.2f} {r:.2f} 0 0 1 {x+w:.2f} {y+r:.2f} V{y+h-r:.2f} A{r:.2f} {r:.2f} 0 0 1 {x+w-r:.2f} {y+h:.2f} '
            f'H{x+r:.2f} A{r:.2f} {r:.2f} 0 0 1 {x:.2f} {y+h-r:.2f} V{y+r:.2f} A{r:.2f} {r:.2f} 0 0 1 {x+r:.2f} {y:.2f} Z')

def pose(name, ink=INK, sig=SIGNAL, size=512):
    s = 240; ox, oy = 176, 120
    er = G['eye_r']*s
    eyes = [(ox + x*s, oy + y*s) for x, y in G['eyes']]
    body = ghost_d(ox, oy, s)
    extra = ''
    rip = True; rip_style = f'stroke="{sig}"'
    if name == 'idle':
        holes = ' '.join(circle_d(x, y, er) for x, y in eyes); rip = False
    elif name == 'blinking':
        holes = ' '.join(rrect_d(x, y, er*2.1, er*0.32, er*0.16) for x, y in eyes)
    elif name == 'zooming':
        holes = ' '.join(circle_d(x, y, er) for x, y in eyes)
        b0, b1, arm = 44, 468, 64
        extra = (f'<path d="M{b0} {b0+arm} V{b0} H{b0+arm} M{b1-arm} {b0} H{b1} V{b0+arm} M{b1} {b1-arm} V{b1} H{b1-arm} M{b0+arm} {b1} H{b0} V{b1-arm}" '
                 f'fill="none" stroke="{ink}" stroke-width="16" stroke-linecap="square"/>')
    elif name == 'saving':
        holes = ' '.join(circle_d(x + er*0.25, y + er*0.45, er*0.9) for x, y in eyes); rip = False
        y = oy + (G['stem'] + G['amp']*2.2)*s + 30; x0 = ox; x1 = ox + G['shoulder']*s
        extra = (f'<path d="M{x0} {y} H{x1}" stroke="{"#DCDAD3" if ink == INK else "#2C2B28"}" stroke-width="12" stroke-linecap="round"/>'
                 f'<path d="M{x0} {y} H{x0 + (x1-x0)*0.62:.1f}" stroke="{sig}" stroke-width="12" stroke-linecap="round"/>')
    elif name == 'error':
        holes = ' '.join(circle_d(x, y, er*1.15) + ' ' + circle_d(x, y, er*0.55) for x, y in eyes)
        rip_style = f'stroke="{GRAPHITE if ink == INK else "#8D8A82"}" stroke-dasharray="{0.07*s:.1f} {0.07*s:.1f}"'
    out = f'<path d="{body}" fill="none" stroke="{ink}" stroke-width="{0.035*s:.1f}" stroke-linejoin="round"/><path d="{body} {holes}" fill="{ink}" fill-rule="evenodd"/>'
    if rip:
        out += f'<path d="{ripple_d(ox, oy, s)}" fill="none" {rip_style} stroke-width="{G["rip_w"]*s:.1f}" stroke-linecap="round"/>'
    return svg(extra + out, size)

POSES = ['idle', 'blinking', 'zooming', 'saving', 'error']
def build():
    for p in POSES:
        for theme, ink, sig in (('light', INK, SIGNAL), ('dark', '#EDEBE5', '#FF6A33')):
            sv = pose(p, ink, sig)
            w(f'{A}/mascot/svg/booya-{p}-{theme}.svg', sv)
            png(sv, f'{A}/mascot/png/booya-{p}-{theme}.png', 512, 512)
            png(sv, f'{A}/mascot/png/booya-{p}-{theme}@2x.png', 1024, 1024)
if __name__ == '__main__':
    build()
    from PIL import Image
    im = Image.new('RGB', (5*260, 2*260), (244, 243, 239))
    for i, p in enumerate(POSES):
        a = Image.open(f'{A}/mascot/png/booya-{p}-light.png').resize((250, 250)); im.paste(a, (i*260+5, 5), a)
        bg = Image.new('RGB', (250, 250), (17, 17, 16)); b = Image.open(f'{A}/mascot/png/booya-{p}-dark.png').resize((250, 250)); bg.paste(b, (0, 0), b); im.paste(bg, (i*260+5, 265))
    im.save('/tmp/poses.png')
