# Component Extraction Plan — Aether DCs

Breaking the two monolithic Design Components into reusable, **embeddable** child DCs.
Every child DC: full `data-props` editor contract + `$preview` size; stays **parent-driven**
when a handler prop is passed, **self-manages** standalone. Inline-styles only (DC rules).

## STATUS: 16 of ~23 components shipped. Both parent files WORK and are non-broken.

---

## ✅ DONE & VERIFIED

### Project Manager (`Aether Project Manager.dc.html`) — FULLY COMPONENTIZED
All 9 extracted, parent rewired via `<dc-import>`, recent list + new-project flow verified:
- `Titlebar.dc.html` (props: title, subtitle, accent) — shared with Editor
- `NavItem.dc.html` (label, icon, active, onClick)
- `ProjectRow.dc.html` (project obj, href, latestVersion)
- `Stepper.dc.html` (labels[], current, onStep)
- `TemplateCard.dc.html` (template obj, selected, toggle, onClick)
- `SegmentedOption.dc.html` (label, icon, active, toggle, accent, onClick) — shared
- `Toggle.dc.html` (on, locked, label, sublabel, showLabel, accent, onChange) — shared
- `GemCard.dc.html` (gem obj, onToggle)
- `SummaryRail.dc.html` (name, templateName, icon, iconColor, rows[], gems[], gemCount, createHref, onCancel)

### Editor (`Aether Editor.dc.html`) chrome — EXTRACTED, WIRED & VERIFIED
- `Transport.dc.html` (playState, simulating, onPlay/onStop/onStep/onSimulate)
- `BuildButton.dc.html` (config, target, open, configShort, configs[], targets[], actions[], onToggle/onClose/onConfig/onTarget/onAction)
- `ModeItem.dc.html` (icon, title, active, onClick)

### Wizard — WRITTEN (standalone), NOT YET WIRED
- `Wizard.dc.html` — renders all 4 steps (Identity/Sources/Policies/Review) from a single
  `wiz` prop; has a compact built-in default so it renders standalone. **Parent still uses its
  inline copy** — rewire is the next step.

---

## ⬜ REMAINING — exact steps

### 1. Wire Wizard into Editor parent  ← RESUME HERE
In `Aether Editor.dc.html`, the inline wizard sits inside `<sc-if value="{{ wizOpen }}">`
(~L1986): a backdrop div, then the wizard card
`<div data-htmlswap-component="wizard" ... style="width:880px;...overflow:hidden;"> … </div>`
spanning roughly L1988–L2150.
STEP: `read_file` offset≈1986 limit≈170 to capture the card div's exact open→close text,
then ONE `dc_html_str_replace` swapping that whole `<div data-htmlswap-component="wizard">…</div>`
for:
```
<dc-import name="Wizard" wiz="{{ wiz }}" hint-size="880px,640px"></dc-import>
```
Keep the surrounding `<sc-if>` + backdrop. Parent renderVals already exposes `wiz`
(inline block reads `{{ wiz.steps }}` etc., so it's present). Verify in preview after.

### 2. Settings Modal  → `Modal.dc.html`
Settings-modal shell currently ~L1958–1995. Props: modalIcon, modalTitle, modalSubtitle,
modalCats[], modalRows[] + onClose(closeModal), onReset(resetModal). The "About" modal variant
can stay inline (low reuse). Rewire parent block to `<dc-import name="Modal" ...>`.

### 3. Status Bar  → `StatusBar.dc.html`
Source at `data-htmlswap-component="statusbar"` ~L1813–1860. Takes ~20 scalar props
(branch, errors, warnings, fps, ms, tris, selectedName, etc.) + arrays (rightToggles[], pipeJobs[]).
Extract reading scalar+array props; rewire parent.

### 4. Mode screens (5) → embeddable DCs, each with its own default data
Each is a full-screen `<sc-if>` block reading named groups from the parent's `renderVals()`.
Extract each as `<Name>.dc.html` taking those groups as props (+ standalone defaults), then
replace each `sc-if` body with a `<dc-import>`:
- `MaterialEditor.dc.html` — Materials screen ~L766
- `ScriptEditor.dc.html` — Scripting screen ~L824
- `GameData.dc.html` — GameData screen ~L864 (LARGEST, ~550 lines; has the table grid)
- `Sequencer.dc.html` — Sequencer screen ~L1413 (line nums shift as edits land — re-grep)
- `AnimationEditor.dc.html` — Animation screen ~L1495
NOTE: line numbers drift as you edit; re-`grep` for `data-htmlswap-component` / screen markers
before each extraction rather than trusting absolute lines.

### 5. (Optional) Dock shell
Three dock instances (hierarchy ~L326, console ~L473, inspector ~L634) hold very different
content → low reuse. EITHER extract just a tab-header shell with a slotted body, OR leave inline.
Decided low-priority; do last or skip.

---

## Extraction pattern (proven on all 16 done)
1. Parent `renderVals()` computes a data group (array of row objects / a single model object),
   adding `active`/`selected`/`raw` fields and `onClick` handlers as needed.
2. Child DC reads ONE prop (the group/object), renders it, exposes inline styles computed in
   its own `renderVals()` (never style-holes for static styling).
3. Child is controlled when a handler prop is passed (`controlled = typeof p.onX === 'function'`),
   else seeds internal state from a `default`/value prop.
4. Parent template: replace inline block with `<dc-import name="X" ...props hint-size="W,H">`.
   Always set `hint-size`. Always write explicit close tag.
5. Verify in preview (show_html screenshot) after each wiring.

## Gotchas
- `dc-import` name = file basename, no extension, lowercase tag (never `<Card/>`).
- Pass raw objects to children via a `{{ p.raw }}`-style field, not the display-massaged copy.
- After extracting an interactive leaf, make parent the source of truth (controlled-when-driven).
- Material Symbols + IBM Plex fonts must be re-declared in each child DC's `<helmet>`.
