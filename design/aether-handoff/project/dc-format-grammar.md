# Design Component (.dc.html) — Authoring Grammar Reference

A transpiler-oriented spec of the **authoring surface** (what appears in `.dc.html`
files). This is the stable subset you parse; the runtime (`support.js`) is out of scope.

The format is React-under-the-hood, but every construct below lowers cleanly to any
reactive UI lib (Vue, Svelte, Solid, plain React/JSX, etc.).

---

## 1. File shape

```
<!doctype html> … <body>
  <x-dc>
    <helmet> … </helmet>          ← optional; HEAD side-effects (see §7)
    …render tree…                 ← everything after </helmet>
  </x-dc>
  <script data-dc-script data-props='…JSON…'>
    class Component extends DCLogic { … }   ← logic class (see §8)
  </script>
</body>
```

- `<x-dc>` is the root container. One per file.
- A transpiler only authors/reads the inner template + the logic class + the
  `data-props` JSON. The document scaffolding and `support.js` include are generated.

---

## 2. Holes — `{{ … }}`

Holes are **dotted-path lookups or literals ONLY**. No expressions, ever.

```
hole     := '{{' WS pathOrLiteral WS '}}'
pathOrLiteral := path | literal
path     := IDENT ( '.' IDENT )*          // user.name, m.btnStyle, it.icon
literal  := 'true' | 'false' | 'null' | NUMBER | $-ident
$-ident  := '$index'                      // loop index; '$' prefixed builtins
```

- `{{ a + b }}`, `{{ !x }}`, `{{ fn() }}`, `{{ a ? b : c }}` are **illegal** — they
  render nothing and warn. All real logic lives in `renderVals()` (§8).
- Resolution scope = object returned by `renderVals()`, plus any loop bindings (§5)
  in scope. Loop/`renderVals` keys override props of the same name.
- An unresolved path renders empty (with a console warning).

**IR node:** `Hole { path: string[] | Literal }`.

---

## 3. Attributes — three coercion modes

| Source form              | Result passed to element                        | IR kind        |
|--------------------------|--------------------------------------------------|----------------|
| `x="literal text"`       | string `"literal text"`                          | `StaticAttr`   |
| `x="{{ path }}"`         | **raw** value — number / fn / ref / object / arr | `RawAttr`      |
| `x="a {{ p }} b"`        | interpolated **string** `"a "+p+" b"`            | `InterpAttr`   |

- The whole-value form (`RawAttr`) is the only way functions and refs reach an
  element. Your attribute type system needs a "raw passthrough" kind — not just
  string/number.
- Name remapping: `class` → `className`, `for` → `htmlFor`.
- Event handlers are camelCase whole-value holes: `onClick="{{ handler }}"`,
  `onMouseEnter="{{ h }}"`. Always `RawAttr` (the value is a function).
- `style` is special — see §6.

---

## 4. Elements & text

Standard HTML elements. Children are a mix of elements, text, holes, and the
control-flow / component tags below. Canonical HTML: every non-void element is
explicitly closed; attribute values are double-quoted; non-void elements are not
self-closed.

**IR node:** `Element { tag, attrs: Attr[], children: Node[] }`,
`Text { value: string }`.

---

## 5. Control flow

### `sc-for`
```
<sc-for list="{{ items }}" as="item" hint-placeholder-count="3">
  …children… ({{ item.* }} and {{ $index }} in scope)
</sc-for>
```
- `list` — RawAttr, the array to iterate.
- `as` — StaticAttr, the per-item binding name introduced into child scope.
- `$index` — implicit numeric binding in child scope.
- `hint-placeholder-count` — int; how many skeleton copies to render before data
  arrives (streaming hint). **Ignorable** for a static target.

**IR node:** `For { list: Hole, itemName: string, body: Node[] }`.

