# Project conventions

## `data-htmlswap-*` semantic markers

When generating DC HTML, add these `data-*` markers **only when they describe durable
UI intent** — never for implementation details.

### Markers
- `data-htmlswap-region` — dotted area/feature name, e.g. `"project.manager"`, `"editor.sidebar"`
- `data-htmlswap-component` — reusable UI concept, e.g. `"titlebar"`, `"project-card"`, `"toolbar"`
- `data-htmlswap-slot` — named child area, e.g. `"actions"`, `"window-controls"`, `"footer"`
- `data-htmlswap-prop-*` — values passed into a component-like subtree
- `data-htmlswap-key` — stable identity for repeated items
- `data-htmlswap-state-owner` — `"source"` when the value is owned by the logic/state
- `data-htmlswap-tone` — `neutral | accent | success | warning | danger | info`
- `data-htmlswap-variant` — `solid | outline | ghost | soft | link`
- `data-htmlswap-size` — `xs | sm | md | lg | xl`
- `data-htmlswap-density` — `compact | comfortable | spacious`

### Do NOT add markers for implementation details
- no file paths
- no module names
- no framework names
- no library names
- no output names
- no made-up build/config values

Bad examples:
- `data-htmlswap-region="src/project_manager.rs"`
- `data-htmlswap-component="RustProjectCard"`
- `data-htmlswap-slot="gpui-actions"`
- `data-htmlswap-prop-module="./components/card"`

### Good examples

```html
<section data-htmlswap-region="project.manager">
  ...
</section>

<header data-htmlswap-component="titlebar" data-htmlswap-region="app.window">
  <div class="brand">
    <span class="logo"></span>
    <strong>Aether</strong>
    <span>Project Manager</span>
  </div>
  <div data-htmlswap-slot="window-controls">
    <button aria-label="Minimize">remove</button>
    <button aria-label="Maximize">crop_square</button>
    <button aria-label="Close">close</button>
  </div>
</header>

<button
  data-htmlswap-tone="accent"
  data-htmlswap-variant="solid"
  data-htmlswap-size="md"
  data-htmlswap-density="comfortable"
  onClick="{{ createProject }}"
>
  New Project
</button>

<sc-for list="{{ projects }}" as="project" hint-placeholder-count="4">
  <article
    data-htmlswap-component="project-card"
    data-htmlswap-region="project.manager.recents"
    data-htmlswap-key="{{ project.id }}"
    data-htmlswap-prop-project="{{ project }}"
  >
    <h3>{{ project.name }}</h3>
    <button
      data-htmlswap-tone="accent"
      data-htmlswap-variant="ghost"
      data-htmlswap-size="sm"
      onClick="{{ project.open }}"
    >
      Open
    </button>
  </article>
</sc-for>

<form data-htmlswap-region="project.manager.create">
  <input
    name="name"
    value="{{ draft.name }}"
    placeholder="Project name"
    data-htmlswap-state-owner="source"
    onInput="{{ updateName }}"
  >
  <button
    type="submit"
    data-htmlswap-tone="success"
    data-htmlswap-variant="solid"
    data-htmlswap-size="md"
    onClick="{{ submitProject }}"
  >
    Create
  </button>
</form>
```

### Note on DC interplay
- `data-htmlswap-prop-*` and `data-htmlswap-key` values may use `{{ path }}` holes;
  these follow normal DC attribute-coercion rules (a whole-value `{{ }}` passes the
  raw value). `data-*` attributes are emitted verbatim (not camel-cased).
- These markers are annotations only — they never replace real DC behavior
  (`sc-for`, `onClick` handlers, `renderVals()` bindings still do the actual work).

---

# htmlswap `.dc.html` compiler grammar

## Core prompt (always include)

# You are designing UI as htmlswap `.dc.html` source

You write declarative HTML views for htmlswap, a compiler that turns them
into code for one or more UI targets (native or web). Your HTML is compiled,
not shipped to a browser as-is. Follow this grammar exactly; anything outside
it is rejected or emitted lossily with warnings.

## File shape

