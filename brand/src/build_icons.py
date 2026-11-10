from geom import *
from small import icon32, icon16, tray16, tray18
from common import *
import io
from PIL import Image

def master(): return app_icon(1024)                      # Windows, Linux, web: near full bleed
def macos(): return app_icon(1024, inset=100, rx=185, body_h=0.58)   # Apple grid: 824 body
def by_size(px, mac=False):
    if px <= 16: return png_bytes(icon16(), px)
    if px <= 24: return png_bytes(icon16(), px) if px == 16 else png_bytes(icon32(), px)
    if px <= 32: return png_bytes(icon32(), px)
    return png_bytes(macos() if mac else master(), px)

def build():
    D = f'{A}/app-icon'
    w(f'{D}/booya-icon-master.svg', master()); w(f'{D}/booya-icon-macos.svg', macos())
    w(f'{D}/booya-icon-32px.svg', icon32()); w(f'{D}/booya-icon-16px.svg', icon16())
    for s in (16, 32, 64, 128, 256, 512, 1024):
        w(f'{D}/png/booya-icon-{s}.png', by_size(s))
        w(f'{D}/png-macos/booya-icon-macos-{s}.png', by_size(s, True))
    icns = {s: by_size(s, True) for s in (16, 32, 64, 128, 256, 512, 1024)}
    write_icns(f'{D}/booya.icns', icns)
    ico = [by_size(s) for s in (16, 24, 32, 48, 64, 128, 256)]
    write_ico(f'{D}/booya.ico', ico)
    # Linux hicolor
    for s in (16, 22, 24, 32, 48, 64, 128, 256, 512):
        w(f'{A}/linux/hicolor/{s}x{s}/apps/booya.png', by_size(s))
    w(f'{A}/linux/hicolor/scalable/apps/booya.svg', master())
    # Tauri set (drop straight into src-tauri/icons)
    T = f'{A}/tauri-icons'
    w(f'{T}/32x32.png', by_size(32)); w(f'{T}/128x128.png', by_size(128)); w(f'{T}/128x128@2x.png', by_size(256))
    w(f'{T}/icon.png', by_size(1024)); w(f'{T}/icon.icns', open(f'{D}/booya.icns', 'rb').read()); w(f'{T}/icon.ico', open(f'{D}/booya.ico', 'rb').read())
    for s in (30, 44, 71, 89, 107, 142, 150, 284, 310):
        w(f'{T}/Square{s}x{s}Logo.png', by_size(s))
    w(f'{T}/StoreLogo.png', by_size(50))
    # Tray / menu bar
    R = f'{A}/tray'
    for st, rec in (('idle', False), ('recording', True)):
        sv = tray18(rec)
        w(f'{R}/macos/tray-{st}Template.svg', sv)
        w(f'{R}/macos/tray-{st}Template.png', png_bytes(sv, 18))
        w(f'{R}/macos/tray-{st}Template@2x.png', png_bytes(sv, 36))
        pdf(sv, f'{R}/macos/tray-{st}Template.pdf')
        for theme, col in (('light', '#111110'), ('dark', '#F4F3EF')):
            # light taskbar gets the ink glyph, dark taskbar gets the paper glyph
            imgs = [png_bytes(tray16(rec, col), 16), png_bytes(tray16(rec, col), 32), png_bytes(tray18(rec, col), 24), png_bytes(tray18(rec, col), 48)]
            write_ico(f'{R}/windows/tray-{st}-{theme}.ico', imgs)
            w(f'{R}/windows/tray-{st}-{theme}-16.png', imgs[0])
        w(f'{R}/windows/tray-{st}-16.svg', tray16(rec, '#111110'))
    # Favicon
    F = f'{A}/favicon'
    fav = favicon_svg(); w(f'{F}/favicon.svg', fav)
    write_ico(f'{F}/favicon.ico', [png_bytes(icon16(), 16), png_bytes(icon32(), 32), by_size(48)])
    w(f'{F}/apple-touch-icon.png', png_bytes(app_icon(1024, inset=0, rx=0), 180))
    w(f'{F}/icon-192.png', by_size(192)); w(f'{F}/icon-512.png', by_size(512))
    w(f'{F}/site.webmanifest', '{\n  "name": "Booya",\n  "short_name": "Booya",\n  "icons": [\n    {"src": "/icon-192.png", "sizes": "192x192", "type": "image/png"},\n    {"src": "/icon-512.png", "sizes": "512x512", "type": "image/png"}\n  ],\n  "theme_color": "#111110",\n  "background_color": "#F4F3EF",\n  "display": "standalone"\n}\n')

def favicon_svg():
    """Tile favicon that flips to the ink tile in dark mode."""
    ox, oy, s = place(1024 - 64, 0.62)
    ox += 32; oy += 32
    body = ghost_d(ox, oy, s) + ' ' + eyes_d(ox, oy, s, scale=1.25)
    gg = dict(G); gg['rip_r'] = 0.30
    return f'''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024">
<style>.t{{fill:#F4F3EF}}.b{{fill:#111110;stroke:#111110}}.r{{stroke:#FF5A1F}}
@media (prefers-color-scheme: dark){{.t{{fill:#111110}}.b{{fill:#EDEBE5;stroke:#EDEBE5}}.r{{stroke:#FF6A33}}}}</style>
<rect class="t" x="32" y="32" width="960" height="960" rx="216"/>
<path class="b" d="{body}" fill-rule="evenodd" stroke-width="0"/>
<path class="r" d="{ripple_d(ox, oy, s, gg)}" fill="none" stroke-width="{0.11*s:.1f}" stroke-linecap="round"/>
</svg>'''
if __name__ == '__main__':
    build()
