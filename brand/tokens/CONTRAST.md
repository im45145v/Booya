# Booya contrast report (WCAG 2.2)

Text needs 4.5:1 (AA) or 3:1 for large text. UI parts and graphics need 3:1.

| Mode | Pair | FG | BG | Ratio | Result |
|---|---|---|---|---|---|
| light | Ink on Paper | `#111110` | `#F4F3EF` | 17.02:1 | AAA |
| light | Ink on Surface | `#111110` | `#FFFFFF` | 18.89:1 | AAA |
| light | Graphite on Paper | `#6B6963` | `#F4F3EF` | 4.94:1 | AA |
| light | Graphite on Surface | `#6B6963` | `#FFFFFF` | 5.49:1 | AA |
| light | Signal on Paper | `#FF5A1F` | `#F4F3EF` | 2.81:1 | Fail |
| light | Signal on Surface | `#FF5A1F` | `#FFFFFF` | 3.12:1 | AA large only |
| light | Signal text on Paper | `#BF4317` | `#F4F3EF` | 4.69:1 | AA |
| light | Signal text on Surface | `#BF4317` | `#FFFFFF` | 5.21:1 | AA |
| light | Ink on Signal (button label) | `#111110` | `#FF5A1F` | 6.06:1 | AA |
| light | White on Signal (button label) | `#FFFFFF` | `#FF5A1F` | 3.12:1 | AA large only |
| light | Signal on Paper (record dot, non-text) | `#FF5A1F` | `#F4F3EF` | 2.81:1 | Below 3:1 |
| light | Signal on Surface (record dot in toolbar) | `#FF5A1F` | `#FFFFFF` | 3.12:1 | Pass 3:1 |
| light | Rule on Paper (divider) | `#DCDAD3` | `#F4F3EF` | 1.26:1 | Below 3:1 |
| light | Border strong on Surface (inputs) | `#8C8A83` | `#FFFFFF` | 3.45:1 | Pass 3:1 |
| light | Success on Surface | `#1D7A45` | `#FFFFFF` | 5.35:1 | AA |
| light | Warning on Surface | `#975A00` | `#FFFFFF` | 5.56:1 | AA |
| light | Danger on Surface | `#C2301F` | `#FFFFFF` | 5.62:1 | AA |
| dark | Paper on Night | `#EDEBE5` | `#111110` | 15.85:1 | AAA |
| dark | Paper on Surface | `#EDEBE5` | `#1C1B19` | 14.44:1 | AAA |
| dark | Stone on Night | `#8D8A82` | `#111110` | 5.48:1 | AA |
| dark | Stone on Surface | `#8D8A82` | `#1C1B19` | 4.99:1 | AA |
| dark | Signal on Night | `#FF6A33` | `#111110` | 6.62:1 | AA |
| dark | Signal on Surface | `#FF6A33` | `#1C1B19` | 6.03:1 | AA |
| dark | Night on Signal (button label) | `#111110` | `#FF6A33` | 6.62:1 | AA |
| dark | Rule on Night (divider) | `#2C2B28` | `#111110` | 1.33:1 | Below 3:1 |
| dark | Border strong on Surface (inputs) | `#76736C` | `#1C1B19` | 3.64:1 | Pass 3:1 |
| dark | Success on Surface | `#4CC385` | `#1C1B19` | 7.75:1 | AAA |
| dark | Warning on Surface | `#F0B24E` | `#1C1B19` | 9.17:1 | AAA |
| dark | Danger on Surface | `#FF6B5B` | `#1C1B19` | 6.15:1 | AA |

Accessible Signal text variant for light UI: `#BF4317` (token `--by-signal-text`). Keep `#FF5A1F` for fills, the record dot and the ripple.

## App UI layer (`--by-ui-*`, Booya 0.9)

Computed with the WCAG 2.2 relative luminance formula. Text needs 4.5:1, large text and control boundaries 3:1. Disabled text is exempt and sits low on purpose. Where `text-3` fails on hover and pressed fills, the app steps the label up to `text` or `text-2` on hover.

