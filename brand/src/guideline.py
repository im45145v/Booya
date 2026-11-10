import math, json, os
from ui import *
from common import *
from build_tokens import report, LIGHT, DARK, SEM_L, SEM_D, SIGNAL_TEXT, TYPE
GD = f'{ROOT}/guideline'
AS = '../assets'
VERSION = 'v1.0 · November 2026'

def bez(t, p1x=0.2, p1y=0.8, p2x=0.2, p2y=1.0):
    # solve x(u)=t for cubic-bezier(0,0,p1,p2,1,1)
    lo, hi = 0.0, 1.0
    for _ in range(40):
        u = (lo+hi)/2; x = 3*(1-u)**2*u*p1x + 3*(1-u)*u*u*p2x + u**3
        lo, hi = (u, hi) if x < t else (lo, u)
    u = (lo+hi)/2
    return 3*(1-u)**2*u*p1y + 3*(1-u)*u*u*p2y + u**3

PAGES = []
def page(sec, title, body, theme='by-l', num=True, cls=''):
    PAGES.append((sec, title, body, theme, num, cls))

def head(sec, title):
    return f'<header class="ph"><span class="mono">{sec}</span><span class="pt">{title}</span><span class="brand">{lockup(13)}</span></header>'

# ---------------- pages ----------------
# 01 cover
page('', '', f'''
<div style="position:absolute;left:96px;top:96px">{lockup(30)}</div>
<div class="mono over" style="position:absolute;right:96px;top:104px">Brand guideline · {VERSION}</div>
<div style="position:absolute;left:96px;bottom:150px">
  <div class="over mono" style="margin-bottom:26px">Direction E · Ghost x Ink mono</div>
  <h1 style="font-size:150px;line-height:.9;letter-spacing:-.045em;font-weight:600">Record it.<br>Booya.</h1>
</div>
<div style="position:absolute;right:120px;top:190px">{mark_svg(520)}</div>
<div class="mono" style="position:absolute;left:96px;bottom:72px;color:var(--by-muted);font-size:13px">Open source screen recorder · offline captions · auto zoom on clicks</div>
''', num=False)

TOC = [('01', 'Brand story'), ('02', 'Meet Booya'), ('03', 'The refined mark'), ('04', 'Logo anatomy'), ('05', 'Lockups, clear space, sizes'),
       ('06', 'Logo use'), ('07', 'Colour'), ('08', 'Contrast and semantic colour'), ('09', 'Typography'), ('10', 'Iconography'),
       ('11', 'Components, light'), ('12', 'Components, dark'), ('13', 'Settings window'), ('14', 'Captions and the focus frame'),
       ('15', 'Motion'), ('16', 'Voice and tone'), ('17', 'In the wild: desktop'), ('18', 'In the wild: web'), ('19', 'Asset index')]
toc_html = ''.join(f'<div class="toc-i"><span class="mono">{n}</span><span>{t}</span></div>' for n, t in TOC)
page('00', 'Contents', f'''{head('00', 'Contents')}
<h2 class="h" style="margin-top:40px">Contents</h2>
<div class="toc">{toc_html}</div>
<img src="{AS}/mascot/svg/booya-idle-light.svg" width="330" height="330" style="position:absolute;right:110px;bottom:120px">''')

page('01', 'Brand story', f'''{head('01', 'Brand story')}
<div class="cols2" style="margin-top:56px">
 <div>
  <div class="over mono">What Booya is</div>
  <h2 class="h" style="margin-top:18px">A screen recorder that<br>knows where to look.</h2>
  <p class="lead" style="margin-top:26px">Booya records your screen, writes captions on your own machine and zooms in when your clicks bunch up. It is free, open source and built with Tauri, so it stays small and fast.</p>
  <p class="lead" style="margin-top:18px">The brand should feel like the app: calm, quick, a little bit funny, and out of the way until you need it.</p>
 </div>
 <div class="principles">
  <div class="pr"><span class="mono">1</span><b>Plain first</b><p>Ink on paper. One accent. If something needs a second colour, it probably needs fewer words instead.</p></div>
  <div class="pr"><span class="mono">2</span><b>On purpose</b><p>Nothing moves, blinks or glows unless it tells you something. The ripple means a click. The orange means recording.</p></div>
  <div class="pr"><span class="mono">3</span><b>Precise, not cold</b><p>Tight type, exact pixels, tidy spacing. The ghost is the one wink, and we only wink once.</p></div>
  <div class="pr"><span class="mono">4</span><b>Yours</b><p>Offline by default. We say what stays on your machine and we never make it sound like magic.</p></div>
 </div>
</div>''')

poses = ''.join(f'<figure class="pose"><img src="{AS}/mascot/svg/booya-{p}-light.svg"><figcaption><b>{p.capitalize()}</b><span>{d}</span></figcaption></figure>' for p, d in (
    ('idle', 'Default. Empty states, docs.'), ('blinking', 'Record start and stop only.'), ('zooming', 'Auto zoom, tutorials.'),
    ('saving', 'Exporting, writing captions.'), ('error', 'Something failed. Stay calm.')))
page('02', 'Meet Booya', f'''{head('02', 'Meet Booya')}
<div class="cols2" style="margin-top:44px;grid-template-columns:560px 1fr">
 <div>
  <div class="over mono">The backstory</div>
  <h2 class="h" style="margin-top:18px">The forgotten arrow.</h2>
  <p class="lead" style="margin-top:24px">For years Booya was the cursor nobody looked at. It pointed, it clicked, it got cropped out of every demo. So it died a little, came back as a ghost, and now it haunts your cursor on purpose.</p>
  <ul class="facts">
   <li><b>It zooms only when three clicks cluster.</b> One click is a click. Three close together means you are showing something.</li>
   <li><b>It blinks when recording starts and stops.</b> Never in between. That is how you know it heard you.</li>
   <li><b>It writes captions offline.</b> It listens, it types, and nothing leaves your machine.</li>
  </ul>
 </div>
 <div><div class="poses">{poses}</div>
  <p class="note" style="margin-top:22px">Poses only change the eyes, the ripple and one small prop. No arms, no mouths, no sparkles, no blush. Files: assets/mascot, light and dark, SVG and PNG.</p></div>
</div>''')

page('03', 'The refined mark', f'''{head('03', 'The refined mark')}
<div class="ba">
 <div class="bai"><div class="over mono">Before · mockup E</div><img src="img/before-E-icon.png" width="360" height="360">
   <div class="smalls"><img src="img/before-E-icon-32.png" width="32"><img src="img/before-E-icon-16.png" width="16"><img class="px" src="img/before-E-icon-16.png" width="64"></div></div>
 <div class="arrow">{icon('chevron', 40, 1.2)}</div>
 <div class="bai"><div class="over mono">After · v1.0</div><img src="{AS}/app-icon/png/booya-icon-1024.png" width="360" height="360">
   <div class="smalls"><img src="{AS}/app-icon/png/booya-icon-32.png" width="32"><img src="{AS}/app-icon/png/booya-icon-16.png" width="16"><img class="px" src="{AS}/app-icon/png/booya-icon-16.png" width="64"></div></div>
 <ul class="facts" style="width:430px;margin-left:20px">
  <li><b>Pointer first.</b> The swollen dome read like a party hat. Now it is a real arrow: straight stem, 45 degree edge, sharp tip.</li>
  <li><b>A proper tail notch.</b> The inner corner on the lower right is the detail everyone recognises from a real cursor.</li>
  <li><b>Chunkier.</b> 1 : 1.32 body, wider at the shoulder, so it holds up in the Dock and the tray.</li>
  <li><b>Ghost second.</b> Three soft scallops on the hem and two cut-out eyes up in the head.</li>
  <li><b>Hand-tuned small sizes.</b> At 32 and 16 px the eyes snap to whole pixels.</li>
 </ul>
</div>''')

