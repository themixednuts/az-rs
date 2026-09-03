# Aether palette (captured from htmlswap codegen)

Exact hex shades the `aether-editor.html` / `aether-project-manager.html` designs use,
harvested from the `htmlswap` GPUI codegen (both real `rgb()`/`rgba()` calls **and** the
`// unmapped CSS: background/border: #hex` comments, since the codegen emits most
backgrounds as comments rather than `.bg()` calls).

**Why this exists:** the adopted codegen views were remapped to semantic
`gpui_component::Theme` tokens (decision: pure theme tokens, colors follow the active
theme). These exact values are kept so we can later author an **Aether theme** whose
tokens reproduce the HTML pixel-for-pixel. Counts are usage frequency across both files;
treat them as rough weight, not gospel (pseudo-element chrome is repeated per-node).

Related: [[project_htmlswap_codegen_views]], `theme_color.rs` token list in
`vendor/gpui-component/crates/ui/src/theme/theme_color.rs`.

## Surfaces / backgrounds (darkest → lightest)
| hex | ~uses | role / suggested token |
|------|------|------|
| `#15171b` | 16 | app background — `background` |
| `#16181c` | 47 | base surface / panel — `background` alt |
| `#17191e` `#181b20` `#191b1f` `#191c21` `#1a1c20` `#1a1d22` `#1b1e23` `#1c1f24` `#1c2025` `#1c2026` `#1e2024` `#1e2228` | 1–9 each | nested panel/sidebar surfaces — `sidebar`, `popover`, `secondary` |
| `#202429` | 5 | title bar / toolbar — `title_bar`, `tab_bar` |
| `#20242a` `#22262c` `#23272e` `#232830` `#24272c` `#262b32` | 2–40 | raised rows / inputs / cards — `input`, `list`, `card` |
| `#2a2f37` `#2b3340` `#2f3640` | 5–49 | hover surfaces / active rows — `list_hover`, `accordion_hover` |

## Borders / dividers
| hex | ~uses | role |
|------|------|------|
| `#2c313a` | 82 | primary border / divider — `border` |
| `#363c46` | 16 | stronger border — `border` (emphasis) |
| `#3a4150` | 29 | input/control border — `input` border, `drag_border` |
| `#4a4f57` `#4a525e` `#4b515b` `#4d...` | 1–3 | hovered control border / `ring` |

## Text ramp (muted → bright)
| hex | ~uses | role / token |
|------|------|------|
| `#5a6068` `#5a6470` `#5f6670` | 1–72 | dimmest text / disabled — `muted_foreground` (dim) |
| `#6b7280` | 82 | muted/secondary text — `muted_foreground` |
| `#7c838f` `#7a8aa0` | 8–29 | secondary text — `muted_foreground` (bright) |
| `#8a909a` `#8b919c` | 1–37 | label text — `secondary_foreground` |
| `#9aa1ac` `#9fb3cc` | 1–47 | body text (dim) |
| `#aeb4bd` `#c0c6cf` `#c2c8d0` `#c9ccd2` `#cdd3da` `#cfd4db` | 1–31 | body text — `foreground` (dim) |
| `#d6dae0` | 49 | primary body text — `foreground` |
| `#e4e7ec` `#e8ebef` `#eef0f3` | 2–19 | headings / emphasis — `foreground` (bright) |
| `#ffffff` | 19 | max-contrast text/icon — `foreground` (max) |

## Accent — blue (primary)
| hex | ~uses | role |
|------|------|------|
| `#4188e0` | 24 | **primary accent** — `accent`, `primary`, `link`, `ring` |
| `#3160a8` `#2f6ad0` `#3a6db0` `#436a9e` | 1–16 | accent pressed/darker — `primary_active`, `button_primary_active` |
| `#5b8fd6` `#6f9fde` `#7fb0ec` `#8fb6ee` | 1–3 | accent hover/lighter — `primary_hover`, `link_hover` |
| `#9fb4d0` `#9fc0ef` `#cfdcec` `#cfe0fa` | 1–4 | accent-tinted text on dark — `accent_foreground` |

## Status — amber / gold (warning)
`#9c7a2e` `#c7ac74` `#caa24a` `#d6a23b` (11) `#d8b066` `#e0c06a` `#e2c98a` `#e5c07b`
→ `warning` / `chart` tones. Primary: `#d6a23b`.

## Status — green (success)
`#3a8f4e` `#4d8a4f` `#57b97e` (6) `#67b96a` `#7ad6a6` `#8fc97e`
→ `success` / `chart_bullish`. Primary: `#57b97e`.

## Status — red (danger)
`#a8524d` `#c0453f` (5) `#cf6a64` `#d9645e` `#e0625c` `#e06c75` `#e88a85`
→ `danger` / `chart_bearish`. Primary: `#c0453f` (fill), `#e06c75` (text).

## Accent — purple / teal (categorical)
`#b78fd6` (purple, 3) `#d68fb0` (pink) `#5ac0c0` (teal, 3) — categorical chips / chart series.

## Chrome (selection, scrollbar) — alpha
| hex | role |
|------|------|
| `#4188e059` | text `::selection` (accent @ ~35% a) — `selection` |
| `#454d59` | scrollbar thumb hover — `scrollbar_thumb_hover` |
| `#39404b` | scrollbar thumb (over content) — `scrollbar_thumb` |
| `#33394300` | scrollbar thumb (idle, transparent) — `scrollbar` |

## Overlays / tints (rgba — alpha in last 2 hex digits)
- White hover overlays: `#ffffff06` `#ffffff08` `#ffffff09` `#ffffff0a` `#ffffff0d` — `list_hover`, ghost-button hover.
- Shadow / scrim: `#00000080`, `#00000000` (transparent).
- Translucent panel bg: `#16181cd9` `#16181cd1` (base surface @ ~85/82% a) — popover/menu over content.
- Accent tints: `#4188e066` `#4188e033` `#4188e029` `#4188e024` `#4188e012` `#4188e00f` — selected-row wash, focus ring glow.
- Gold tints: `#d6a23b38` `#d6a23b29` `#d6a23b14`.
- Red tints: `#c0453feb` `#c0453f1f`.
- Purple tints: `#b78fd629` `#b78fd61a`.
- Green tint: `#57b97e21`.

## Fonts
- UI sans: **IBM Plex Sans** → `theme.font_family`
- Mono (code/numeric/grid): **IBM Plex Mono** (97 uses) → `theme.mono_font_family`
- Icons: **Material Symbols Outlined** (287 uses) — icon font; map glyphs to `IconName` or load the icon font.
