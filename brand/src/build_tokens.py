import json, os
from common import ROOT, w
def lum(h):
    h = h.lstrip('#'); c = [int(h[i:i+2], 16)/255 for i in (0, 2, 4)]
    c = [x/12.92 if x <= 0.03928 else ((x+0.055)/1.055)**2.4 for x in c]
    return 0.2126*c[0] + 0.7152*c[1] + 0.0722*c[2]
def cr(a, b):
    la, lb = sorted((lum(a), lum(b)), reverse=True); return (la+0.05)/(lb+0.05)

LIGHT = dict(paper='#F4F3EF', surface='#FFFFFF', ink='#111110', graphite='#6B6963', rule='#DCDAD3', signal='#FF5A1F')
DARK = dict(night='#111110', surface='#1C1B19', paper='#EDEBE5', stone='#8D8A82', rule='#2C2B28', signal='#FF6A33')

# find the accessible Signal text variant: same hue family, darkened until 4.5:1 on Paper and Surface
def darken(h, f):
    h = h.lstrip('#'); c = [int(h[i:i+2], 16) for i in (0, 2, 4)]
    return '#' + ''.join(f'{round(x*f):02X}' for x in c)
f = 1.0
while cr(darken('#FF5A1F', f), LIGHT['paper']) < 4.6: f -= 0.01
SIGNAL_TEXT = darken('#FF5A1F', f)

SEM_L = dict(success='#1D7A45', warning='#975A00', danger='#C2301F', recording=LIGHT['signal'], focus=LIGHT['ink'],
             signal_text=SIGNAL_TEXT, border_strong='#8C8A83', on_signal=LIGHT['ink'], text=LIGHT['ink'], text_muted=LIGHT['graphite'])
SEM_D = dict(success='#4CC385', warning='#F0B24E', danger='#FF6B5B', recording=DARK['signal'], focus=DARK['paper'],
             signal_text=DARK['signal'], border_strong='#76736C', on_signal='#111110', text=DARK['paper'], text_muted=DARK['stone'])

PAIRS = [
 ('light', 'Ink on Paper', LIGHT['ink'], LIGHT['paper'], 'text'),
 ('light', 'Ink on Surface', LIGHT['ink'], LIGHT['surface'], 'text'),
 ('light', 'Graphite on Paper', LIGHT['graphite'], LIGHT['paper'], 'text'),
 ('light', 'Graphite on Surface', LIGHT['graphite'], LIGHT['surface'], 'text'),
 ('light', 'Signal on Paper', LIGHT['signal'], LIGHT['paper'], 'text'),
 ('light', 'Signal on Surface', LIGHT['signal'], LIGHT['surface'], 'text'),
 ('light', 'Signal text on Paper', SIGNAL_TEXT, LIGHT['paper'], 'text'),
 ('light', 'Signal text on Surface', SIGNAL_TEXT, LIGHT['surface'], 'text'),
 ('light', 'Ink on Signal (button label)', LIGHT['ink'], LIGHT['signal'], 'text'),
 ('light', 'White on Signal (button label)', '#FFFFFF', LIGHT['signal'], 'text'),
 ('light', 'Signal on Paper (record dot, non-text)', LIGHT['signal'], LIGHT['paper'], 'ui'),
 ('light', 'Signal on Surface (record dot in toolbar)', LIGHT['signal'], LIGHT['surface'], 'ui'),
 ('light', 'Rule on Paper (divider)', LIGHT['rule'], LIGHT['paper'], 'ui'),
 ('light', 'Border strong on Surface (inputs)', SEM_L['border_strong'], LIGHT['surface'], 'ui'),
 ('light', 'Success on Surface', SEM_L['success'], LIGHT['surface'], 'text'),
 ('light', 'Warning on Surface', SEM_L['warning'], LIGHT['surface'], 'text'),
 ('light', 'Danger on Surface', SEM_L['danger'], LIGHT['surface'], 'text'),
 ('dark', 'Paper on Night', DARK['paper'], DARK['night'], 'text'),
 ('dark', 'Paper on Surface', DARK['paper'], DARK['surface'], 'text'),
 ('dark', 'Stone on Night', DARK['stone'], DARK['night'], 'text'),
 ('dark', 'Stone on Surface', DARK['stone'], DARK['surface'], 'text'),
 ('dark', 'Signal on Night', DARK['signal'], DARK['night'], 'text'),
 ('dark', 'Signal on Surface', DARK['signal'], DARK['surface'], 'text'),
 ('dark', 'Night on Signal (button label)', DARK['night'], DARK['signal'], 'text'),
 ('dark', 'Rule on Night (divider)', DARK['rule'], DARK['night'], 'ui'),
 ('dark', 'Border strong on Surface (inputs)', SEM_D['border_strong'], DARK['surface'], 'ui'),
 ('dark', 'Success on Surface', SEM_D['success'], DARK['surface'], 'text'),
 ('dark', 'Warning on Surface', SEM_D['warning'], DARK['surface'], 'text'),
 ('dark', 'Danger on Surface', SEM_D['danger'], DARK['surface'], 'text'),
]
def report():
    rows = []
    for mode, name, fg, bg, kind in PAIRS:
        r = cr(fg, bg)
        if kind == 'text': res = 'AAA' if r >= 7 else 'AA' if r >= 4.5 else 'AA large only' if r >= 3 else 'Fail'
        else: res = 'Pass 3:1' if r >= 3 else 'Below 3:1'
        rows.append(dict(mode=mode, pair=name, fg=fg, bg=bg, ratio=round(r, 2), kind=kind, result=res))
    return rows