# 04 anatomy: construction svg
def construction():
    s = 380; ox, oy = 150, 140; W, H = 640, 700
    P = lambda x, y: (ox + x*s, oy + y*s)
    grid = ''.join(f'<path d="M{ox + i*0.1*s:.1f} {oy - 0.4*s:.1f} V{oy + 1.45*s:.1f}" class="g{" gm" if i % 5 == 0 else ""}"/>' for i in range(-3, 12))
    grid += ''.join(f'<path d="M{ox - 0.36*s:.1f} {oy + j*0.1*s:.1f} H{ox + 1.12*s:.1f}" class="g{" gm" if j % 5 == 0 else ""}"/>' for j in range(-3, 15))
    body = mark(ox, oy, s, '#111110', '#FF5A1F', soft=0.035)
    nx, ny = G['notch']; sh = G['shoulder']; st = G['stem']; he = G['hem_end']
    pts = [((0, 0), 'tip', -16, -14, 'end'), ((0, st), 'stem', -16, 6, 'end'), ((sh, sh), 'shoulder', 14, 5, 'start'), ((nx, ny), 'notch', 4, -14, 'start'), ((he, st), 'hem end', 14, 26, 'start')]
    dots = ''.join(f'<circle cx="{P(*p)[0]:.1f}" cy="{P(*p)[1]:.1f}" r="6" class="kp"/><text x="{P(*p)[0]+dx:.1f}" y="{P(*p)[1]+dy:.1f}" text-anchor="{a}" class="lb">{t}</text>' for p, t, dx, dy, a in pts)
    guide45 = f'<path d="M{P(-0.15,-0.15)[0]:.1f} {P(-0.15,-0.15)[1]:.1f} L{P(1.08,1.08)[0]:.1f} {P(1.08,1.08)[1]:.1f}" class="gd"/><text x="{P(0.62,0.5)[0]:.1f}" y="{P(0.62,0.5)[1]:.1f}" class="lb sg" transform="rotate(45 {P(0.62,0.5)[0]:.1f} {P(0.62,0.5)[1]:.1f})">45°</text>'
    ripc = f'<circle cx="{ox}" cy="{oy}" r="{G["rip_r"]*s:.1f}" class="gd"/>'
    e = G['eyes'][0]; er = G['eye_r']*s
    eyeg = f'<path d="M{P(*e)[0]-er:.1f} {P(*e)[1]+er+18:.1f} h{2*er:.1f}" class="dim"/><text x="{P(*e)[0]:.1f}" y="{P(*e)[1]+er+40:.1f}" text-anchor="middle" class="lb">e</text>'
    box = f'<rect x="{ox}" y="{oy}" width="{sh*s:.1f}" height="{(st+0.077)*s:.1f}" class="bx"/>'
    dimw = f'<text x="{ox + sh*s/2:.1f}" y="{oy + 1.40*s + 22:.1f}" text-anchor="middle" class="lb">1.00</text><text x="{ox + sh*s + 34:.1f}" y="{oy + 0.66*s:.1f}" class="lb">1.32</text>'
    return f'''<svg viewBox="0 0 {W} {H}" width="{W}" height="{H}"><style>.g{{stroke:#DCDAD3;stroke-width:1}}.gm{{stroke:#C9C6BE}}.gd{{stroke:#FF5A1F;stroke-width:1.5;stroke-dasharray:6 6;fill:none}}
    .kp{{fill:#F4F3EF;stroke:#FF5A1F;stroke-width:2.5}}.lb{{font:500 15px 'Geist Mono';fill:#6B6963}}.sg{{fill:#BF4317}}.bx{{fill:none;stroke:#6B6963;stroke-width:1;stroke-dasharray:3 4}}.dim{{stroke:#111110;stroke-width:2}}</style>
    {grid}{box}<g opacity=".94">{body}</g>{guide45}{ripc}{dots}{eyeg}{dimw}</svg>'''
page('04', 'Logo anatomy', f'''{head('04', 'Logo anatomy')}
<div style="display:grid;grid-template-columns:640px 1fr;gap:90px;margin-top:14px">
 <div>{construction()}</div>
 <div style="padding-top:40px">
  <div class="over mono">Construction</div>
  <h2 class="h" style="margin-top:16px;font-size:44px">Built on the arrow.</h2>
  <ul class="facts">
   <li><b>Grid.</b> The body is 1.00 wide and 1.32 tall, drawn on a 0.1 grid from the tip.</li>
   <li><b>Edges.</b> A straight stem on the left and a 45 degree edge from the tip to the shoulder.</li>
   <li><b>Tail notch.</b> At the shoulder line, the edge steps in to 0.60, then the tail runs down to the hem.</li>
   <li><b>Hem.</b> Three equal scallops, 0.26 wide, 0.077 deep.</li>
   <li><b>Eyes.</b> Two holes, diameter <b>e</b> = 0.156, centred 0.24 apart in the head. <b>e</b> is also the clear space unit.</li>
   <li><b>Ripple.</b> One quarter ring around the tip, radius 0.27, stroke 0.085, round caps. Always Signal, or mono.</li>
   <li><b>Corners.</b> A 0.035 round join softens the tip and shoulder without blunting them.</li>
  </ul>
 </div>
</div>''')

def cs_box(inner, pad):
    return f'<div class="cs" style="padding:{pad}px">{inner}<i class="csl" style="inset:{pad}px"></i></div>'
page('05', 'Lockups, clear space, sizes', f'''{head('05', 'Lockups, clear space and minimum sizes')}
<div class="grid3" style="margin-top:44px">
 <div class="card"><div class="over mono">Horizontal · primary</div><div class="cbox">{cs_box(lockup(54), 22)}</div><p class="note">Mark body is 1.58 x the cap height. Gap is 0.42 x cap. The hem sits a hair under the baseline.</p></div>
 <div class="card"><div class="over mono">Stacked</div><div class="cbox"><img src="{AS}/logo/svg/booya-lockup-stacked-color.svg" height="230"></div><p class="note">For square spaces, splash screens and stickers. Body is 2.1 x cap.</p></div>
 <div class="card"><div class="over mono">Mark and wordmark</div><div class="cbox" style="gap:46px">{mark_svg(110)}{wordmark_svg(52)}</div><p class="note">The wordmark is Geist SemiBold, tracked -3.5%, outlined. Never retype it.</p></div>
</div>
<div class="grid3" style="margin-top:22px">
 <div class="card" style="display:flex;gap:26px;align-items:center"><div>{cs_box(mark_svg(56), 9)}</div><div><div class="over mono">Clear space</div><p class="note" style="margin-top:10px">Keep at least one eye width <b>e</b> clear on every side, measured from the ripple, the stem and the hem. More is better.</p></div></div>
 <div class="card"><div class="over mono">Minimum size</div><div class="mins"><div>{mark_svg(16)}<span class="mono">mark 16 px</span></div><div>{lockup(9)}<span class="mono">lockup 88 px wide</span></div><div>{wordmark_svg(11)}<span class="mono">wordmark 56 px</span></div></div></div>
 <div class="card"><div class="over mono">Small sizes</div><div class="mins"><div><img src="{AS}/app-icon/png/booya-icon-32.png" width="32"><span class="mono">32 tuned</span></div><div><img src="{AS}/app-icon/png/booya-icon-16.png" width="16"><span class="mono">16 tuned</span></div><div><img class="px" src="{AS}/app-icon/png/booya-icon-16.png" width="64"><span class="mono">16 at 4x</span></div></div></div>
</div>''')

def dont(inner, label, bg='var(--by-surface)'):
    return f'<div class="dn"><div class="dnb" style="background:{bg}">{inner}</div><div class="dnl">{icon("close", 16, 2)}<span>{label}</span></div></div>'
def do(inner, label, bg):
    return f'<div class="dn"><div class="dnb" style="background:{bg}">{inner}</div><div class="dnl ok">{icon("check", 16, 2)}<span>{label}</span></div></div>'
