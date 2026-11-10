import sys; sys.path.insert(0,'.')
from geom import *
import geom, copy, subprocess
variants = {
 'a': {},
 'b': dict(notch=(0.55,0.94), hem_end=0.84, stem=1.30),
 'c': dict(notch=(0.60,1.0), hem_end=0.78, stem=1.26, shoulder=1.0),
}
cells=''
for i,(k,v) in enumerate(variants.items()):
    g=dict(G); g.update(v)
    s=470; ox=512-0.5*s+40; oy=512-0.68*s+30
    tile=f'<rect x="32" y="32" width="960" height="960" rx="216" fill="{PAPER}" stroke="#E2E0D9" stroke-width="6"/>'
    open(f'/tmp/p{k}.svg','w').write(svg(tile+mark(ox,oy,s,g=g)))