```html
<x-dc>
  <helmet>
    <!-- optional: <style>, <link rel="stylesheet">, external <script src> -->
  </helmet>
  <section> <!-- exactly one render root -->
    ...
  </section>
</x-dc>
```

## Data holes — `{{ ... }}`

Holes are bindings, NOT expressions. Allowed forms only:
- dotted paths: `{{ user.name }}`, `{{ item.onSelect }}`
- loop index: `{{ $index }}`
- literals: `{{ true }}`, `{{ false }}`, `{{ null }}`, `{{ 42 }}`

NEVER write calls, arithmetic, ternaries, or comparisons inside holes
(`{{ save() }}`, `{{ a + b }}`, `{{ x ? y : z }}` are all invalid). If logic
is needed, compute it in the component script and bind the result.

Attribute value shapes:
- `x="literal"` → string
- `x="{{ path }}"` → raw binding
- `x="Hello {{ name }}"` → interpolated string

## Control flow (elements, not attributes)

```html
<sc-if value="{{ visible }}"> ... </sc-if>
<sc-else-if value="{{ other }}"> ... </sc-else-if>
<sc-else> ... </sc-else>

<sc-for list="{{ items }}" as="item" hint-placeholder-count="3">
  <div>{{ item.label }} — {{ $index }}</div>
</sc-for>

<sc-switch value="{{ mode }}">
  <sc-case value="edit"> ... </sc-case>
  <sc-default> ... </sc-default>
</sc-switch>
```

`hint-placeholder-*` attributes give static preview hints (e.g. how many
placeholder rows to render); use them on every `sc-for`/`sc-if` whose data is
runtime-only.

## Events

Bind handlers with `on*` attributes whose value is a single hole:
`onClick`, `onDoubleClick`, `onInput`, `onChange`, `onSubmit`,
`onMouseEnter`, `onMouseDown`.

```html
<button onClick="{{ handleSave }}">Save</button>
<input value="{{ name }}" onInput="{{ onName }}">
```

## State

- Form controls (`input`, `textarea`, `select`, checkboxes, radios) own their
  state by default (the target framework manages it).
- Add `data-htmlswap-state-owner="source"` when your script owns the value
  and the control should reflect it.
- Use native attributes for control state: `disabled`, `readonly`,
  `required`, `checked`, `multiple`, `placeholder`, `value`, `min`, `max`,
  `step`, `pattern`, `minlength`, `maxlength`, `rows`.

## Semantic intent markers (the only `data-htmlswap-*` you may use)

| Attribute | Allowed values | Meaning |
|---|---|---|
| `data-htmlswap-tone` | `neutral` `accent` `success` `warning` `danger` `info` | intent color |
| `data-htmlswap-variant` | `solid` `outline` `ghost` `soft` `link` `text` | visual treatment |
| `data-htmlswap-size` | `xs` `sm` `md` `lg` `xl` | semantic size |
| `data-htmlswap-density` | `compact` `comfortable` `spacious` | spacing density |
| `data-htmlswap-region` | dotted namespace, e.g. `editor.sidebar` | feature grouping, inherited |
| `data-htmlswap-component` | kebab-case name, e.g. `gem-card` | component identity |
| `data-htmlswap-slot` | slot name, e.g. `header-actions` | slot the parent consumes |
| `data-htmlswap-prop-*` | any | props for a component usage |
| `data-htmlswap-key` | binding | list reconciliation key |
| `data-htmlswap-state` / `-state-owner` | id / `source` `external` `target` | state identity/ownership |
| `data-htmlswap-loading` | boolean attribute | busy state (or use `aria-busy="true"`) |
| `data-htmlswap-semantic-*` | any | escape hatch for intent not listed above |
| `data-htmlswap-raw` | boolean attribute | preserve subtree as opaque HTML |

Do NOT invent other `data-htmlswap-*` attributes. Use
`data-htmlswap-semantic-<axis>` for anything not covered.

Interactive state uses ARIA, not custom markers:
- toggle button pressed → `aria-pressed="true"` (or bare `selected`)
- selected tab → `aria-selected="true"` on the `role="tab"` element
- loading control → `aria-busy="true"`
- tooltips → the native `title` attribute

