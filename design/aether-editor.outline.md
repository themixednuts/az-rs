# aether-editor.html — structural breakdown (ownership + flow)

Authoritative map of `design/aether-editor.html` so the Rust editor shell mirrors
it exactly. Source of truth for *who owns who* and *what lives where*.

## Top-level stack (vertical, full viewport)

```
┌ menu / title bar      34px  flex-none
├ toolbar               42px  flex-none
├ WORKSPACE             flex-1            ← horizontal: [activity rail | mode body]
└ status bar            27px  flex-none
```

### 1. Menu / title bar (34px)
- left: brand diamond → in-window menu bar (File, Edit, Create, Select, Tools, Build, Window, Help)
- center: project breadcrumb — `RiftRunner.aeproj — Canyon_01 ·unsaved-dot`
- right: layout selector + window controls (min/max/close)

### 2. Toolbar (42px) — three zones
- left (clips when tight): save / undo / redo | **transform tools** (select·move·rotate·scale segmented) | pivot+space segmented | grid-snap + angle-snap
- center (always centered): **transport** — play / stop / step | simulate
- right: **Build** dropdown (config + platform + actions) | settings gear

### 3. WORKSPACE = `[activity rail | <active mode body>]`
- **activity rail** (46px, left edge): **MODE switch icons** (Scene / Materials / Scripting / …) + help + settings pinned bottom. Switching a mode swaps the *entire body* (`modeScene` / `modeMaterials` / `modeScripting`). **The rail is NOT a dock toggler.**

#### Mode body: SCENE (default) — this is the docked workspace
```
SCENE BODY  (horizontal)
├ MAIN AREA            flex-1, vertical
│   ├ upper row        flex-1, horizontal
│   │   ├ LEFT DOCK    hierarchy   (tabs: Hierarchy | Layers) + search + rows + entity-count footer
│   │   ├ split
│   │   └ CENTER       viewport tabs + viewport canvas (overlays: view pills, stats HUD, nav info, gizmo triad)
│   ├ split (horizontal)
│   └ BOTTOM DOCK      full width under left+center — tabs: Assets | Console | Output | Profiler | Gems
├ split (vertical)
└ RIGHT DOCK           inspector (tabs: Inspector | …) — entity header + components list + Add Component
```

Dock ownership (the bit I keep getting wrong):
| Dock   | Contents                                            |
|--------|-----------------------------------------------------|
| Left   | **Hierarchy / Outliner** (entities), Layers         |
| Center | **Viewport** (+ other center workbenches as tabs)   |
| Bottom | **Asset Browser**, Console, Output, Profiler, Gems  |
| Right  | **Inspector**                                       |

*Asset Browser is BOTTOM, never Left. Left is entities.*

#### Mode body: MATERIALS — node-graph material editor (palette | canvas | preview+params). *Not implemented in az editor.*
#### Mode body: SCRIPTING — file-tree | code | outline. *Not implemented in az editor.*

### 4. Status bar (27px)
- far-left: **left-dock toggle** (hierarchy show/hide)
- info cluster (clips): compiler/build status · asset pipeline · source-control (branch ±dirty) · selection+filetype · diagnostics (err/warn counts)
- right cluster: cursor XYZ · fps/ms/tris/draws · gpu+version · **panel show/hide toggles** (bottom/right docks)

## Mapping to the az editor (what actually exists)
- Modes that exist: **Scene** (LevelViewport center workbench) and **Graph** (VisualGraph center workbench). Materials/Scripting do **not** exist → omit from the rail (don't fake dead buttons).
- Dock layout already correct in `crates/editor/core/src/workspace/layout.rs`: Left=AuthoredOutliner, Right=AuthoredInspector, Bottom=AssetBrowser+Console, Center=ViewportPanel.
- Dock show/hide → status bar + View menu (NOT the activity rail).
- Dead panel files under `crates/editor/ui/src/panels/` (gems, layers, profiler, output_log, create_authored_type) are not registered → ignore.

## Status bar (az shell) — wired to real globals
Segments render only when their data global exists (no faked cursor/fps/tris):
| Segment        | Source global                          |
|----------------|----------------------------------------|
| source control | `EditorSessionStatus` (branch, clean)  |
| assets         | `EditorAssetBrowserStatus` (entries/roots/error) → toggles bottom dock |
| selection      | `EditorAuthoredInspection` (schema)    → toggles inspector |
| diagnostics    | `ConsoleState` (Error/Warn counts)     → toggles console |
| runtime        | `EditorRuntimeStatus.state`            |
| viewport       | `EditorViewportRenderStatus` (WxH/state) |
| gpu + version  | `EditorGpuStatus.adapter_name` + `AZoth <pkg-version>` |
| dock toggles   | PanelLeft/Bottom/Right → Toggle{Outliner,Console,Inspector} |
Omitted vs HTML (no real data source yet): cursor XYZ, fps/ms/tris/draws/vram.

## Menus (az shell, `crates/editor/ui/src/menu/mod.rs`) — real actions only
File · Edit · View (panels + graph + fullscreen) · Run (editor-world launch/stop/refresh) · Session (lifecycle) · Window · Help. Every item dispatches a real, installed action handler. HTML categories with no backing (Create/Select/Tools, Bake Lighting, transform tools) are omitted rather than added as dead items.

## Invariants for the Rust shell
1. Activity rail = workspace-mode switch only (Scene, Graph). Never dock toggles.
2. Asset Browser lives in the **bottom** dock. Hierarchy/entities live **left**. Inspector **right**.
3. Dock toggle actions must target the dock that actually owns the panel:
   `ToggleOutliner→Left`, `ToggleInspector→Right`, `ToggleAssetBrowser→Bottom`, `ToggleConsole→Bottom`, `ToggleSessionPanel→Bottom`.
4. No fake/dead controls: only render a button if a real action backs it.
