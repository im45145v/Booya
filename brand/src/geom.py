"""Booya mark geometry. Unit space: tip at (0,0), width 1.0, height about 1.4."""
import math
INK, PAPER, SIGNAL = '#111110', '#F4F3EF', '#FF5A1F'

# key points (unit space)
G = dict(
    stem=1.24,          # bottom of the straight left edge
    hem_end=0.78,       # where the scalloped hem meets the tail
    notch=(0.60, 1.0),  # inner corner of the tail notch
    shoulder=1.0,       # 45 degree edge ends at (shoulder, shoulder)
    humps=3, amp=0.066,
    eyes=((0.215, 0.745), (0.455, 0.745)), eye_r=0.078,
    rip_r=0.27, rip_w=0.085, rip_a=(182, 268),
)

def ghost_d(ox=0, oy=0, s=1, g=G, humps=None, amp=None):
    g = dict(g); humps = humps or g['humps']; amp = g['amp'] if amp is None else amp
    P = lambda x, y: f'{ox + x*s:.2f} {oy + y*s:.2f}'
    st, he = g['stem'], g['hem_end']
    d = f'M{P(0,0)} L{P(0,st)}'
    w = he / humps
    for k in range(humps):
        x0, x1 = k*w, (k+1)*w
        d += f' C{P(x0 + w*0.08, st + amp*1.55)} {P(x1 - w*0.08, st + amp*1.55)} {P(x1, st)}'
    nx, ny = g['notch']; sh = g['shoulder']
    d += f' L{P(nx, ny)} L{P(sh, sh)} Z'
    return d

def circle_d(cx, cy, r):
    return f'M{cx+r:.2f} {cy:.2f} A{r:.2f} {r:.2f} 0 1 0 {cx-r:.2f} {cy:.2f} A{r:.2f} {r:.2f} 0 1 0 {cx+r:.2f} {cy:.2f} Z'

def eyes_d(ox=0, oy=0, s=1, g=G, scale=1.0):
    return ' '.join(circle_d(ox + x*s, oy + y*s, g['eye_r']*s*scale) for x, y in g['eyes'])

def ripple_d(ox=0, oy=0, s=1, g=G, r=None):
    r = (r or g['rip_r']) * s
    a0, a1 = (math.radians(a) for a in g['rip_a'])
    return f'M{ox + r*math.cos(a0):.2f} {oy + r*math.sin(a0):.2f} A{r:.2f} {r:.2f} 0 0 1 {ox + r*math.cos(a1):.2f} {oy + r*math.sin(a1):.2f}'

def mark(ox, oy, s, ink=INK, sig=SIGNAL, eyes=True, rip=True, soft=0.035, eye_scale=1.0, rip_w=None, g=G, humps=None, amp=None):
    """Body with true eye holes (evenodd) plus the ripple arc. Soft corners from a same-colour round-join stroke,
    with eyes re-cut on top so the stroke never closes them."""
    body = ghost_d(ox, oy, s, g, humps, amp)
    out = f'<path d="{body}" fill="{ink}" stroke="{ink}" stroke-width="{soft*s:.2f}" stroke-linejoin="round"/>' if soft else ''
    full = body + (' ' + eyes_d(ox, oy, s, g, eye_scale) if eyes else '')
    if soft and eyes:
        # draw the softened silhouette as a mask-free composite: stroke outline + evenodd fill with holes
        out = (f'<path d="{body}" fill="none" stroke="{ink}" stroke-width="{soft*s:.2f}" stroke-linejoin="round"/>'
               f'<path d="{full}" fill="{ink}" fill-rule="evenodd"/>')
    elif not soft:
        out = f'<path d="{full}" fill="{ink}" fill-rule="evenodd"/>'
    if rip:
        out += f'<path d="{ripple_d(ox, oy, s, g)}" fill="none" stroke="{sig}" stroke-width="{(rip_w or g["rip_w"])*s:.2f}" stroke-linecap="round"/>'
    return out

def svg(body, w=1024, h=None, vb=None):
    h = h or w; vb = vb or f'0 0 {w} {h}'
    return f'<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="{vb}">{body}</svg>'

# mark bounding box in unit space (with ripple): x from -rip_r-rip_w/2 to shoulder, y from -rip_r-... to stem+amp
def bbox(g=G, rip=True):
    x0 = -(g['rip_r'] + g['rip_w']/2) if rip else 0
    y0 = x0
    return x0, y0, g['shoulder'], g['stem'] + g['amp']*1.2

def place(size, body_h=0.60, dx=0.02, dy=0.025, g=G):
    """Scale and origin so the body sits optically centred in a square of `size`."""
    s = size*body_h/(g['stem'] + g['amp']*1.2)
    bw, bh = g['shoulder']*s, (g['stem'] + g['amp']*1.2)*s
    ox = size/2 - bw/2 + dx*size
    oy = size/2 - bh/2 + dy*size
    return ox, oy, s

def tile(size=1024, inset=32, rx=216, fill=PAPER, stroke='#E2E0D9', sw=6):
    st = f' stroke="{stroke}" stroke-width="{sw}"' if stroke else ''
    return f'<rect x="{inset}" y="{inset}" width="{size-2*inset}" height="{size-2*inset}" rx="{rx}" fill="{fill}"{st}/>'

def app_icon(size=1024, inset=32, rx=216, body_h=0.58, **kw):
    inner = size - 2*inset
    ox, oy, s = place(inner, body_h)
    return svg(tile(size, inset, rx) + mark(ox+inset, oy+inset, s, soft=0.05, **kw), size)
