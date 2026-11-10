from ui import *
from common import *
from render import shots
S = f'{A}/social'
FD = '../../../fonts'
def page(body, w_, h_, theme='by-l', extra=''):
    return f'''<!doctype html><html><head><meta charset="utf-8"><style>{css(FD)}
*{{box-sizing:border-box;margin:0;padding:0}}html,body{{width:{w_}px;height:{h_}px;overflow:hidden}}
body{{background:var(--by-bg);-webkit-font-smoothing:antialiased}}
.over{{font:500 13px/1 'Geist Mono';letter-spacing:.08em;text-transform:uppercase;color:var(--by-muted)}}
h1{{font-weight:600;letter-spacing:-.035em;line-height:.98}}
.sub{{color:var(--by-muted);font-size:22px;line-height:1.4;letter-spacing:-.005em}}
.screen{{position:absolute;background:var(--by-surface);border:1px solid var(--by-rule);border-radius:16px;overflow:hidden;box-shadow:var(--by-shadow)}}
.screen .bar{{height:34px;border-bottom:1px solid var(--by-rule);display:flex;align-items:center;gap:7px;padding:0 14px}}
.screen .bar i{{width:10px;height:10px;border-radius:50%;background:var(--by-rule)}}
.ln{{height:10px;border-radius:5px;background:var(--by-rule);margin:0 0 12px}}
{extra}</style></head><body class="{theme}">{body}</body></html>'''

def fake_screen(x, y, w_, h_, frame=True, cursor=True, caption=True):
    lines = ''.join(f'<div class="ln" style="width:{v}%;{"background:var(--by-text);opacity:.85;height:14px;margin-bottom:18px" if i == 0 else ""}"></div>' for i, v in enumerate((46, 88, 72, 80, 64, 0, 52, 84, 70)) if v)
    ff = focus_frame(int(w_*0.46), int(h_*0.30), int(w_*0.40), int(h_*0.34)) if frame else ''
    cur = (f'<div style="position:absolute;left:{int(w_*0.62)}px;top:{int(h_*0.43)}px">{mark_svg(30)}</div>') if cursor else ''
    cap = f'<div style="position:absolute;left:0;right:0;bottom:64px;text-align:center"><span class="cap" style="font-size:17px;line-height:24px;padding:6px 12px">So this is the part where it zooms in on its own.</span></div>' if caption else ''
    return f'''<div class="screen" style="left:{x}px;top:{y}px;width:{w_}px;height:{h_}px"><div class="bar"><i></i><i></i><i></i></div>
      <div style="padding:30px 34px">{lines}</div>{ff}{cur}{cap}</div>'''

def build():
    jobs = []
    # GitHub social preview 1280x640
    gh = page(f'''<div style="position:absolute;left:80px;top:84px">{lockup(46)}</div>
      <h1 style="position:absolute;left:80px;top:250px;font-size:84px">Record it.<br>Booya.</h1>
      <p class="sub" style="position:absolute;left:80px;top:470px;width:470px">Open source screen recorder. Offline captions. Auto zoom on clicks.</p>
      {fake_screen(640, 92, 580, 400)}
      <div style="position:absolute;left:712px;top:466px">{toolbar()}</div>
      <div class="over" style="position:absolute;left:80px;bottom:56px;text-transform:none;letter-spacing:.02em">Open source · github.com/im45145v/Booya</div>''', 1280, 640)
    w(f'{S}/src/github-social-preview.html', gh); jobs.append((f'{S}/src/github-social-preview.html', f'{S}/github-social-preview.png', 1280, 640))
    # og image 1200x630
    og = page(f'''<div style="position:absolute;left:84px;top:88px">{mark_svg(150)}</div>
      <h1 style="position:absolute;left:84px;top:290px;font-size:68px;width:640px">A screen recorder that zooms when you click.</h1>
      <p class="sub" style="position:absolute;left:86px;top:520px;width:520px">Free, open source, and captions stay on your machine.</p>
      <div style="position:absolute;right:84px;top:92px">{wordmark_svg(40)}</div>
      <div style="position:absolute;right:84px;bottom:76px">{toolbar('1:07')}</div>''', 1200, 630)
    w(f'{S}/src/og-image.html', og); jobs.append((f'{S}/src/og-image.html', f'{S}/og-image.png', 1200, 630))
    # README headers 1280x320 light/dark
    for th, cls in (('light', 'by-l'), ('dark', 'by-d')):
        rd = page(f'''<div style="position:absolute;left:72px;top:118px">{lockup(52)}</div>
          <p class="sub" style="position:absolute;left:520px;top:112px;width:330px;font-size:19px">Open source screen recorder with offline captions and auto zoom on clicks.</p>
          <div style="position:absolute;right:64px;top:132px">{toolbar()}</div>''', 1280, 320, cls)
        w(f'{S}/src/readme-header-{th}.html', rd); jobs.append((f'{S}/src/readme-header-{th}.html', f'{S}/readme-header-{th}.png', 1280, 320))
    # X / LinkedIn post template 1200x675 with editable slots
    post = page(f'''<div class="over" style="position:absolute;left:80px;top:80px" data-slot="overline">Booya v0.8</div>
      <h1 style="position:absolute;left:80px;top:128px;font-size:68px;width:620px" data-slot="headline">Captions now run fully offline.</h1>
      <p class="sub" style="position:absolute;left:80px;top:360px;width:560px" data-slot="body">The speech model runs on your machine. Nothing leaves it, and it works on a plane.</p>
      <div style="position:absolute;left:80px;bottom:72px">{lockup(30)}</div>
      <div class="mono" style="position:absolute;left:330px;bottom:76px;color:var(--by-muted);font-size:15px" data-slot="link">github.com/im45145v/Booya</div>
      <div style="position:absolute;right:70px;top:120px;width:420px;height:420px;border-radius:28px;background:var(--by-surface);border:1px solid var(--by-rule);display:grid;place-items:center">
        <img src="../../mascot/svg/booya-zooming-light.svg" width="340" height="340" data-slot="pose"></div>''', 1200, 675)
    w(f'{S}/src/post-template.html', post); jobs.append((f'{S}/src/post-template.html', f'{S}/post-template-x-linkedin.png', 1200, 675))
    shots(jobs)
if __name__ == '__main__':
    build()
