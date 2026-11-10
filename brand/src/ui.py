"""Shared HTML/CSS building blocks for the guideline, social images and mockups."""
from geom import *
from small import tray18, tray16
ICON = {  # 20px grid, 1.5 stroke, round caps and joins
 'record': '<circle cx="10" cy="10" r="5.5"/>',
 'stop': '<rect x="5" y="5" width="10" height="10" rx="2"/>',
 'pause': '<path d="M7.5 5v10M12.5 5v10"/>',
 'mic': '<rect x="7.25" y="2.75" width="5.5" height="9" rx="2.75"/><path d="M4.5 9.5a5.5 5.5 0 0 0 11 0M10 15v2.5"/>',
 'captions': '<rect x="2.75" y="4.25" width="14.5" height="11.5" rx="2.5"/><path d="M6 12.25h3.5M11.5 12.25H14M6 9h8"/>',
 'zoom': '<path d="M2.75 7V2.75H7M13 2.75h4.25V7M17.25 13v4.25H13M7 17.25H2.75V13"/><circle cx="10" cy="10" r="1.6" fill="currentColor" stroke="none"/>',
 'cursor': '<path d="M5 3v12.5l3.2-3 2.3 4.7 2.2-1-2.2-4.6H15z"/>',
 'sliders': '<path d="M3 6h7M14 6h3M3 14h3M10 14h7"/><circle cx="12" cy="6" r="2"/><circle cx="8" cy="14" r="2"/>',
 'folder': '<path d="M2.75 5.5a1.5 1.5 0 0 1 1.5-1.5h3.5l2 2h6a1.5 1.5 0 0 1 1.5 1.5v7.25a1.5 1.5 0 0 1-1.5 1.5H4.25a1.5 1.5 0 0 1-1.5-1.5z"/>',
 'export': '<path d="M10 12.5V3M6.5 6.5 10 3l3.5 3.5M4 11v4.25c0 .7.55 1.25 1.25 1.25h9.5c.7 0 1.25-.55 1.25-1.25V11"/>',
 'keyboard': '<rect x="2.25" y="5" width="15.5" height="10" rx="2"/><path d="M5.5 8.25h.01M8.5 8.25h.01M11.5 8.25h.01M14.5 8.25h.01M6.5 11.75h7"/>',
 'display': '<rect x="2.25" y="3.25" width="15.5" height="10.5" rx="2"/><path d="M7 17h6M10 13.75V17"/>',
 'check': '<path d="M4.5 10.5 8.25 14 15.5 6.5"/>',
 'close': '<path d="M5.5 5.5l9 9M14.5 5.5l-9 9"/>',
 'chevron': '<path d="M7.5 5l5 5-5 5"/>',
 'warning': '<path d="M10 3.25 17.25 16H2.75z"/><path d="M10 8.25v3.5M10 14h.01"/>',
}
def icon(name, size=20, sw=1.5, cls=''):
    return (f'<svg class="ic {cls}" width="{size}" height="{size}" viewBox="0 0 20 20" fill="none" stroke="currentColor" '
            f'stroke-width="{sw}" stroke-linecap="round" stroke-linejoin="round">{ICON[name]}</svg>')

def mark_svg(h=40, ink='currentColor', sig='var(--by-signal)', rip=True, eyes=True):
    """Inline mark sized by body height; eyes are true holes."""
    s = 100
    x0 = -(G['rip_r'] + G['rip_w']/2)*s if rip else -2
    bw = G['shoulder']*s; bh = (G['stem'] + G['amp']*1.3)*s
    vb = f'{x0:.1f} {x0:.1f} {bw - x0 + 2:.1f} {bh - x0 + 2:.1f}'
    ratio = (bw - x0 + 2)/(bh - x0 + 2)
    hh = h*(bh - x0 + 2)/bh
    return f'<svg class="mk" viewBox="{vb}" width="{hh*ratio:.1f}" height="{hh:.1f}">{mark(0, 0, s, ink, sig, eyes=eyes, rip=rip, soft=0.035)}</svg>'

def wordmark_svg(h=40, color='currentColor'):
    from wordmark import word_path
    d, ww, m = word_path('booya', 100)
    top, bot = m['b_top'], m['y_bot']
    return f'<svg class="wm" viewBox="0 {-top:.1f} {ww:.1f} {top+bot:.1f}" height="{h*(top+bot)/top:.1f}" width="{h*ww/top:.1f}"><path d="{d}" fill="{color}"/></svg>'