TYPE = {
 'display': dict(size=56, line=60, weight=600, tracking='-0.03em'),
 'h1': dict(size=40, line=44, weight=600, tracking='-0.025em'),
 'h2': dict(size=28, line=34, weight=600, tracking='-0.02em'),
 'h3': dict(size=20, line=28, weight=600, tracking='-0.01em'),
 'body': dict(size=15, line=24, weight=400, tracking='0'),
 'body-strong': dict(size=15, line=24, weight=550, tracking='0'),
 'small': dict(size=13, line=20, weight=400, tracking='0'),
 'caption': dict(size=12, line=16, weight=500, tracking='0.01em'),
 'overline': dict(size=11, line=14, weight=500, tracking='0.08em', family='mono', transform='uppercase'),
 'timer': dict(size=15, line=20, weight=500, tracking='0', family='mono', numeric='tabular-nums'),
 'caption-overlay': dict(size=22, line=30, weight=500, tracking='-0.005em'),
}
SPACE = {str(k): f'{v}px' for k, v in dict(zip([0, 0.5, 1, 2, 3, 4, 5, 6, 8, 10, 12, 16], [0, 2, 4, 8, 12, 16, 20, 24, 32, 40, 48, 64])).items()}
RADII = dict(xs='4px', sm='6px', md='10px', lg='14px', xl='20px', pill='999px')
SHADOW_L = dict(sm='0 1px 2px rgba(17,17,16,0.08)', md='0 1px 2px rgba(17,17,16,0.06), 0 8px 24px rgba(17,17,16,0.10)', lg='0 2px 4px rgba(17,17,16,0.06), 0 18px 48px rgba(17,17,16,0.16)')
SHADOW_D = dict(sm='0 1px 2px rgba(0,0,0,0.4)', md='0 1px 2px rgba(0,0,0,0.4), 0 8px 24px rgba(0,0,0,0.45)', lg='0 2px 4px rgba(0,0,0,0.4), 0 18px 48px rgba(0,0,0,0.55)')
MOTION = dict(duration=dict(instant='0ms', blink='120ms', fast='150ms', base='200ms', ripple='400ms', trail='240ms'),
              easing=dict(standard='cubic-bezier(0.2, 0.8, 0.2, 1)', exit='cubic-bezier(0.4, 0, 1, 1)', linear='linear'),
              ripple=dict(scale_from=0.4, scale_to=1.6, opacity_from=0.6, opacity_to=0, rings=1),
              blink=dict(scale_y=0.1, when='record start and record stop only'))
FONTS = dict(sans="'Geist', ui-sans-serif, system-ui, -apple-system, 'Segoe UI', sans-serif",
             mono="'Geist Mono', ui-monospace, 'SF Mono', Menlo, Consolas, monospace",
             wordmark='Geist SemiBold (600), tracking -0.035em, outlined in logo files')