## Structure conveys widgets

Adapters recognize widgets from standard HTML tags and ARIA roles, so use
them precisely: `button`, `a`, `label`, `input`/`textarea`/`select` with
correct `type`, `form`/`fieldset`/`legend`, list markup for lists, and
`role="tablist"`/`role="tab"` for tabs. Icons are written as Material Symbol
spans with the icon name as text:
`<span class="material-symbols-outlined">save</span>`.

## Styling

- Use `style="..."` and `<style>` blocks with a flexbox-only layout model:
  `display:flex`, `flex-direction`, `flex-wrap`, `flex`, `gap`, `padding`,
  `margin`, `width/height/min-*/max-*`, `border*`, `border-radius`,
  `background`, `color`, `font-size`, `font-weight`, `opacity`, `overflow`.
- Do NOT use: `position: absolute/fixed/sticky`, `z-index`, `float`,
  `display: inline/inline-block/grid`, CSS animations. Targets that cannot
  express them preserve them as source comments instead of rendering them.
- Theme through CSS custom properties in `:root` (`--background`,
  `--foreground`, `--accent`, …) and reference them with `var(--token)`.
- Pseudo states via attributes: `style-hover="..."`, `style-active`,
  `style-focus`, `style-before`, `style-after`.
- Dynamic styles: `style="{{ computedStyle }}"` binds a style object;
  `data-htmlswap-style-hover="{{ hoverStyle }}"` binds per-state.

## Components

```html
<!-- source-defined component usage -->
<dc-import name="Card" item="{{ item }}" hint-size="100%,120px">
  <span>projected child</span>
</dc-import>

<!-- foreign component (body not compiled here) -->
<x-import component="Chart" from="./Chart.jsx" data="{{ rows }}"></x-import>
```

Kebab-case prop attributes become camelCase props (`item-name` →
`itemName`); `data-*`/`aria-*` props stay kebab-case. `hint-size` is a
placeholder render size ("width,height"), unrelated to
`data-htmlswap-size`.

## Never do

- No JavaScript expressions in holes — bindings only.
- No target names, adapter names, module paths, file paths, or output
  locations anywhere in source (`data-htmlswap-region` is a dotted
  namespace like `project.manager`, never a path).
- No absolute positioning or overlay hacks; express layout as nested flex.
- No inventing tags or attributes outside this document.
- Do not silently work around a limitation — if the design needs something
  this grammar cannot express, say so explicitly and propose the closest
  expressible alternative.

## Self-check before you finish

1. Every `{{ }}` is a dotted path, `$index`, or literal.
2. Every interactive element has an `id` and an `on*` handler bound to a hole.
3. Every `sc-for` has `list`, `as`, and a `hint-placeholder-count`.
4. Semantic markers only use the allowed values in the table above.
5. Layout is pure flexbox; theme colors come from `var(--token)`.
6. If a target addendum follows, honor its lossy-value warnings.

## Target addenda (append only the ones you compile to)

**gpui-components** (widget-level native desktop output):

# Target addendum: gpui-components

This design compiles to the gpui-component widget library. Design within its
grammar:

- Buttons: tones `accent/success/warning/danger/info` and variants
  `outline/ghost/link/text` map 1:1 to real widget methods, as do `disabled`,
  `aria-pressed` (selected), and `aria-busy` (loading). `variant="soft"`,
  `density="spacious"`, and `size="xl"` are LOSSY on this target (warned;
  `xl` clamps to large) — avoid them unless another target needs them.
- Tabs: mark the container `role="tablist"` (or
  `data-htmlswap-component="tabs"`) and children `role="tab"`. Tab-bar
  variants are `outline`, `pill`, `segmented`, `underline`.
- Title bars: `data-htmlswap-component="titlebar"` on a header element; a
  child with `data-htmlswap-slot="window-controls"` is replaced by native
  window controls.
- Text inputs, selects, checkboxes, radios, links, labels, list items, forms
  and fieldsets map to native widgets automatically from tags and roles.
- Material Symbol spans render as native icons.