### `sc-if`
```
<sc-if value="{{ cond }}" hint-placeholder-val="{{ true }}"> …children… </sc-if>
```
- `value` — RawAttr, truthiness tested.
- `hint-placeholder-val` — literal hole; the value to assume while streaming.
  **Ignorable** for a static target.
- **No `sc-else`.** Mutually-exclusive branches are emitted as two complementary
  `sc-if`s (e.g. a separator row guarded by `it.sep` and the normal row guarded by
  `it.notsep`). Your conditional lowering should recognize/optionally fuse these.

**IR node:** `If { cond: Hole, body: Node[] }`.

---

## 6. Styling

Inline only. No stylesheets / CSS classes in the template (except what physically
must live in `<helmet>` — `@font-face`, `@keyframes`, body resets).

- `style="prop:val; prop:val"` — a CSS string compiled to a React style object.
  May contain mixed holes for genuinely live values: `color:{{ activeColor }}`.
  Whole-value style holes also occur: `style="{{ m.btnStyle }}"` (RawAttr → object
  or string assembled in `renderVals()`).
- **Pseudo-state sibling attributes** (the main lowering work for non-React targets):
  ```
  style-hover, style-active, style-focus, style-before, style-after
  ```
  Each is a CSS string for that pseudo-state. Most frameworks can't take these
  inline — hoist them into generated scoped rules (`.gen-xxx:hover { … }`,
  `::before { content … }`, etc.).

**IR nodes:** `StyleAttr { decls }`, `PseudoStyleAttr { state: 'hover'|'active'|'focus'|'before'|'after', decls }`.

---

## 7. `<helmet>`

Goes at the top of the template. Legal contents:
- `<link>` (fonts, preconnect)
- `<style>` — only `@font-face`, `@keyframes`, body/global resets
- `<script>` (incl. `<script src>`) — mounts when `</helmet>` closes

Map to your target's document head / global setup. Scripts here are the only legal
`<script>` tags inside the template body.

**IR node:** `Helmet { links: [], styles: [], scripts: [] }`.

---

## 8. Component instantiation

### Sibling Design Component
```
<dc-import name="Card" item="{{ it }}" hint-size="100%,120px"> …children… </dc-import>
```
- `name` — StaticAttr, sibling file basename (`Card` → `Card.dc.html`). Never a
  capitalized JSX tag.
- Other attrs → props (kebab-case → camelCase). RawAttr/InterpAttr/StaticAttr rules
  apply per §3.
- Children → `props.children`.
- `hint-size` — `"width,height"` placeholder + min-size while streaming.

### External React / JS / web component
```
<x-import component="Chart" from="./Chart.jsx" data="{{ rows }}" hint-size="100%,320px"> … </x-import>
<x-import component-from-global-scope="deck-stage" from="./deck-stage.js" …> … </x-import>
<x-import component-from-global-scope="NS.Button" …> … </x-import>
```
- `component` — a named export / `window.X` React component.
- `component-from-global-scope` — either a **custom-element tag name**
  (`customElements.define`) or a **window global** (may be dotted: `NS.Button`).
- `from` — **literal URL only** (fetch starts at parse time; a `{{ }}` here never
  loads). The component-name attrs DO accept holes and re-resolve per render.
- `dc-props="{{ obj }}"` — spreads an object of extra props.
- Other attrs → props (kebab→camel; `aria-*`/`data-*` verbatim). Children →
  `props.children`. `hint-size` as above.

**IR node:** `Component { source: 'dc'|'module'|'global', name, from?, props: Attr[], children: Node[], hintSize }`.

---

## 9. Logic class (`<script data-dc-script>`)

```js
class Component extends DCLogic {
  state = { n: 0 };
  componentDidMount() { … }              // + full React-class lifecycle
  renderVals() {
    return {
      n: this.state.n,
      inc: () => this.setState(s => ({ n: s.n + 1 })),
      // …one key per name any template hole references…
    };
  }
}
```