M = lambda h=120, **k: mark_svg(h, **k)
page('06', 'Logo use', f'''{head('06', 'Logo use')}
<div class="over mono" style="margin-top:40px">Colourways</div>
<div class="ways">
 {do(lockup(30, '#111110', '#FF5A1F'), 'Full colour on paper', '#F4F3EF')}
 {do(lockup(30, '#111110', '#FF5A1F'), 'Full colour on white', '#FFFFFF')}
 {do(lockup(30, '#EDEBE5', '#FF6A33'), 'Paper on ink', '#111110')}
 {do(lockup(30, '#000000', '#000000'), 'Mono black', '#FFFFFF')}
 {do(lockup(30, '#FFFFFF', '#FFFFFF'), 'Mono white', '#6B6963')}
</div>
<div class="over mono" style="margin-top:34px">Please don't</div>
<div class="donts">
 {dont(f'<div style="transform:rotate(-24deg)">{M(110)}</div>', 'Rotate or tilt it')}
 {dont(M(110, ink='#7C5CFA', sig='#F7B6C8'), 'Bring back lavender or purple', '#E9E4FB')}
 {dont(M(110) + '<i class="blush"></i><i class="blush b2"></i>', 'Add blush, mouths or limbs')}
 {dont(f'<div style="transform:scaleX(1.45)">{M(110)}</div>', 'Stretch or squash it')}
 {dont(f'<div style="filter:drop-shadow(0 0 14px #FF5A1F)">{M(110, ink="url(#gr)")}</div><svg width="0" height="0" style="position:absolute"><defs><linearGradient id="gr" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#FF5A1F"/><stop offset="1" stop-color="#111110"/></linearGradient></defs></svg>', 'Add gradients or glow')}
 {dont(f'<svg viewBox="-34 -34 140 170" width="96" height="117"><path d="{ghost_d(0,0,100)}" fill="none" stroke="#111110" stroke-width="4"/></svg>', 'Use it as an outline')}
 {dont(f'<div style="display:flex;gap:28px;align-items:center">{M(100, rip=False)}<svg width="40" height="40" viewBox="0 0 40 40"><path d="M36 30 A26 26 0 0 0 10 4" fill="none" stroke="#FF5A1F" stroke-width="8" stroke-linecap="round"/></svg></div>', 'Move the ripple away from the tip')}
 {dont(M(110), 'Place it on busy imagery', 'repeating-linear-gradient(45deg,#FF5A1F 0 10px,#F4F3EF 10px 20px,#111110 20px 30px)')}
</div>''')

def sw(name, hexv, note, dark=False):
    return f'<div class="sw"><div class="swc" style="background:{hexv}"></div><b>{name}</b><span class="mono">{hexv}</span><em>{note}</em></div>'
page('07', 'Colour', f'''{head('07', 'Colour')}
<div class="cols2" style="grid-template-columns:1fr 1fr;margin-top:40px;gap:48px">
 <div><div class="over mono">Light UI</div><div class="sws">
  {sw('Paper', '#F4F3EF', 'App background')}{sw('Surface', '#FFFFFF', 'Panels, toolbar')}{sw('Ink', '#111110', 'Text, mark')}
  {sw('Graphite', '#6B6963', 'Secondary text')}{sw('Rule', '#DCDAD3', 'Dividers')}{sw('Signal', '#FF5A1F', 'Recording, ripple')}</div></div>
 <div><div class="over mono">Dark UI</div><div class="sws">
  {sw('Night', '#111110', 'App background')}{sw('Surface', '#1C1B19', 'Panels, toolbar')}{sw('Paper', '#EDEBE5', 'Text, mark')}
  {sw('Stone', '#8D8A82', 'Secondary text')}{sw('Rule', '#2C2B28', 'Dividers')}{sw('Signal', '#FF6A33', 'Recording, ripple')}</div></div>
</div>
<div class="over mono" style="margin-top:40px">Usage ratio on a typical screen</div>
<div class="ratio"><i style="flex:60;background:#F4F3EF">Paper 60%</i><i style="flex:20;background:#FFFFFF">Surface 20%</i><i style="flex:12;background:#111110;color:#F4F3EF">Ink 12%</i><i style="flex:6;background:#DCDAD3">Rule, Graphite 6%</i><i style="flex:2;background:#FF5A1F"></i></div>
<div class="cols3" style="margin-top:24px">
 <p class="note"><b>Signal stays under 10% of any screen</b>, and usually under 3%. It is the record dot, the Stop button, the ripple and the focus frame while recording.</p>
 <p class="note"><b>Recording state is always Signal.</b> If something is orange, the user should be able to assume it is recording, or about the click.</p>
 <p class="note"><b>No other accents.</b> Status colours (next page) appear only inside messages, never as decoration.</p>
</div>''')

rows = report()
def rrow(r):
    ok = r['result'] in ('AA', 'AAA', 'Pass 3:1')
    return f'<tr><td><span class="chip2" style="background:{r["bg"]};color:{r["fg"]}">Aa</span></td><td>{r["pair"]}</td><td class="mono">{r["fg"]} / {r["bg"]}</td><td class="mono">{r["ratio"]:.2f}</td><td class="{"ok" if ok else "bad"}">{r["result"]}</td></tr>'
sel = [r for r in rows if r['mode'] == 'light']; seld = [r for r in rows if r['mode'] == 'dark']
sem = ''.join(f'<div class="sem"><i style="background:{SEM_L[k]}"></i><i style="background:{SEM_D[k]}"></i><b>{n}</b><span class="mono">{SEM_L[k]} · {SEM_D[k]}</span></div>' for k, n in (
    ('success', 'Success'), ('warning', 'Warning'), ('danger', 'Danger'), ('recording', 'Recording'), ('focus', 'Focus ring'), ('signal_text', 'Signal text')))
page('08', 'Contrast and semantic colour', f'''{head('08', 'Contrast and semantic colour')}
<div style="display:grid;grid-template-columns:330px 1fr 1fr;gap:34px;margin-top:36px">
 <div><div class="over mono">Semantic · light · dark</div><div class="sems">{sem}</div>
  <p class="note" style="margin-top:18px"><b>Signal on Paper fails for text</b> at {[r for r in rows if r['pair']=='Signal on Paper'][0]['ratio']}:1. Use <b>Signal text {SIGNAL_TEXT}</b> for orange words and links in light mode. Keep #FF5A1F for fills.</p>
  <p class="note"><b>Buttons on Signal use Ink labels</b> (6.06:1). White on Signal is 3.12:1 and is not allowed.</p></div>
 <table class="ct"><tr><th></th><th>Light</th><th>Colours</th><th>Ratio</th><th>WCAG</th></tr>{''.join(rrow(r) for r in sel)}</table>
 <table class="ct"><tr><th></th><th>Dark</th><th>Colours</th><th>Ratio</th><th>WCAG</th></tr>{''.join(rrow(r) for r in seld)}</table>
</div>''')

scale = ''.join(f'<div class="ts"><span class="mono">{k} · {v["size"]}/{v["line"]} · {v["weight"]}</span><div style="font-size:{v["size"]}px;line-height:{v["line"]}px;font-weight:{v["weight"]};letter-spacing:{v["tracking"]};{"font-family:Geist Mono;text-transform:uppercase" if v.get("family")=="mono" and k=="overline" else ""}{"font-family:Geist Mono;font-variant-numeric:tabular-nums" if k=="timer" else ""}">{t}</div></div>' for k, v, t in (
    (k, TYPE[k], t) for k, t in (('display', 'Record it. Booya.'), ('h1', 'Your recordings'), ('h2', 'Captions are ready'), ('h3', 'Auto zoom'),
                                 ('body', 'Booya zooms in when three clicks land close together, then eases back out.'), ('small', 'Saved to Movies / Booya'), ('caption', 'Last edited 2 min ago'), ('overline', 'Recording'), ('timer', '00:42.18'))))