def light_vars():
    return {
     'bg': LIGHT['paper'], 'surface': LIGHT['surface'], 'surface-raised': LIGHT['surface'], 'text': LIGHT['ink'], 'text-muted': LIGHT['graphite'],
     'rule': LIGHT['rule'], 'border-strong': SEM_L['border_strong'], 'signal': LIGHT['signal'], 'signal-text': SIGNAL_TEXT, 'on-signal': LIGHT['ink'],
     'success': SEM_L['success'], 'warning': SEM_L['warning'], 'danger': SEM_L['danger'], 'recording': LIGHT['signal'], 'focus': LIGHT['ink'],
     'inverse': LIGHT['ink'], 'on-inverse': LIGHT['paper'], 'overlay-caption-bg': 'rgba(17,17,16,0.82)', 'overlay-caption-text': '#F4F3EF'}
def dark_vars():
    return {
     'bg': DARK['night'], 'surface': DARK['surface'], 'surface-raised': '#24231F', 'text': DARK['paper'], 'text-muted': DARK['stone'],
     'rule': DARK['rule'], 'border-strong': SEM_D['border_strong'], 'signal': DARK['signal'], 'signal-text': DARK['signal'], 'on-signal': '#111110',
     'success': SEM_D['success'], 'warning': SEM_D['warning'], 'danger': SEM_D['danger'], 'recording': DARK['signal'], 'focus': DARK['paper'],
     'inverse': DARK['paper'], 'on-inverse': DARK['night'], 'overlay-caption-bg': 'rgba(17,17,16,0.82)', 'overlay-caption-text': '#F4F3EF'}