- Plain classic JS. No `import`/`export`, no TS. `DCLogic` and `React` are injected.
- Class **must** be named `Component`.
- React-class semantics: `this.props`, `this.state`, `this.setState`,
  `this.forceUpdate`, lifecycle methods. No `render()` — `renderVals()` replaces it.
- **`renderVals()` is the binding contract.** Its returned object is the scope every
  template hole resolves against. Values can be data, arrays, functions (handlers),
  refs, or `React.createElement(...)` subtrees (used for the rare animated piece that
  must survive re-render — opaque to the editor, avoid for normal layout).

**Lowering targets:**
- Vue: `renderVals()` → `setup()` returning the same object (or `data()` +
  `computed()` + `methods()`); `state` → `ref`/`reactive`; lifecycle → `onMounted` etc.
- Svelte: returned keys → top-level reactive `let`s / `$:` derivations; handlers as
  functions.
- React/JSX: nearly 1:1 — wrap `renderVals()` as a hook returning the scope object.

---

## 10. `data-props` JSON (editor metadata — additive)

On the `<script data-dc-script data-props='…'>` tag. Not required to render; useful
for IR completeness and round-tripping editor affordances.

```json
{
  "$preview": { "width": 1280, "height": 720 },
  "title":  { "editor": "text",  "default": "Hello", "tsType": "string" },
  "accent": { "editor": "color", "default": "#4188e0" },
  "count":  { "editor": "int",   "default": 3, "min": 0, "max": 10, "step": 1 },
  "variant":{ "editor": "enum",  "default": "a", "options": ["a","b"] },
  "onPick": { "editor": null,    "tsType": "(id:string)=>void" }
}
```

- `editor` ∈ `text | color | int | float | range | boolean | enum | null`
  (`null` = non-editable: callbacks, ReactNode, objects).
- `$preview.{width,height}` — preferred preview size for sized fragments; omit for
  full pages.
- Optional per-prop: `options` (enum), `min`/`max`/`step`/`unit` (number/range),
  `section` (grouping heading).
- `default` seeds the editor, **not** the runtime — the component still falls back
  via `this.props.x ?? …` in `renderVals()`.

---

## 11. Parser checklist (gotchas)

1. **Holes are dotted-path-only** — no expression parser needed in templates. All
   logic is already in `renderVals()`.
2. **Three attribute coercion modes** (§3) — whole-value vs interpolated vs static.
   Whole-value can carry functions/refs → need a raw-passthrough attr kind.
3. **Hoist `style-*` pseudo-attrs** (§6) into real CSS for non-React targets. Biggest
   lowering task.
4. **No `sc-else`** — complementary `sc-if` pairs encode either/or (§5).
5. **`from` on `x-import` is a literal URL**, never a hole; the name attrs may be holes.
6. **`renderVals()` is opaque JS** — transpile the template statically, but mostly
   wrap/pass-through the logic class rather than re-deriving it.
7. **`hint-*` attrs are streaming hints** — safe to drop for a static emit, or reuse
   `hint-placeholder-val` as a default and `hint-placeholder-count` as skeleton count.

---

## 12. Minimal IR node summary

```
Node =
  | Element       { tag, attrs[], children[] }
  | Text          { value }
  | Hole          { path[] | literal }
  | For           { list:Hole, itemName, body[] }     // binds itemName, $index
  | If            { cond:Hole, body[] }                // no else
  | Component      { source:'dc'|'module'|'global', name, from?, props[], children[], hintSize }
  | Helmet        { links[], styles[], scripts[] }

Attr =
  | StaticAttr      { name, value:string }
  | RawAttr         { name, hole:Hole }                 // raw passthrough (fn/ref/num/obj)
  | InterpAttr      { name, parts:(string|Hole)[] }     // → string
  | StyleAttr       { decls }                            // may contain holes
  | PseudoStyleAttr { state, decls }

Logic = { className:'Component', state, lifecycle[], renderVals:JS }   // opaque JS body
Props = Record<name, { editor, default, tsType?, options?, min?, max?, step?, unit?, section? }>
```