page('09', 'Typography', f'''{head('09', 'Typography')}
<div style="display:grid;grid-template-columns:560px 1fr;gap:64px;margin-top:40px">
 <div>
  <div style="font-size:250px;line-height:.85;font-weight:600;letter-spacing:-.05em">Aa</div>
  <div style="display:flex;gap:40px;margin-top:26px"><div><b style="font-size:20px">Geist</b><p class="note">Interface, headings, docs. Weights 400, 550, 600.</p></div><div><b class="mono" style="font-size:20px">Geist Mono</b><p class="note">Timers, shortcuts, file sizes, labels.</p></div></div>
  <ul class="facts" style="margin-top:20px">
   <li><b>Sentence case everywhere.</b> Buttons too.</li>
   <li><b>Tight headings</b>, -2% to -3.5% tracking. Body stays at 0.</li>
   <li><b>Numbers that change use tabular figures</b>, so the timer never jitters.</li>
   <li>Both fonts are SIL OFL 1.1. Files and licences are in fonts/.</li>
  </ul>
 </div>
 <div class="scale">{scale}</div>
</div>''')

icons_big = ''.join(f'<div class="ig"><div class="igb">{icon(n, 40, 1.5)}</div><span class="mono">{n}</span></div>' for n in ICON)
page('10', 'Iconography', f'''{head('10', 'Iconography')}
<div style="display:grid;grid-template-columns:1fr 420px;gap:60px;margin-top:40px">
 <div class="igs">{icons_big}</div>
 <div>
  <div class="over mono">Rules</div>
  <ul class="facts">
   <li><b>20 px grid</b>, 2 px padding, drawn at 1.5 px stroke.</li>
   <li><b>Round caps and joins</b>, 2 px corner radius on rectangles.</li>
   <li><b>One colour</b>, currentColor. Filled shapes only for state, like the record dot.</li>
   <li><b>Plain metaphors.</b> A mic is a mic. The zoom icon borrows the focus frame.</li>
   <li><b>Sizes:</b> 16, 20 and 24. Scale the stroke with the size, never below 1.25.</li>
  </ul>
  <div class="row" style="margin-top:26px;gap:22px">{icon('record',16,1.25)}{icon('record',20)}{icon('record',24,1.75)}{icon('zoom',16,1.25)}{icon('zoom',20)}{icon('zoom',24,1.75)}</div>
 </div>
</div>''')

def components():
    return f'''
 <div class="cgrid">
  <div class="cc"><div class="over mono">Recording toolbar</div><div class="stack">{toolbar()}<div style="opacity:.9">{toolbar('12:03', rec=False).replace('<span class="stop"><i></i>Stop</span>', '<span class="btn primary sm" style="height:36px;border-radius:10px;margin-left:4px">' + icon('record', 16, 2) + 'Resume</span>')}</div></div>
   <p class="note">Floating, 48 px tall, Surface with a soft shadow. The dot and Stop are the only Signal parts.</p></div>
  <div class="cc"><div class="over mono">Buttons</div>
   <div class="row wrap"><span class="btn primary">Start recording</span><span class="btn signal"><i style="width:10px;height:10px;border-radius:2.5px;background:currentColor;display:block"></i>Stop</span><span class="btn secondary">Export</span><span class="btn ghost">Cancel</span></div>
   <div class="row wrap" style="margin-top:12px"><span class="btn secondary focus">Focused</span><span class="btn danger">Delete recording</span><span class="btn primary dis">Disabled</span><span class="btn secondary sm">{icon('folder',16)}Show in folder</span></div></div>
  <div class="cc"><div class="over mono">Inputs</div>
   <div class="row" style="gap:16px;align-items:flex-start">
    <div class="field" style="width:250px"><label>File name</label><div class="in">Product tour 03</div><span class="hint">Saved as MP4</span></div>
    <div class="field" style="width:250px"><label>Start or stop</label><div class="in focus"><span class="kbd">Alt</span><span class="kbd">Shift</span><span class="kbd">R</span></div><span class="hint">Press a new shortcut</span></div>
   </div></div>
  <div class="cc"><div class="over mono">Toggles and choices</div>
   <div class="row" style="justify-content:space-between"><span>Auto zoom on clicks</span><span class="tg on"></span></div>
   <div class="row" style="justify-content:space-between;margin-top:12px"><span>Show captions while recording</span><span class="tg"></span></div>
   <div class="row" style="margin-top:14px"><span class="seg"><span class="on">Full screen</span><span>Window</span><span>Area</span></span></div></div>
  <div class="cc wide"><div class="over mono">Toasts</div>
   <div class="row wrap" style="gap:14px">
    <div class="toast"><span class="ti" style="color:var(--by-success)">{icon('check',18,2)}</span><div><b>Saved</b><span>Product tour 03.mp4 · 48 MB</span></div><span class="act">Show</span></div>
    <div class="toast"><span class="ti">{icon('captions',18)}</span><div><b>Captions are ready</b><span>Written on this device</span></div><span class="act">Open</span></div>
    <div class="toast"><span class="ti" style="color:var(--by-danger)">{icon('warning',18)}</span><div><b>Couldn't reach the mic</b><span>Check System Settings, then try again</span></div><span class="act">Retry</span></div>
   </div></div>
 </div>'''
page('11', 'Components, light', f'''{head('11', 'Components, light')}{components()}''')
page('12', 'Components, dark', f'''{head('12', 'Components, dark')}{components()}''', theme='by-d')

def settings(theme):
    side = ''.join(f'<div class="si{" on" if n=="Recording" else ""}">{icon(i,18)}<span>{n}</span></div>' for n, i in (('General', 'sliders'), ('Recording', 'record'), ('Captions', 'captions'), ('Zoom', 'zoom'), ('Shortcuts', 'keyboard'), ('Storage', 'folder')))
    rows = ''.join(f'<div class="sr"><div><b>{a}</b><span>{b}</span></div>{c}</div>' for a, b, c in (
        ('Show the floating toolbar', 'Stays above other windows while you record', '<span class="tg on"></span>'),
        ('Countdown', 'Gives you a moment before the first frame', '<span class="seg"><span>Off</span><span class="on">3 s</span><span>5 s</span></span>'),
        ('Record system audio', 'Mic is always recorded separately', '<span class="tg"></span>'),
        ('Frame rate', 'Higher looks smoother, files get bigger', '<span class="seg"><span>30</span><span class="on">60</span></span>'),
        ('Save to', '~/Movies/Booya', '<span class="btn secondary sm">Change</span>')))
    return f'''<div class="win {theme}" style="width:660px"><div class="tb"><i class="tl"></i><i class="tl"></i><i class="tl"></i><span style="margin-left:10px">Settings</span></div>
     <div style="display:grid;grid-template-columns:180px 1fr;height:430px"><div class="side">{side}<div style="margin-top:auto;padding:14px;display:flex;align-items:center;gap:8px;color:var(--by-muted);font-size:12px">{mark_svg(16)}<span class="mono">v0.8.0</span></div></div>
     <div style="padding:22px 24px;background:var(--by-surface)"><div style="font-weight:600;font-size:18px;margin-bottom:8px">Recording</div>{rows}</div></div></div>'''
page('13', 'Settings window', f'''{head('13', 'Settings window')}
<div style="display:flex;gap:40px;margin-top:40px;align-items:flex-start">{settings('by-l')}{settings('by-d')}</div>
<p class="note" style="margin-top:26px;width:900px">One window, a quiet sidebar and plain rows: label, one line of help, one control. No icons in rows, no Signal anywhere in settings.</p>''')

def screen(theme, w_, h_):
    lines = ''.join(f'<div class="ln" style="width:{v}%"></div>' for v in (40, 86, 70, 78, 0, 60, 82, 66, 74, 48))
    lines = lines.replace('<div class="ln" style="width:0%"></div>', '<div style="height:18px"></div>')
    return f'''<div class="scr {theme}" style="width:{w_}px;height:{h_}px"><div class="bar"><i></i><i></i><i></i></div><div style="padding:30px 36px">{lines}</div>
      {focus_frame(int(w_*0.42), int(h_*0.27), int(w_*0.44), int(h_*0.36))}
      <div style="position:absolute;left:{int(w_*0.64)}px;top:{int(h_*0.42)}px;color:{"#111110" if theme=="by-l" else "#EDEBE5"}">{mark_svg(26)}</div>
      <div style="position:absolute;left:0;right:0;bottom:26px;text-align:center"><span class="cap">Click the gear, then pick Export settings.</span></div></div>'''