def build():
    T = f'{ROOT}/tokens'
    tokens = {
      '$description': 'Booya design tokens. Direction E, Ghost x Ink mono. Values in px unless stated.',
      'color': {'palette': {'light': LIGHT, 'dark': DARK},
                'semantic': {'light': light_vars(), 'dark': dark_vars()}},
      'font': {'family': FONTS, 'scale': TYPE},
      'space': SPACE, 'radius': RADII, 'shadow': {'light': SHADOW_L, 'dark': SHADOW_D}, 'motion': MOTION,
      'usage': {'signal_max_share_of_screen': '10%', 'recording_state_color': 'signal', 'min_text_contrast': '4.5:1'},
    }
    w(f'{T}/tokens.json', json.dumps(tokens, indent=2))
    def block(vs, sh):
        out = ''.join(f'  --by-{k}: {v};\n' for k, v in vs.items())
        out += ''.join(f'  --by-shadow-{k}: {v};\n' for k, v in sh.items())
        return out
    common = ''.join([
        f"  --by-font-sans: {FONTS['sans']};\n  --by-font-mono: {FONTS['mono']};\n",
        ''.join(f"  --by-text-{k}: {v['size']}px;\n  --by-leading-{k}: {v['line']}px;\n  --by-weight-{k}: {v['weight']};\n  --by-tracking-{k}: {v['tracking']};\n" for k, v in TYPE.items()),
        ''.join(f"  --by-space-{k.replace('.', '_')}: {v};\n" for k, v in SPACE.items()),
        ''.join(f"  --by-radius-{k}: {v};\n" for k, v in RADII.items()),
        ''.join(f"  --by-duration-{k}: {v};\n" for k, v in MOTION['duration'].items()),
        ''.join(f"  --by-ease-{k}: {v};\n" for k, v in MOTION['easing'].items()),
    ])
    css = f"""/* Booya tokens. Light by default, dark via prefers-color-scheme, and [data-theme] wins over both. */
@font-face {{ font-family: 'Geist'; src: url('../fonts/Geist-Variable.ttf') format('truetype'); font-weight: 100 900; font-display: swap; }}
@font-face {{ font-family: 'Geist Mono'; src: url('../fonts/GeistMono-Variable.ttf') format('truetype'); font-weight: 100 900; font-display: swap; }}

:root {{
{common}{block(light_vars(), SHADOW_L)}  color-scheme: light;
}}
@media (prefers-color-scheme: dark) {{
  :root:not([data-theme="light"]) {{
{block(dark_vars(), SHADOW_D).replace('  --', '    --')}    color-scheme: dark;
  }}
}}
[data-theme="light"] {{
{block(light_vars(), SHADOW_L)}  color-scheme: light;
}}
[data-theme="dark"] {{
{block(dark_vars(), SHADOW_D)}  color-scheme: dark;
}}
@media (prefers-reduced-motion: reduce) {{
  :root {{ --by-duration-ripple: 0ms; --by-duration-trail: 0ms; --by-duration-blink: 0ms; --by-duration-fast: 0ms; --by-duration-base: 0ms; }}
}}
:focus-visible {{ outline: 2px solid var(--by-focus); outline-offset: 2px; }}
"""
    w(f'{T}/tokens.css', css)
    keys = list(light_vars())
    tw = "/** Booya Tailwind preset. Colours read the CSS variables in tokens.css, so light/dark switching stays in CSS. */\n"
    tw += "module.exports = {\n  darkMode: ['class', '[data-theme=\"dark\"]'],\n  theme: {\n    extend: {\n      colors: {\n        by: {\n"
    tw += ''.join(f"          '{k}': 'var(--by-{k})',\n" for k in keys)
    tw += "        },\n      },\n"
    tw += "      fontFamily: { sans: ['Geist', 'ui-sans-serif', 'system-ui', 'sans-serif'], mono: ['Geist Mono', 'ui-monospace', 'monospace'] },\n"
    tw += "      fontSize: {\n" + ''.join(f"        '{k}': ['{v['size']}px', {{ lineHeight: '{v['line']}px', letterSpacing: '{v['tracking']}', fontWeight: '{v['weight']}' }}],\n" for k, v in TYPE.items()) + "      },\n"
    tw += "      spacing: {\n" + ''.join(f"        'by-{k.replace('.', '_')}': '{v}',\n" for k, v in SPACE.items()) + "      },\n"
    tw += "      borderRadius: {\n" + ''.join(f"        'by-{k}': '{v}',\n" for k, v in RADII.items()) + "      },\n"
    tw += "      boxShadow: {\n" + ''.join(f"        'by-{k}': 'var(--by-shadow-{k})',\n" for k in SHADOW_L) + "      },\n"
    tw += "      transitionDuration: {\n" + ''.join(f"        'by-{k}': '{v}',\n" for k, v in MOTION['duration'].items()) + "      },\n"
    tw += "      transitionTimingFunction: {\n" + ''.join(f"        'by-{k}': '{v}',\n" for k, v in MOTION['easing'].items()) + "      },\n"
    tw += "      keyframes: {\n        'by-ripple': { '0%': { transform: 'scale(0.4)', opacity: '0.6' }, '100%': { transform: 'scale(1.6)', opacity: '0' } },\n        'by-blink': { '0%, 100%': { transform: 'scaleY(1)' }, '50%': { transform: 'scaleY(0.1)' } },\n      },\n"
    tw += "      animation: {\n        'by-ripple': 'by-ripple 400ms cubic-bezier(0.2, 0.8, 0.2, 1) 1 forwards',\n        'by-blink': 'by-blink 120ms cubic-bezier(0.2, 0.8, 0.2, 1) 1',\n      },\n"
    tw += "    },\n  },\n};\n"
    w(f'{T}/tailwind.preset.js', tw)
    rows = report()
    w(f'{T}/contrast-report.json', json.dumps(rows, indent=2))
    md = '# Booya contrast report (WCAG 2.2)\n\nText needs 4.5:1 (AA) or 3:1 for large text. UI parts and graphics need 3:1.\n\n| Mode | Pair | FG | BG | Ratio | Result |\n|---|---|---|---|---|---|\n'
    md += ''.join(f"| {r['mode']} | {r['pair']} | `{r['fg']}` | `{r['bg']}` | {r['ratio']}:1 | {r['result']} |\n" for r in rows)
    md += f"\nAccessible Signal text variant for light UI: `{SIGNAL_TEXT}` (token `--by-signal-text`). Keep `#FF5A1F` for fills, the record dot and the ripple.\n"
    w(f'{T}/CONTRAST.md', md)
    return rows
if __name__ == '__main__':
    for r in build(): print(f"{r['mode']:5} {r['pair']:40} {r['ratio']:5}  {r['result']}")
    print('signal text', SIGNAL_TEXT)
