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