page('14', 'Captions and the focus frame', f'''{head('14', 'Captions and the focus frame')}
<div style="display:flex;gap:34px;margin-top:38px">{screen('by-l', 700, 470)}{screen('by-d', 700, 470)}</div>
<div class="cols3" style="margin-top:28px">
 <p class="note"><b>Caption overlay.</b> Geist 500, 22/30, on Ink at 82%. Max two lines, 720 px wide, 26 px from the bottom. Same in light and dark, because it sits on the recording.</p>
 <p class="note"><b>Focus frame.</b> Four corners, 3 px, 26 px arms, 6 px outer radius, in Signal. It appears when three clicks cluster and eases in with the zoom.</p>
 <p class="note"><b>Cursor.</b> In recordings Booya can swap your pointer for the mark at 26 px, with a single ripple per click.</p>
</div>''')

def ripple_frames():
    out = ''
    for t in (0, 100, 200, 300, 400):
        p = bez(t/400); sc = 0.4 + 1.2*p; op = 0.6*(1-p)
        out += f'''<div class="fr"><svg width="120" height="100" viewBox="-46 -40 120 100"><circle r="{26*sc:.1f}" fill="none" stroke="#FF5A1F" stroke-width="3" opacity="{op:.2f}"/>
        <g transform="scale(.36)">{mark(0, 0, 100, '#111110', '#FF5A1F', rip=False, soft=0.035)}</g></svg><span class="mono">{t} ms</span><em class="mono">scale {sc:.2f} · {op:.2f}</em></div>'''
    return out
def trail_frames():
    out = ''
    for t, n in ((0, 0), (80, 2), (160, 1), (240, 0)):
        echoes = ''.join(f'<path transform="translate({14*k} {14*k})" d="{ghost_d(0,0,100)}" fill="{c}"/>' for k, c in ((2, '#E4E2DC'), (1, '#C9C6BE'))[2-n:]) if n else ''
        out += f'<div class="fr"><svg width="120" height="100" viewBox="-30 -14 170 170">{echoes}<path d="{ghost_d(0,0,100)} {eyes_d(0,0,100)}" fill="#111110" fill-rule="evenodd"/></svg><span class="mono">{t} ms</span><em class="mono">{n} echo frames</em></div>'
    return out
def blink_frames():
    out = ''
    for t, sy in ((0, 1), (40, .55), (60, .1), (80, .55), (120, 1)):
        out += f'<div class="fr"><svg width="120" height="100" viewBox="-30 -14 150 150"><path d="{ghost_d(0,0,100)}" fill="#111110"/>' + ''.join(
            f'<ellipse cx="{x*100:.1f}" cy="{y*100:.1f}" rx="7.8" ry="{7.8*sy:.2f}" fill="#F4F3EF"/>' for x, y in G['eyes']) + f'</svg><span class="mono">{t} ms</span><em class="mono">scaleY {sy}</em></div>'
    return out
page('15', 'Motion', f'''{head('15', 'Motion')}
<div style="display:grid;grid-template-columns:1fr 470px;gap:56px;margin-top:34px">
 <div>
  <div class="over mono">Click ripple · one ring · 400 ms · cubic-bezier(0.2, 0.8, 0.2, 1)</div><div class="frs">{ripple_frames()}</div>
  <div class="over mono" style="margin-top:26px">Blink · eyes scaleY 1 to 0.1 and back · 120 ms · record start and stop only</div><div class="frs">{blink_frames()}</div>
  <div class="over mono" style="margin-top:26px">Optional trail · record start only · 240 ms · flat tints, no blur</div><div class="frs">{trail_frames()}</div>
 </div>
 <div>
  <table class="mt">
   <tr><td>Ripple</td><td class="mono">400ms · scale 0.4 to 1.6 · opacity 0.6 to 0</td></tr>
   <tr><td>Blink</td><td class="mono">120ms · scaleY 0.1 · start and stop</td></tr>
   <tr><td>UI in</td><td class="mono">150 to 200ms · standard ease</td></tr>
   <tr><td>UI out</td><td class="mono">150ms · cubic-bezier(0.4, 0, 1, 1)</td></tr>
   <tr><td>Zoom</td><td class="mono">350ms in, 450ms out, standard ease</td></tr>
   <tr><td>Trail</td><td class="mono">optional · 240ms · 2 echo frames on record start</td></tr>
  </table>
  <div class="over mono" style="margin-top:24px">Reduced motion</div>
  <ul class="facts" style="margin-top:8px">
   <li>No ripple scale. Show a static ring for 200 ms, then fade.</li>
   <li>No blink, no trail. Zoom cuts instead of easing.</li>
   <li>Transitions drop to opacity only, 100 ms or less.</li>
  </ul>
  <pre class="code">.ripple {{
  animation: by-ripple 400ms
    cubic-bezier(.2, .8, .2, 1) forwards;
}}
@keyframes by-ripple {{
  from {{ transform: scale(.4); opacity: .6 }}
  to   {{ transform: scale(1.6); opacity: 0 }}
}}
@media (prefers-reduced-motion: reduce) {{
  .ripple {{ animation: none }}
}}</pre>
 </div>
</div>''')

micro = ''.join(f'<tr><td>{a}</td><td>{b}</td><td class="bad2">{c}</td></tr>' for a, b, c in (
    ('Start', 'Start recording', 'Let’s go!'), ('Stopped', 'Saved. 1 min 12 s, 48 MB.', 'Awesome! Your video is ready!!'),
    ('Captions', 'Captions are ready. Written on this device.', 'AI-powered captions generated by our magic engine'),
    ('Auto zoom', 'Zooms in when three clicks land close together.', 'Revolutionary smart zoom technology'),
    ('Error', 'Couldn’t reach the mic. Check System Settings, then try again.', 'Oops! Something went wrong :('),
    ('Empty', 'Nothing here yet. Press Alt Shift R to record.', 'Boo! No videos found, spooky!'),
    ('Delete', 'Delete this recording? This can’t be undone.', 'Are you sure you want to permanently delete???')))
page('16', 'Voice and tone', f'''{head('16', 'Voice and tone')}
<div style="display:grid;grid-template-columns:470px 1fr;gap:56px;margin-top:36px">
 <div>
  <div class="over mono">Tagline</div>
  <div style="font-size:64px;font-weight:600;letter-spacing:-.035em;line-height:1;margin-top:14px">Record it.<br>Booya.</div>
  <ul class="facts" style="margin-top:26px">
   <li><b>Plain.</b> Say what happened, then what to do.</li>
   <li><b>Short.</b> One idea per sentence. Most UI strings fit on one line.</li>
   <li><b>Friendly, a little playful.</b> The ghost joke lives in the mascot and the docs, not in buttons.</li>
   <li><b>Never cutesy.</b> No spooky puns in errors, no baby talk.</li>
   <li><b>No hype.</b> Skip revolutionary, magic, seamless, powerful, AI-powered.</li>
   <li><b>No exclamation marks in the UI.</b> Calm is the point.</li>
  </ul>
 </div>
 <table class="vt"><tr><th>Moment</th><th>Write</th><th>Not</th></tr>{micro}</table>
</div>''')

