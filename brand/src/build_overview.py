from ui import *
from common import *
from render import shots
def build():
    sw = lambda n, h, dark=False: f'<div class="s"><i style="background:{h}"></i><b>{n}</b><span class="mono">{h}</span></div>'
    html = f'''<!doctype html><html><head><meta charset="utf-8"><style>{css('fonts')}
*{{box-sizing:border-box;margin:0;padding:0}}body{{width:1600px;height:1000px;background:#F4F3EF;-webkit-font-smoothing:antialiased;font-family:Geist;color:#111110;overflow:hidden}}
.over{{font:500 12px/1 'Geist Mono';letter-spacing:.08em;text-transform:uppercase;color:#6B6963}}
.c{{position:absolute;background:#fff;border:1px solid #DCDAD3;border-radius:20px;padding:26px}}
.s{{display:grid;grid-template-columns:46px 1fr;column-gap:12px;align-items:center}}.s i{{grid-row:1/3;width:46px;height:46px;border-radius:12px;border:1px solid rgba(0,0,0,.08)}}.s b{{font-size:14px;font-weight:600}}.s .mono{{font-size:12px;color:#6B6963}}
.pal{{display:grid;grid-template-columns:1fr 1fr;gap:14px 20px;margin-top:18px}}
</style></head><body class="by-l">
<div style="position:absolute;left:64px;top:52px;display:flex;align-items:center;gap:20px">{lockup(22)}<span class="over" style="margin-left:14px">Brand kit v1.0 · Direction E, Ghost x Ink mono</span></div>
<div class="c" style="left:64px;top:120px;width:500px;height:560px;display:flex;flex-direction:column;align-items:center">
  <div class="over" style="align-self:flex-start">App icon</div>
  <img src="assets/app-icon/png/booya-icon-1024.png" width="380" height="380" style="margin-top:14px">
  <div style="display:flex;align-items:flex-end;gap:24px;margin-top:20px"><img src="assets/app-icon/png/booya-icon-64.png" width="64"><img src="assets/app-icon/png/booya-icon-32.png" width="32"><img src="assets/app-icon/png/booya-icon-16.png" width="16"><img src="assets/app-icon/png/booya-icon-16.png" width="64" style="image-rendering:pixelated"></div></div>
<div class="c" style="left:588px;top:120px;width:560px;height:268px"><div class="over">Lockups</div>
  <div style="display:flex;align-items:center;justify-content:space-between;margin-top:26px;padding:0 6px">{lockup(46)}<img src="assets/logo/svg/booya-lockup-stacked-color.svg" height="160"></div></div>
<div class="c" style="left:588px;top:412px;width:272px;height:268px;background:#111110;border-color:#111110;display:grid;place-items:center">{lockup(30, '#EDEBE5', '#FF6A33')}</div>
<div class="c" style="left:876px;top:412px;width:272px;height:268px;display:grid;place-items:center"><img src="assets/mascot/svg/booya-zooming-light.svg" width="220"></div>
<div class="c" style="left:1172px;top:120px;width:364px;height:560px"><div class="over">Palette · light</div>
  <div class="pal">{sw('Paper', '#F4F3EF')}{sw('Surface', '#FFFFFF')}{sw('Ink', '#111110')}{sw('Graphite', '#6B6963')}{sw('Rule', '#DCDAD3')}{sw('Signal', '#FF5A1F')}</div>
  <div class="over" style="margin-top:28px">Palette · dark</div>
  <div class="pal">{sw('Night', '#111110')}{sw('Surface', '#1C1B19')}{sw('Paper', '#EDEBE5')}{sw('Stone', '#8D8A82')}{sw('Rule', '#2C2B28')}{sw('Signal', '#FF6A33')}</div></div>
<div class="c" style="left:64px;top:704px;width:736px;height:236px"><div class="over">UI · light</div>
  <div style="display:flex;flex-direction:column;gap:18px;margin-top:22px">{toolbar()}<div style="display:flex;gap:10px"><span class="btn primary">Start recording</span><span class="btn secondary">Export</span><span class="toast" style="min-width:0;padding:6px 12px 6px 8px;box-shadow:none"><span class="ti" style="color:var(--by-success)">{icon('check',16,2)}</span><div><b>Saved</b><span>48 MB</span></div></span></div></div></div>
<div class="c by-d" style="left:824px;top:704px;width:712px;height:236px;background:#111110;border-color:#2C2B28"><div class="over" style="color:#8D8A82">UI · dark</div>
  <div style="display:flex;flex-direction:column;gap:18px;margin-top:22px">{toolbar('12:03')}<div><span class="cap" style="font-size:18px;line-height:26px">Captions are written on this device.</span></div></div></div>
<div class="mono" style="position:absolute;right:64px;top:58px;font-size:13px;color:#6B6963">Record it. Booya.</div>
</body></html>'''
    w(f'{ROOT}/overview.html', html)
    shots([(f'{ROOT}/overview.html', f'{ROOT}/overview.png', 1600, 1000)])
    os.remove(f'{ROOT}/overview.html')
if __name__ == '__main__':
    build()