def lockup(h=40, color='currentColor', sig='var(--by-signal)'):
    """Horizontal lockup inline: mark body ~1.58 cap height, gap 0.42 cap. h = cap height."""
    return (f'<span class="lk" style="display:inline-flex;align-items:flex-end;gap:{h*0.42:.1f}px;color:{color}">'
            f'<span style="margin-bottom:{-h*0.10:.1f}px">{mark_svg(h*1.58, "currentColor", sig)}</span>'
            f'<span style="margin-bottom:{-h*0.21:.1f}px">{wordmark_svg(h, "currentColor")}</span></span>')

CSS = """
@font-face{font-family:'Geist';src:url('FONTDIR/Geist-Variable.ttf');font-weight:100 900}
@font-face{font-family:'Geist Mono';src:url('FONTDIR/GeistMono-Variable.ttf');font-weight:100 900}
.by-l{--by-bg:#F4F3EF;--by-surface:#FFFFFF;--by-raised:#FFFFFF;--by-text:#111110;--by-muted:#6B6963;--by-rule:#DCDAD3;--by-border:#8C8A83;--by-signal:#FF5A1F;--by-signal-text:#BF4317;--by-on-signal:#111110;--by-success:#1D7A45;--by-warning:#975A00;--by-danger:#C2301F;--by-focus:#111110;--by-shadow:0 1px 2px rgba(17,17,16,.06),0 8px 24px rgba(17,17,16,.10);--by-shadow-lg:0 2px 4px rgba(17,17,16,.06),0 18px 48px rgba(17,17,16,.16);--by-chip:#F4F3EF}
.by-d{--by-bg:#111110;--by-surface:#1C1B19;--by-raised:#24231F;--by-text:#EDEBE5;--by-muted:#8D8A82;--by-rule:#2C2B28;--by-border:#76736C;--by-signal:#FF6A33;--by-signal-text:#FF6A33;--by-on-signal:#111110;--by-success:#4CC385;--by-warning:#F0B24E;--by-danger:#FF6B5B;--by-focus:#EDEBE5;--by-shadow:0 1px 2px rgba(0,0,0,.4),0 8px 24px rgba(0,0,0,.45);--by-shadow-lg:0 2px 4px rgba(0,0,0,.4),0 18px 48px rgba(0,0,0,.55);--by-chip:#24231F}
.by-l,.by-d{color:var(--by-text);font-family:'Geist',sans-serif}
.mono{font-family:'Geist Mono',monospace}
.ic{display:block;flex:none}
/* toolbar */
.tbar{display:inline-flex;align-items:center;gap:6px;height:48px;padding:0 6px 0 16px;background:var(--by-surface);border:1px solid var(--by-rule);border-radius:14px;box-shadow:var(--by-shadow)}
.tbar .dot{width:10px;height:10px;border-radius:50%;background:var(--by-signal);margin-right:2px}
.tbar .time{font:500 15px/1 'Geist Mono';font-variant-numeric:tabular-nums;width:56px;letter-spacing:-.01em}
.tbar .sep{width:1px;height:22px;background:var(--by-rule);margin:0 4px}
.tbar .ib{width:34px;height:34px;border-radius:9px;display:grid;place-items:center;color:var(--by-text)}
.tbar .ib.on{background:var(--by-chip)}
.tbar .stop{display:flex;align-items:center;gap:8px;height:36px;padding:0 14px 0 12px;border-radius:10px;background:var(--by-signal);color:var(--by-on-signal);font:600 14px/1 'Geist';margin-left:4px}
.tbar .stop i{width:10px;height:10px;border-radius:2.5px;background:currentColor;display:block}
/* buttons */
.btn{display:inline-flex;align-items:center;gap:8px;height:36px;padding:0 14px;border-radius:10px;font:550 14px/1 'Geist';border:1px solid transparent;white-space:nowrap}
.btn.primary{background:var(--by-text);color:var(--by-bg)}
.btn.signal{background:var(--by-signal);color:var(--by-on-signal)}
.btn.secondary{background:var(--by-surface);color:var(--by-text);border-color:var(--by-rule)}
.btn.ghost{background:transparent;color:var(--by-text)}
.btn.danger{background:transparent;color:var(--by-danger);border-color:var(--by-rule)}
.btn.focus{outline:2px solid var(--by-focus);outline-offset:2px}
.btn.dis{opacity:.4}
.btn.sm{height:28px;padding:0 10px;font-size:13px;border-radius:8px}
/* inputs */
.field{display:flex;flex-direction:column;gap:6px;font-size:13px}
.field label{font-weight:550}
.field .in{height:36px;border:1px solid var(--by-border);border-radius:10px;background:var(--by-surface);display:flex;align-items:center;padding:0 12px;font-size:14px;gap:8px}
.field .in.focus{outline:2px solid var(--by-focus);outline-offset:2px}
.field .hint{color:var(--by-muted);font-size:12px}
.field .in .ph{color:var(--by-muted)}
.kbd{font:500 12px/1 'Geist Mono';padding:4px 6px;border:1px solid var(--by-rule);border-bottom-width:2px;border-radius:6px;background:var(--by-surface)}
/* toggle */
.tg{width:36px;height:22px;border-radius:999px;background:var(--by-rule);position:relative;flex:none}
.tg::after{content:'';position:absolute;left:3px;top:3px;width:16px;height:16px;border-radius:50%;background:var(--by-surface);box-shadow:0 1px 2px rgba(0,0,0,.2)}
.tg.on{background:var(--by-text)}
.tg.on::after{left:17px;background:var(--by-bg)}
.row{display:flex;align-items:center;gap:12px}
/* toast */
.toast{display:flex;align-items:center;gap:12px;padding:12px 14px 12px 12px;border-radius:12px;background:var(--by-surface);border:1px solid var(--by-rule);box-shadow:var(--by-shadow);font-size:14px;min-width:300px}
.toast .ti{width:28px;height:28px;border-radius:8px;display:grid;place-items:center;background:var(--by-chip)}
.toast b{font-weight:600;display:block}
.toast span{color:var(--by-muted);font-size:13px}
.toast .act{margin-left:auto;font-weight:600;font-size:13px}
/* segmented */
.seg{display:inline-flex;padding:3px;border-radius:10px;background:var(--by-chip);gap:2px;font-size:13px;font-weight:550}
.seg span{padding:6px 12px;border-radius:8px;color:var(--by-muted)}
.seg span.on{background:var(--by-surface);color:var(--by-text);box-shadow:0 1px 2px rgba(0,0,0,.08)}
/* window */
.win{background:var(--by-bg);border:1px solid var(--by-rule);border-radius:14px;box-shadow:var(--by-shadow-lg);overflow:hidden}
.win .tb{height:40px;display:flex;align-items:center;gap:8px;padding:0 14px;border-bottom:1px solid var(--by-rule);background:var(--by-surface);font-size:13px;font-weight:600}
.win .tl{width:12px;height:12px;border-radius:50%;background:var(--by-rule)}
/* caption overlay */
.cap{display:inline-block;background:rgba(17,17,16,.82);color:#F4F3EF;font:500 22px/30px 'Geist';letter-spacing:-.005em;padding:8px 16px;border-radius:10px;max-width:720px;text-align:center}
/* focus frame */
.ff{position:absolute;border:0;pointer-events:none}
.ff i{position:absolute;width:26px;height:26px;border:3px solid var(--by-signal)}
.ff i:nth-child(1){left:0;top:0;border-right:0;border-bottom:0;border-top-left-radius:6px}
.ff i:nth-child(2){right:0;top:0;border-left:0;border-bottom:0;border-top-right-radius:6px}
.ff i:nth-child(3){right:0;bottom:0;border-left:0;border-top:0;border-bottom-right-radius:6px}
.ff i:nth-child(4){left:0;bottom:0;border-right:0;border-top:0;border-bottom-left-radius:6px}
"""
def css(fontdir): return CSS.replace('FONTDIR', fontdir)

def toolbar(t='0:42', rec=True):
    return (f'<div class="tbar">{"<span class=dot></span>" if rec else ""}<span class="time">{t}</span><span class="sep"></span>'
            f'<span class="ib">{icon("mic")}</span><span class="ib on">{icon("captions")}</span><span class="ib on">{icon("zoom")}</span>'
            f'<span class="sep"></span><span class="ib">{icon("pause")}</span><span class="stop"><i></i>Stop</span></div>')

def focus_frame(x, y, w, h):
    return f'<div class="ff" style="left:{x}px;top:{y}px;width:{w}px;height:{h}px"><i></i><i></i><i></i><i></i></div>'