def dock():
    apps = ''.join(f'<div class="app" style="background:{c}">{icon(i, 30, 1.6)}</div>' for c, i in (('#E7E5DF', 'folder'), ('#D9DEE6', 'display'), ('#E3DED6', 'keyboard'), ('#DCE3DA', 'sliders')))
    return f'''<div class="desk"><div class="mbar"><span style="font-weight:700"></span><span style="font-weight:600">Booya</span><span>File</span><span>Edit</span><span>View</span><span>Window</span>
      <span style="margin-left:auto" class="trayi"><img src="{AS}/tray/macos/tray-recordingTemplate@2x.png" width="18" height="18"><span class="mono" style="font-size:12px">00:42</span></span><span>{icon('display', 16)}</span><span>Tue 10 Nov 18:15</span></div>
      <div class="win by-l" style="position:absolute;left:120px;top:64px;width:600px;height:330px"><div class="tb"><i class="tl"></i><i class="tl"></i><i class="tl"></i><span style="margin-left:10px;color:#6B6963;font-weight:500">Untitled design</span></div>
        <div style="padding:26px 30px;background:#fff;height:100%">{''.join(f'<div class="ln" style="width:{v}%"></div>' for v in (44, 86, 72, 80, 0, 58, 82))}</div>{focus_frame(300, 120, 230, 130)}<div style="position:absolute;left:420px;top:178px">{mark_svg(24)}</div></div>
      <div style="position:absolute;left:228px;top:360px">{toolbar()}</div>
      <div class="dock">{apps}<div class="app bo"><img src="{AS}/app-icon/png-macos/booya-icon-macos-128.png" width="62" height="62"><i></i></div><div class="dsep"></div><div class="app" style="background:#E4E2DC">{icon('folder', 30, 1.6)}</div></div></div>'''
def taskbar():
    apps = ''.join(f'<div class="tapp">{icon(i, 20)}</div>' for i in ('folder', 'display', 'keyboard'))
    return f'''<div class="wdesk"><div class="win by-d" style="position:absolute;left:40px;top:26px;width:400px;height:190px"><div class="tb"><span style="color:#8D8A82;font-weight:500">Booya</span></div><div style="padding:18px 20px;display:flex;flex-direction:column;gap:10px">
      <div class="row" style="justify-content:space-between;font-size:13px"><span>Auto zoom on clicks</span><span class="tg on"></span></div><div class="row" style="justify-content:space-between;font-size:13px"><span>Captions</span><span class="tg on"></span></div>
      <div class="row"><span class="btn primary sm">Start recording</span></div></div></div><div class="task"><div class="tapp">{icon('sliders', 20)}</div>{apps}<div class="tapp on"><img src="{AS}/app-icon/png/booya-icon-32.png" width="24" height="24"><i></i></div>
      <div style="margin-left:auto;display:flex;align-items:center;gap:14px;color:#EDEBE5;font-size:12px"><img src="{AS}/tray/windows/tray-recording-dark-16.png" width="16" height="16" style="image-rendering:pixelated">{icon('display', 16)}<span>18:15<br>10/11/2026</span></div></div></div>'''
page('17', 'In the wild: desktop', f'''{head('17', 'In the wild: desktop')}
<div style="display:grid;grid-template-columns:860px 1fr;gap:30px;margin-top:34px">
 <div>{dock()}<p class="note" style="margin-top:12px">macOS: Apple grid icon in the Dock, template glyph in the menu bar with the timer while recording.</p></div>
 <div>{taskbar()}<p class="note" style="margin-top:12px">Windows: 24 px icon in the taskbar, pixel-drawn 16 px tray glyph in paper on the dark taskbar.</p>
  <div class="trays">
   <div style="background:#ECECEC"><img src="{AS}/tray/macos/tray-idleTemplate@2x.png" width="18"><img src="{AS}/tray/macos/tray-recordingTemplate@2x.png" width="18"></div>
   <div style="background:#2A2A2A"><img src="{AS}/tray/windows/tray-idle-dark-16.png" width="16" style="image-rendering:pixelated"><img src="{AS}/tray/windows/tray-recording-dark-16.png" width="16" style="image-rendering:pixelated"></div>
   <div style="background:#ECECEC"><img class="px" src="{AS}/tray/windows/tray-idle-light-16.png" width="64"><img class="px" src="{AS}/tray/windows/tray-recording-light-16.png" width="64"></div>
  </div><p class="note">Idle and recording. Recording adds the ripple, nothing else changes.</p></div>
</div>''')

def gh():
    return f'''<div class="ghp"><div class="ghh"><span style="display:flex;align-items:center;gap:8px">{icon('folder',18)}<span>im45145v / <b>Booya</b></span><span class="pill">Public</span></span>
     <span style="margin-left:auto;display:flex;gap:8px"><span class="btn secondary sm">Watch</span><span class="btn secondary sm">Star</span></span></div>
     <div style="padding:18px 22px"><img src="{AS}/social/readme-header-light.png" style="width:100%;border-radius:8px;border:1px solid #DCDAD3"><div style="margin-top:14px;font-size:14px;color:#6B6963">Open source screen recorder with offline captions and auto zoom on clicks.</div>
     <div style="display:flex;gap:8px;margin-top:10px">{''.join(f'<span class="pill">{t}</span>' for t in ('tauri', 'screen-recorder', 'captions', 'rust'))}</div>
     <div style="margin-top:16px;border:1px solid #DCDAD3;border-radius:8px;font-size:13px">{''.join(f'<div style="display:flex;gap:10px;padding:8px 12px;border-top:{"0" if i == 0 else "1px solid #DCDAD3"}"><span style="color:#6B6963">{icon("folder" if d else "captions", 16)}</span><span>{n}</span><span style="margin-left:auto;color:#6B6963">{m}</span></div>' for i, (n, d, m) in enumerate((("src-tauri", 1, "Brand kit v1.0 icons"), ("src", 1, "New tokens"), ("brand", 1, "Brand kit v1.0"), ("README.md", 0, "New header"))))}</div></div></div>'''
def hero():
    return f'''<div class="hero by-l"><div class="nav">{lockup(14)}<span style="margin-left:auto">Features</span><span>Docs</span><span>GitHub</span><span class="btn primary sm">Download</span></div>
     <div style="display:grid;grid-template-columns:minmax(0,1fr) minmax(0,.9fr);gap:30px;padding:40px 44px">
      <div><div class="over mono">Free and open source</div><h3 style="font-size:52px;line-height:1;letter-spacing:-.035em;font-weight:600;margin-top:14px">Record it.<br>Booya.</h3>
       <p style="color:var(--by-muted);font-size:16px;line-height:1.5;margin-top:16px;width:330px">A screen recorder that zooms in when you click and writes captions on your own machine.</p>
       <div class="row" style="margin-top:22px"><span class="btn primary">Download for macOS</span><span class="btn secondary">Windows and Linux</span></div></div>
      <div style="position:relative;min-width:0"><div style="transform:scale(.72);transform-origin:left top;height:36px">{toolbar()}</div><div style="margin-top:14px;border-radius:12px;border:1px solid var(--by-rule);background:var(--by-surface);height:210px;position:relative;overflow:hidden">
        <div style="padding:20px">{''.join(f'<div class="ln" style="width:{v}%"></div>' for v in (50, 84, 70, 76, 58))}</div>{focus_frame(58, 44, 104, 84)}<div style="position:absolute;left:112px;top:80px">{mark_svg(18)}</div></div></div>
     </div></div>'''
page('18', 'In the wild: web', f'''{head('18', 'In the wild: web')}
<div style="display:grid;grid-template-columns:690px 1fr;gap:34px;margin-top:34px">{gh()}{hero()}</div>
<p class="note" style="margin-top:16px">Repository header for github.com/im45145v/Booya with the README banner, and a website hero.</p>''')

idx = [('assets/logo', 'Mark, wordmark, horizontal and stacked lockups in color, on-paper, on-ink, mono-black, mono-white. SVG plus PNG 1x and 2x.'),
       ('assets/app-icon', '1024 master SVG, Apple-grid macOS SVG, tuned 32 and 16 px SVGs, PNG 16 to 1024, booya.icns, booya.ico.'),
       ('assets/tauri-icons', 'Drop-in for src-tauri/icons: 32x32, 128x128, 128x128@2x, icon.png, icon.icns, icon.ico, Square*Logo, StoreLogo.'),
       ('assets/linux/hicolor', '16 to 512 PNGs and scalable SVG in the freedesktop layout.'),
       ('assets/tray', 'macOS template PNG @1x @2x, PDF and SVG for idle and recording. Windows .ico for light and dark taskbars.'),
       ('assets/favicon', 'favicon.svg with dark mode, favicon.ico, apple-touch-icon, 192 and 512 PNGs, site.webmanifest.'),
       ('assets/social', 'GitHub social preview, README headers light and dark, og image, X and LinkedIn post template with editable HTML.'),
       ('assets/mascot', 'Idle, blinking, zooming, saving, error. Light and dark, SVG and PNG.'),
       ('tokens', 'tokens.json, tokens.css, tailwind.preset.js, CONTRAST.md and contrast-report.json.'),
       ('fonts', 'Geist and Geist Mono variable TTFs with their OFL licences.'),
       ('guideline', 'This document as HTML and PDF, plus page PNGs.'),
       ('IMPLEMENTATION.md', 'Step by step notes for the brand kit PR in the Tauri app.')]