| Mode | Pair | FG | BG | Ratio | Result |
|---|---|---|---|---|---|
| dark | text-primary on bg | `#EDEBE5` | `#111110` | 15.85:1 | AAA |
| dark | text-secondary on bg | `#B3B0A8` | `#111110` | 8.72:1 | AAA |
| dark | text-tertiary on bg | `#8D8A82` | `#111110` | 5.48:1 | AA |
| dark | text-disabled on bg | `#5C5A55` | `#111110` | 2.74:1 | n/a (disabled, exempt) |
| dark | text-primary on panel | `#EDEBE5` | `#191917` | 14.77:1 | AAA |
| dark | text-secondary on panel | `#B3B0A8` | `#191917` | 8.13:1 | AAA |
| dark | text-tertiary on panel | `#8D8A82` | `#191917` | 5.11:1 | AA |
| dark | text-disabled on panel | `#5C5A55` | `#191917` | 2.56:1 | n/a (disabled, exempt) |
| dark | text-primary on raised | `#EDEBE5` | `#21201E` | 13.66:1 | AAA |
| dark | text-secondary on raised | `#B3B0A8` | `#21201E` | 7.51:1 | AAA |
| dark | text-tertiary on raised | `#8D8A82` | `#21201E` | 4.72:1 | AA |
| dark | text-disabled on raised | `#5C5A55` | `#21201E` | 2.36:1 | n/a (disabled, exempt) |
| dark | text-primary on hover | `#EDEBE5` | `#2A2926` | 12.20:1 | AAA |
| dark | text-secondary on hover | `#B3B0A8` | `#2A2926` | 6.71:1 | AA |
| dark | text-tertiary on hover | `#8D8A82` | `#2A2926` | 4.22:1 | FAIL |
| dark | text-disabled on hover | `#5C5A55` | `#2A2926` | 2.11:1 | n/a (disabled, exempt) |
| dark | text-primary on pressed | `#EDEBE5` | `#33322E` | 10.76:1 | AAA |
| dark | text-secondary on pressed | `#B3B0A8` | `#33322E` | 5.92:1 | AA |
| dark | text-tertiary on pressed | `#8D8A82` | `#33322E` | 3.72:1 | FAIL |
| dark | text-disabled on pressed | `#5C5A55` | `#33322E` | 1.86:1 | n/a (disabled, exempt) |
| light | text-primary on bg | `#111110` | `#F4F3EF` | 17.02:1 | AAA |
| light | text-secondary on bg | `#47453F` | `#F4F3EF` | 8.63:1 | AAA |
| light | text-tertiary on bg | `#6B6963` | `#F4F3EF` | 4.94:1 | AA |
| light | text-disabled on bg | `#A9A69F` | `#F4F3EF` | 2.19:1 | n/a (disabled, exempt) |
| light | text-primary on panel | `#111110` | `#FAF9F7` | 17.96:1 | AAA |
| light | text-secondary on panel | `#47453F` | `#FAF9F7` | 9.11:1 | AAA |
| light | text-tertiary on panel | `#6B6963` | `#FAF9F7` | 5.22:1 | AA |
| light | text-disabled on panel | `#A9A69F` | `#FAF9F7` | 2.31:1 | n/a (disabled, exempt) |
| light | text-primary on raised | `#111110` | `#FFFFFF` | 18.89:1 | AAA |
| light | text-secondary on raised | `#47453F` | `#FFFFFF` | 9.58:1 | AAA |
| light | text-tertiary on raised | `#6B6963` | `#FFFFFF` | 5.49:1 | AA |
| light | text-disabled on raised | `#A9A69F` | `#FFFFFF` | 2.43:1 | n/a (disabled, exempt) |
| light | text-primary on hover | `#111110` | `#EDEBE6` | 15.86:1 | AAA |
| light | text-secondary on hover | `#47453F` | `#EDEBE6` | 8.05:1 | AAA |
| light | text-tertiary on hover | `#6B6963` | `#EDEBE6` | 4.61:1 | AA |
| light | text-disabled on hover | `#A9A69F` | `#EDEBE6` | 2.04:1 | n/a (disabled, exempt) |
| light | text-primary on pressed | `#111110` | `#E4E2DC` | 14.58:1 | AAA |
| light | text-secondary on pressed | `#47453F` | `#E4E2DC` | 7.40:1 | AAA |
| light | text-tertiary on pressed | `#6B6963` | `#E4E2DC` | 4.24:1 | FAIL |
| light | text-disabled on pressed | `#A9A69F` | `#E4E2DC` | 1.88:1 | n/a (disabled, exempt) |
| dark | Ink on Signal (Record label) | `#111110` | `#FF6A33` | 6.62:1 | AA |
| dark | Ink on Signal hover | `#111110` | `#FF8150` | 7.65:1 | AAA |
| dark | Ink on Signal active | `#111110` | `#EE5A24` | 5.51:1 | AA |
| dark | Signal on bg (dot, timer text) | `#FF6A33` | `#111110` | 6.62:1 | AA |
| dark | Signal on panel (toolbar timer) | `#FF6A33` | `#191917` | 6.17:1 | AA |
| dark | Signal on raised | `#FF6A33` | `#21201E` | 5.71:1 | AA |
| dark | Signal fill vs panel (button edge) | `#FF6A33` | `#191917` | 6.17:1 | Pass 3:1 |
| light | Ink on Signal (Record label) | `#111110` | `#FF5A1F` | 6.06:1 | AA |
| light | Ink on Signal hover | `#111110` | `#FF7440` | 7.04:1 | AAA |
| light | Ink on Signal active | `#111110` | `#E84E15` | 4.98:1 | AA |
| light | signal-text on bg | `#BF4317` | `#F4F3EF` | 4.69:1 | AA |
| light | signal-text on panel | `#BF4317` | `#FAF9F7` | 4.95:1 | AA |
| light | signal-text on raised | `#BF4317` | `#FFFFFF` | 5.21:1 | AA |
| light | Signal fill vs panel (button edge) | `#FF5A1F` | `#FAF9F7` | 2.96:1 | FAIL |
| light | Signal dot on raised (record dot) | `#FF5A1F` | `#FFFFFF` | 3.12:1 | Pass 3:1 |
| dark | Ink on Paper (primary button) | `#111110` | `#EDEBE5` | 15.85:1 | AAA |
| dark | Ink on primary hover #D9D6CF | `#111110` | `#D9D6CF` | 13.02:1 | AAA |
| dark | Ink on primary active #C6C3BB | `#111110` | `#C6C3BB` | 10.73:1 | AAA |
| dark | Paper button vs panel (edge) | `#EDEBE5` | `#191917` | 14.77:1 | Pass 3:1 |
| light | Paper on Ink (primary button) | `#F4F3EF` | `#111110` | 17.02:1 | AAA |
| light | Paper on primary hover #2A2926 | `#F4F3EF` | `#2A2926` | 13.10:1 | AAA |
| light | Paper on primary active #3A3935 | `#F4F3EF` | `#3A3935` | 10.41:1 | AAA |
| dark | border-input on panel | `#76736C` | `#191917` | 3.72:1 | Pass 3:1 |
| dark | border-input on raised | `#76736C` | `#21201E` | 3.44:1 | Pass 3:1 |
| dark | border #353430 on panel | `#353430` | `#191917` | 1.41:1 | n/a (decorative) |
| dark | border-subtle #262523 on panel | `#262523` | `#191917` | 1.15:1 | n/a (decorative) |
| dark | focus ring Paper on bg | `#EDEBE5` | `#111110` | 15.85:1 | Pass 3:1 |
| dark | focus ring Paper on Signal fill | `#EDEBE5` | `#FF6A33` | 2.39:1 | n/a (decorative) |
| dark | focus ring Paper vs panel (offset gap) | `#EDEBE5` | `#191917` | 14.77:1 | Pass 3:1 |
| light | border-input on raised | `#8C8A83` | `#FFFFFF` | 3.45:1 | Pass 3:1 |
| light | border-input on panel | `#8C8A83` | `#FAF9F7` | 3.28:1 | Pass 3:1 |
| light | focus ring Ink on bg | `#111110` | `#F4F3EF` | 17.02:1 | Pass 3:1 |
| light | focus ring Ink on panel | `#111110` | `#FAF9F7` | 17.96:1 | Pass 3:1 |
| dark | success on panel | `#4CC385` | `#191917` | 7.92:1 | AAA |
| dark | warning on panel | `#F0B24E` | `#191917` | 9.38:1 | AAA |
| dark | danger on panel | `#FF6B5B` | `#191917` | 6.29:1 | AA |
| dark | danger on raised | `#FF6B5B` | `#21201E` | 5.82:1 | AA |
| light | success on panel | `#1D7A45` | `#FAF9F7` | 5.09:1 | AA |
| light | warning on panel | `#975A00` | `#FAF9F7` | 5.28:1 | AA |
| light | danger on panel | `#C2301F` | `#FAF9F7` | 5.34:1 | AA |
| dark | checked box Paper vs raised | `#EDEBE5` | `#21201E` | 13.66:1 | Pass 3:1 |
| dark | progress fill Paper vs track #33322E | `#EDEBE5` | `#33322E` | 10.76:1 | Pass 3:1 |
| light | progress fill Ink vs track #E4E2DC | `#111110` | `#E4E2DC` | 14.58:1 | Pass 3:1 |
| dark | kbd text-secondary on raised | `#B3B0A8` | `#21201E` | 7.51:1 | AAA |
| dark | text-3 on hover (why text steps up on hover) | `#8D8A82` | `#2A2926` | 4.22:1 | FAIL |
| dark | Old 0.8.0: Ink logo body on stage (sampled #2C2B28) | `#111110` | `#2C2B28` | 1.33:1 | FAIL (ghost invisible) |
| dark | Old 0.8.0: disabled Run auto edit fill (sampled from screenshot) vs panel | `#787776` | `#1C1B19` | 3.85:1 | a full-width 32 px block at this contrast is the largest light area in the panel, so a disabled control reads as the main action |
| light | Light Record fill vs bg #F4F3EF | `#FF5A1F` | `#F4F3EF` | 2.81:1 | FAIL |
| dark | Step current ring text on panel | `#EDEBE5` | `#191917` | 14.77:1 | Pass 3:1 |
| dark | Step next ring (border-input) on panel | `#76736C` | `#191917` | 3.72:1 | Pass 3:1 |
| dark | Signal playhead on timeline panel | `#FF6A33` | `#191917` | 6.17:1 | Pass 3:1 |
