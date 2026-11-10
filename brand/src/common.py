import os, io, struct, cairosvg
from PIL import Image
ROOT = '/workspace/booya-brand'
A = f'{ROOT}/assets'
def w(path, data):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    open(path, 'w' if isinstance(data, str) else 'wb').write(data)
def png(svgtext, path, w_=None, h_=None):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    cairosvg.svg2png(bytestring=svgtext.encode(), write_to=path, output_width=w_, output_height=h_)
def png_bytes(svgtext, w_, h_=None):
    return cairosvg.svg2png(bytestring=svgtext.encode(), output_width=w_, output_height=h_ or w_)
def pdf(svgtext, path):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    cairosvg.svg2pdf(bytestring=svgtext.encode(), write_to=path)

def write_ico(path, pngs):
    """pngs: list of PNG bytes (each square). Writes a PNG-compressed ICO."""
    n = len(pngs); hdr = struct.pack('<HHH', 0, 1, n); off = 6 + 16*n; ents = b''; body = b''
    for b in pngs:
        im = Image.open(io.BytesIO(b)); s = im.size[0]
        ents += struct.pack('<BBBBHHII', s if s < 256 else 0, s if s < 256 else 0, 0, 0, 1, 32, len(b), off + len(body))
        body += b
    w(path, hdr + ents + body)

ICNS_TYPES = {16: b'icp4', 32: b'icp5', 64: b'icp6', 128: b'ic07', 256: b'ic08', 512: b'ic09', 1024: b'ic10'}
RETINA = {32: b'ic11', 64: b'ic12', 256: b'ic13', 512: b'ic14'}   # 16@2x, 32@2x, 128@2x, 256@2x
def write_icns(path, by_size):
    """by_size: {px: PNG bytes}. Writes both standard and @2x slots."""
    chunks = b''
    for s, t in ICNS_TYPES.items():
        if s in by_size: chunks += t + struct.pack('>I', 8 + len(by_size[s])) + by_size[s]
    for s, t in RETINA.items():
        if s in by_size: chunks += t + struct.pack('>I', 8 + len(by_size[s])) + by_size[s]
    w(path, b'icns' + struct.pack('>I', 8 + len(chunks)) + chunks)