page('19', 'Asset index', f'''{head('19', 'Asset index')}
<table class="ai" style="margin-top:36px">{''.join(f'<tr><td class="mono">{a}</td><td>{b}</td></tr>' for a, b in idx)}</table>
<div style="position:absolute;right:96px;bottom:96px;display:flex;align-items:center;gap:20px">{mark_svg(64)}<span class="note" style="margin:0">Questions about the brand go in the repo as an issue with the <span class="mono">brand</span> label.</span></div>''')

PCSS = """
*{box-sizing:border-box;margin:0;padding:0}
@page{size:1600px 900px;margin:0}
body{background:#888;-webkit-font-smoothing:antialiased}
.page{width:1600px;height:900px;position:relative;overflow:hidden;padding:64px 96px;background:var(--by-bg);break-after:page}
.ph{display:flex;align-items:center;gap:18px;font-size:13px;color:var(--by-muted);padding-bottom:18px;border-bottom:1px solid var(--by-rule)}
.ph .mono{color:var(--by-signal-text);font-weight:500}.ph .pt{font-weight:500;color:var(--by-text)}.ph .brand{margin-left:auto;color:var(--by-text)}
.pf{position:absolute;left:96px;right:96px;bottom:34px;display:flex;justify-content:space-between;font:500 12px/1 'Geist Mono';color:var(--by-muted)}
.over{font:500 12px/1 'Geist Mono';letter-spacing:.08em;text-transform:uppercase;color:var(--by-muted)}
.h{font-size:56px;line-height:1.02;letter-spacing:-.035em;font-weight:600}
.lead{font-size:19px;line-height:1.55;color:var(--by-text);max-width:600px}
.note{font-size:14px;line-height:1.55;color:var(--by-muted);margin-top:12px}.note b{color:var(--by-text);font-weight:600}
.cols2{display:grid;grid-template-columns:1fr 1fr;gap:80px}.cols3{display:grid;grid-template-columns:1fr 1fr 1fr;gap:40px}.grid3{display:grid;grid-template-columns:1fr 1fr 1fr;gap:22px}
.toc{columns:2;column-gap:80px;margin-top:40px;width:1100px}.toc-i{display:flex;gap:24px;font-size:22px;padding:13px 0;border-bottom:1px solid var(--by-rule);break-inside:avoid;letter-spacing:-.01em}.toc-i .mono{color:var(--by-signal-text);font-size:15px;padding-top:5px}
.principles{display:grid;grid-template-columns:1fr 1fr;gap:20px;align-content:start}
.pr{background:var(--by-surface);border:1px solid var(--by-rule);border-radius:16px;padding:24px 24px 26px}.pr .mono{color:var(--by-signal-text);font-size:13px}.pr b{display:block;font-size:22px;letter-spacing:-.015em;margin:12px 0 8px;font-weight:600}.pr p{font-size:15px;line-height:1.5;color:var(--by-muted)}
.facts{list-style:none;margin-top:22px}.facts li{font-size:15.5px;line-height:1.5;color:var(--by-muted);padding:11px 0;border-top:1px solid var(--by-rule)}.facts li b{color:var(--by-text);font-weight:600}
.poses{display:grid;grid-template-columns:repeat(3,1fr);gap:14px}.pose{background:var(--by-surface);border:1px solid var(--by-rule);border-radius:16px;padding:10px 14px 16px}.pose img{width:100%;display:block}
.pose figcaption b{display:block;font-size:15px;font-weight:600}.pose figcaption span{font-size:13px;color:var(--by-muted)}
.poses .pose:nth-child(4),.poses .pose:nth-child(5){}
.ba{display:flex;align-items:center;gap:30px;margin-top:56px}.bai{display:flex;flex-direction:column;gap:18px;align-items:flex-start}.arrow{color:var(--by-muted)}
.smalls{display:flex;align-items:flex-end;gap:18px}.px{image-rendering:pixelated}
.card{background:var(--by-surface);border:1px solid var(--by-rule);border-radius:16px;padding:22px 24px}
.cbox{height:250px;display:flex;align-items:center;justify-content:center;margin-top:10px}
.cs{position:relative;outline:1px dashed #FF5A1F;outline-offset:0}.csl{position:absolute;outline:1px solid rgba(255,90,31,.35)}
.mins{display:flex;align-items:flex-end;gap:34px;margin-top:22px}.mins>div{display:flex;flex-direction:column;gap:10px;align-items:flex-start}.mins .mono{font-size:12px;color:var(--by-muted)}
.ways{display:grid;grid-template-columns:repeat(5,1fr);gap:14px;margin-top:14px}.donts{display:grid;grid-template-columns:repeat(4,1fr);gap:14px;margin-top:14px}
.dn .dnb{height:162px;border-radius:14px;border:1px solid var(--by-rule);display:flex;align-items:center;justify-content:center;position:relative;overflow:hidden}
.ways .dnb{height:104px}
.dnl{display:flex;align-items:center;gap:8px;font-size:14px;margin-top:10px;color:var(--by-danger)}.dnl span{color:var(--by-text)}.dnl.ok{color:var(--by-success)}
.blush{position:absolute;width:22px;height:12px;border-radius:50%;background:#F7B6C8;left:calc(50% - 20px);top:calc(50% + 22px)}.blush.b2{left:calc(50% + 18px)}
.sws{display:grid;grid-template-columns:repeat(3,1fr);gap:14px;margin-top:14px}.sw{display:flex;flex-direction:column;gap:3px}.swc{height:110px;border-radius:14px;border:1px solid rgba(0,0,0,.08);margin-bottom:8px}
.sw b{font-size:15px;font-weight:600}.sw .mono{font-size:13px;color:var(--by-muted)}.sw em{font-style:normal;font-size:13px;color:var(--by-muted)}
.ratio{display:flex;height:56px;border-radius:12px;overflow:hidden;border:1px solid var(--by-rule);margin-top:14px}.ratio i{font:500 12px/56px 'Geist Mono';font-style:normal;padding-left:12px;white-space:nowrap;overflow:hidden;color:#111110;border-right:1px solid var(--by-rule)}
.sems{margin-top:14px;display:flex;flex-direction:column;gap:10px}.sem{display:grid;grid-template-columns:26px 26px 1fr;gap:8px;align-items:center;font-size:14px}.sem i{height:26px;border-radius:7px;border:1px solid rgba(0,0,0,.1)}
.sem .mono{grid-column:3;font-size:11.5px;color:var(--by-muted);margin-top:-8px}.sem b{font-weight:600}
.ct{border-collapse:collapse;font-size:12.5px;width:100%;align-self:start}.ct th{text-align:left;font:500 11px 'Geist Mono';text-transform:uppercase;letter-spacing:.06em;color:var(--by-muted);padding:0 6px 8px}
.ct td{padding:3px 6px;border-top:1px solid var(--by-rule);line-height:1.3}.ct .mono{font-size:10.5px;color:var(--by-muted);white-space:nowrap}.ct .ok{color:var(--by-success);font-weight:600}.ct .bad{color:var(--by-danger);font-weight:600}
.chip2{display:inline-block;width:30px;height:20px;border-radius:5px;font:600 11px/20px 'Geist';text-align:center;border:1px solid rgba(0,0,0,.08)}
.scale{display:flex;flex-direction:column}.ts{padding:10px 0;border-top:1px solid var(--by-rule)}.ts .mono{font-size:11.5px;color:var(--by-muted);display:block;margin-bottom:4px}
.igs{display:grid;grid-template-columns:repeat(6,1fr);gap:14px}.ig{display:flex;flex-direction:column;gap:8px;align-items:center}.igb{width:100%;height:110px;border-radius:14px;background:var(--by-surface);border:1px solid var(--by-rule);display:grid;place-items:center;
 background-image:linear-gradient(var(--by-rule) 1px,transparent 1px),linear-gradient(90deg,var(--by-rule) 1px,transparent 1px);background-size:4px 4px;background-position:center;background-blend-mode:normal}
.igb{background-image:none}.ig .mono{font-size:12px;color:var(--by-muted)}
.cgrid{display:grid;grid-template-columns:1fr 1fr;gap:18px;margin-top:30px}.cc{background:var(--by-surface);border:1px solid var(--by-rule);border-radius:16px;padding:22px 24px;display:flex;flex-direction:column;gap:14px}
.cc .over{margin-bottom:2px}.cc.wide{grid-column:1/3}.stack{display:flex;gap:16px;flex-wrap:wrap}.row.wrap{flex-wrap:wrap}.cc .note{margin-top:0}
.by-d .cc .tbar{background:var(--by-raised)}.by-d .toast{background:var(--by-raised)}
.side{background:var(--by-bg);border-right:1px solid var(--by-rule);padding:12px 10px;display:flex;flex-direction:column;gap:2px}.si{display:flex;gap:10px;align-items:center;padding:8px 10px;border-radius:8px;font-size:13.5px;color:var(--by-muted)}
.si.on{background:var(--by-surface);color:var(--by-text);font-weight:550;box-shadow:0 0 0 1px var(--by-rule)}.sr{display:flex;align-items:center;justify-content:space-between;gap:20px;padding:13px 0;border-top:1px solid var(--by-rule)}
.sr b{display:block;font-size:14px;font-weight:550}.sr span{font-size:12.5px;color:var(--by-muted)}.sr:first-of-type{border-top:0}
.scr{position:relative;border-radius:16px;overflow:hidden;background:var(--by-surface);border:1px solid var(--by-rule)}.scr .bar{height:34px;border-bottom:1px solid var(--by-rule);display:flex;align-items:center;gap:7px;padding:0 14px}.scr .bar i{width:10px;height:10px;border-radius:50%;background:var(--by-rule)}
.ln{height:10px;border-radius:5px;background:var(--by-rule);margin:0 0 13px}
.frs{display:flex;gap:14px;margin-top:14px}.fr{background:var(--by-surface);border:1px solid var(--by-rule);border-radius:14px;padding:6px 8px 10px;display:flex;flex-direction:column;align-items:center;gap:3px;width:150px}
.fr .mono{font-size:12px}.fr em{font-style:normal;font-size:11px;color:var(--by-muted)}
.mt{width:100%;border-collapse:collapse;font-size:14px}.mt td{padding:9px 0;border-top:1px solid var(--by-rule)}.mt td:first-child{font-weight:600;width:90px}.mt .mono{font-size:12.5px;color:var(--by-muted)}
.code{margin-top:16px;font-size:11.5px!important;background:#111110;color:#EDEBE5;font:12px/1.6 'Geist Mono';padding:14px 16px;border-radius:12px;white-space:pre}
.vt{width:100%;border-collapse:collapse;font-size:14.5px;align-self:start}.vt th{text-align:left;font:500 11px 'Geist Mono';letter-spacing:.06em;text-transform:uppercase;color:var(--by-muted);padding:0 10px 10px 0}
.vt td{padding:12px 14px 12px 0;border-top:1px solid var(--by-rule);vertical-align:top}.vt td:first-child{font-weight:600;width:100px}.bad2{color:var(--by-muted);text-decoration:line-through;text-decoration-color:var(--by-danger)}
.desk{position:relative;height:520px;border-radius:16px;overflow:hidden;background:#C9C4B8;border:1px solid var(--by-rule)}
.desk{background:linear-gradient(#CFCAC0,#CFCAC0)}
.mbar{height:26px;background:rgba(244,243,239,.86);display:flex;align-items:center;gap:18px;padding:0 14px;font-size:13px;color:#111110}.trayi{display:flex;align-items:center;gap:6px;background:rgba(17,17,16,.08);padding:2px 8px;border-radius:5px}
.dock{position:absolute;left:50%;bottom:14px;transform:translateX(-50%);display:flex;gap:12px;align-items:center;padding:10px 14px;border-radius:24px;background:rgba(244,243,239,.6);border:1px solid rgba(255,255,255,.6)}
.app{width:62px;height:62px;border-radius:15px;display:grid;place-items:center;color:#6B6963;position:relative}.app.bo{background:none}.app.bo i{position:absolute;bottom:-8px;left:28px;width:5px;height:5px;border-radius:50%;background:#111110}.dsep{width:1px;height:50px;background:rgba(17,17,16,.2)}
.wdesk{position:relative;height:300px;border-radius:16px;overflow:hidden;background:#BFC3C7;border:1px solid var(--by-rule)}.task{position:absolute;left:0;right:0;bottom:0;height:48px;background:#1C1B19;display:flex;align-items:center;gap:6px;padding:0 12px}
.tapp{width:40px;height:40px;border-radius:6px;display:grid;place-items:center;color:#8D8A82;position:relative}.tapp.on{background:rgba(255,255,255,.08)}.tapp.on i{position:absolute;bottom:2px;left:14px;width:12px;height:3px;border-radius:2px;background:#EDEBE5}
.trays{display:grid;grid-template-columns:1fr 1fr 1fr;gap:10px;margin-top:16px}.trays>div{height:90px;border-radius:12px;display:flex;align-items:center;justify-content:center;gap:16px}
.ghp{background:#FFFFFF;border:1px solid #DCDAD3;border-radius:14px;overflow:hidden;color:#111110}.ghh{display:flex;align-items:center;padding:14px 22px;border-bottom:1px solid #DCDAD3;font-size:15px}
.pill{font-size:12px;border:1px solid #DCDAD3;border-radius:999px;padding:3px 9px;color:#6B6963}
.hero{border:1px solid var(--by-rule);border-radius:14px;overflow:hidden;background:var(--by-bg)}.nav{display:flex;align-items:center;gap:22px;padding:14px 22px;border-bottom:1px solid var(--by-rule);font-size:13.5px}
.ai{width:100%;border-collapse:collapse;font-size:15px}.ai td{padding:12px 0;border-top:1px solid var(--by-rule);vertical-align:top}.ai td.mono{width:300px;font-size:13.5px;color:var(--by-signal-text)}
"""

