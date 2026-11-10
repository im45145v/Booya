"""Hand-tuned small sizes. Drawn on a 1024 canvas where one output pixel is a fixed number of units,
so eyes land exactly on whole pixels."""
from geom import *
def snapped_icon(px, body_h, eye_cells, eye_size, rip_w, rip_r=None, dx=0.0, dy=0.0, inset_px=1, rx_px=None, stroke=True, rip=True, g=G, amp=None):
    u = 1024/px
    inset = inset_px*u
    rx = (rx_px if rx_px is not None else px*0.21)*u
    inner = 1024 - 2*inset
    ox, oy, s = place(inner, body_h, dx, dy)
    gg = dict(g)
    if rip_r: gg['rip_r'] = rip_r
    body = mark(ox+inset, oy+inset, s, eyes=False, rip=rip, soft=0.05, rip_w=rip_w, g=gg, amp=amp)
    eyes = ''.join(f'<rect x="{cx*u}" y="{cy*u}" width="{eye_size*u}" height="{eye_size*u}" fill="{PAPER}"/>' for cx, cy in eye_cells)
    t = tile(1024, inset, rx, stroke='#D9D6CE' if stroke else None, sw=u*0.9)
    return svg(t + body + eyes, 1024)

def eye_guess(px, body_h, dx=0.0, dy=0.0, inset_px=1):
    u = 1024/px; inset = inset_px*u; inner = 1024 - 2*inset
    ox, oy, s = place(inner, body_h, dx, dy)
    return [((ox+inset+x*s)/u, (oy+inset+y*s)/u) for x, y in G['eyes']], G['eye_r']*s*2/u

ICON32 = dict(px=32, amp=0.085, body_h=0.62, eye_cells=((11, 16), (14, 16)), eye_size=2, rip_w=0.12, rip_r=0.30)
ICON16 = dict(px=16, body_h=0.66, eye_cells=((5, 8), (7, 8)), eye_size=1, rip_w=0.17, rip_r=0.32, rx_px=3.2, inset_px=0.5)
def icon32(): return snapped_icon(**ICON32)
def icon16(): return snapped_icon(**ICON16)

# ---- tray glyph, pixel drawn for 16 (Windows) and vector for macOS 18pt ----
TRAY16 = [
 "................",
 "....X...........",
 "....XX..........",
 "....XXX.........",
 "....XXXX........",
 "....XXXXX.......",
 "....XXXXXX......",
 "....XXXXXXX.....",
 "....XX.X.XXX....",
 "....XXXXXXXXX...",
 "....XXXXXXXXXX..",
 "....XXXXXXX.....",
 "....XXXXXXX.....",
 "....XXXXXXXX....",
 "....XX.XX.XX....",
 "................",
]
RIP16 = [(1, 0), (2, 0), (0, 1), (0, 2)]
def tray16(rec=False, color='#000'):
    r = ''
    for y, row in enumerate(TRAY16):
        for x, c in enumerate(row):
            if c == 'X': r += f'<rect x="{x}" y="{y}" width="1" height="1"/>'
    if rec: r += ''.join(f'<rect x="{x}" y="{y}" width="1" height="1"/>' for x, y in RIP16)
    return f'<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16" shape-rendering="crispEdges"><g fill="{color}">{r}</g></svg>'

def tray18(rec=False, color='#000', size=18):
    """macOS menu bar template on an 18pt grid. Eyes are real holes so the OS tint shows through."""
    s = 9.8; ox, oy = 6.0, 3.0
    body = ghost_d(ox, oy, s)
    eyes = ' '.join(circle_d(ox + x*s, oy + y*s, 0.95) for x, y in G['eyes'])
    out = f'<path d="{body} {eyes}" fill="{color}" fill-rule="evenodd"/>'
    if rec:
        gg = dict(G); gg['rip_r'] = 0.30
        out += f'<path d="{ripple_d(ox, oy, s, gg)}" fill="none" stroke="{color}" stroke-width="1.5" stroke-linecap="round"/>'
    return f'<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" viewBox="0 0 18 18">{out}</svg>'

if __name__ == '__main__':
    for p in (32, 16):
        print(p, eye_guess(p, ICON32['body_h'] if p == 32 else ICON16['body_h'], inset_px=1 if p == 32 else 0.5))