def build():
    pages_html = ''
    for i, (sec, title, body, theme, num, cls) in enumerate(PAGES):
        foot = f'<div class="pf"><span>Booya brand guideline · {VERSION}</span><span>{i+1:02d} / {len(PAGES):02d}</span></div>' if num else ''
        pages_html += f'<section class="page {theme} {cls}">{body}{foot}</section>\n'
    html = f'<!doctype html><html lang="en"><head><meta charset="utf-8"><title>Booya brand guideline</title><style>{css("../fonts")}{PCSS}</style></head><body>{pages_html}</body></html>'
    w(f'{GD}/booya-brand-guideline.html', html)
    from playwright.sync_api import sync_playwright
    with sync_playwright() as p:
        b = p.chromium.launch(executable_path='/usr/bin/google-chrome')
        pg = b.new_page(viewport={'width': 1600, 'height': 900})
        pg.goto(f'file://{GD}/booya-brand-guideline.html'); pg.wait_for_load_state('networkidle'); pg.evaluate('document.fonts.ready'); pg.wait_for_timeout(300)
        os.makedirs(f'{GD}/pages', exist_ok=True)
        for i, el in enumerate(pg.query_selector_all('section.page')):
            el.screenshot(path=f'{GD}/pages/page-{i+1:02d}.png')
        pg.pdf(path=f'{GD}/booya-brand-guideline.pdf', width='1600px', height='900px', print_background=True, margin={'top': '0', 'bottom': '0', 'left': '0', 'right': '0'})
        b.close()
if __name__ == '__main__':
    build()
