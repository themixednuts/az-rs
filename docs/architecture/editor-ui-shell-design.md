# Azoth Editor UI Shell Design

## Invalidation boundary

The editor shell is a composition of persistent GPUI region entities, not one
monolithic invalidation scope. Titlebar, toolbar, activity rail, dock workspace,
status, and modal layers own their render notifications. Mode changes notify
the small chrome regions and let the cached `DockArea` swap its workspace;
selection and toolbar state no longer require rebuilding the full-window root.

This document is the reference design for the Azoth editor shell. It uses O3DE
editor conventions to define where Azoth UI elements belong.

The visual system and persistent agent-facing UI rules live in
`DESIGN.md`. This document owns component placement, interaction flow, and
editor shell architecture.

## Source touchpoints

O3DE reference paths are relative to its repository root:

- `Code/Tools/ProjectManager/Source/ProjectManagerWindow.cpp`
- `Code/Tools/ProjectManager/Source/ProjectsScreen.cpp`
- `Code/Tools/ProjectManager/Source/CreateProjectCtrl.cpp`
- `Code/Editor/MainWindow.cpp`
- `Code/Editor/QtViewPaneManager.cpp`
- `Code/Editor/AzAssetBrowser/AzAssetBrowserWindow.ui`
- `Gems/ScriptCanvas/Code/Editor/Components/EditorGraph.cpp`
- `Gems/GraphCanvas/Code/Include/GraphCanvas/Widgets/RootGraphicsItem.h`
- `Gems/Atom/Tools/MaterialEditor/Code/Source/Window/MaterialEditorMainWindow.h`

Current Azoth implementation and decisions:

  - `crates/editor/core/src/app.rs`
  - `crates/editor/core/src/workspace/dock.rs`
  - `crates/editor/ui/src/panels/mod.rs`
  - Linear ADR 0001, Universal Editor and Out-of-Process Project Services
  - Linear ADR 0012, Editor Viewport GPU Interop
  - Linear ADR 0013, Editor Reflection and Visual Node Graphs

## Reference lessons

| Source | Pattern | Azoth take |
| --- | --- | --- |
| O3DE | `ProjectManager` is a separate startup/project-maintenance UI. The loaded editor is a level/tool workspace: central viewport layout, registered view panes around it, status bar, asset browser, console, outliner, inspector. | Project creation/open/build/gem management starts outside the loaded editing shell. The loaded editor starts from content work, not setup forms. |

## Shell Split

The first architectural correction is explicit shell separation. The launcher
and loaded project editor are different applications surfaces, even when they
live in the same binary and GPUI process.

```mermaid
flowchart TB
    Start[Editor Process Start] --> HasProject{Project attached?}

    HasProject -- no --> Launcher[Launcher Shell]
    HasProject -- yes --> Workspace[Loaded Project Workspace]

    Launcher --> Recent[Recent Projects]
    Launcher --> Open[Open Existing Project]
    Launcher --> Create[Create Project Wizard]
    Launcher --> Import[Import or Register Project]
    Launcher --> EngineHealth[Engine and SDK Health]
    Launcher --> ProjectHealth[Selected Project Health]

    Open --> Attach[Attach Project Session]
    Create --> Init[azoth init/new workflow]
    Import --> Attach
    Attach --> Workspace
    Init --> Attach

    Workspace --> Menu[Menu Bar and Command Palette]
    Workspace --> Workbench[Center Document Workbench]
    Workspace --> LeftDock[Left Context Dock]
    Workspace --> RightDock[Right Details Dock]
    Workspace --> BottomDock[Bottom Diagnostics Dock]
    Workspace --> Status[Status Bar and Drawers]
    Workspace --> Modal[Modal and Notification Layer]
```

Rules:

- The launcher is for finding, creating, importing, repairing, and attaching
  projects.
- The launcher must not show loaded-editor docks such as asset browser,
  inspector, graph canvas, viewport, console, or session panels.
- The loaded editor must not use a project setup form as a default bottom panel.
- Opening an existing project is a file/directory selection flow. Git, LFS,
  gems, SDK state, and workflow health are read from the selected project,
  not entered as redundant launcher fields.

## Launcher Component Map

The launcher is a project browser and project-management surface. It is not a
small loaded editor and it should not expose editor docks.

```mermaid
flowchart TB
    subgraph Launcher[Launcher Shell]
        Title[Title Bar]
        Nav[Left Navigation]
        Recents[Recent Projects]
        Open[Open Existing Project]
        Create[Create Project Wizard]
        Import[Import/Register Project]
        Details[Selected Project Details]
        Health[Project Health Rail]
        Footer[Engine Version and Settings]
    end

    Nav --> Recents
    Nav --> Open
    Nav --> Create
    Nav --> Import

    Recents --> Details
    Open --> DirectoryPicker[Directory Picker]
    DirectoryPicker --> Probe[Read project manifest and .azoth state]
    Probe --> Details
    Create --> Template[Template and location selection]
    Template --> Review[Review source-control and gems]
    Review --> Init[azoth new/init]
    Import --> Probe

    Details --> Health
    Health --> CanAttach{Attachable?}
    CanAttach -- yes --> Attach[Open Project Session]
    CanAttach -- no --> Repair[Show required fixes]
    Attach --> Workspace[Loaded Workspace]
```

Launcher UX rules:

- Recent projects are navigable rows or tiles, not forms.
- Selecting a project shows details and health before opening.
- Open Existing starts with a directory picker. It does not ask for Git, LFS,
  gems, or package fields up front.
- Create Project is a guided wizard: identity, location, template, source
  control policy, review, create.
- Repair actions are explicit and explain what will change before they run.
- The launcher can show progress and errors for project creation/attach, but it
  does not show asset browser, viewport, graph canvas, inspector, console, or
  workspace session docks.

## Loaded Workspace Layout

The loaded editor shell should be content-first. The center is always an active
document or tool workbench. Docks support the active work; they are not the
primary workflow.

```mermaid
flowchart LR
    subgraph Shell[Loaded Project Workspace]
        Menu[Top: menu, command palette, project/session selector, run/build controls]
        subgraph Body[Docked Body]
            Left[Left Dock: project tree, entity outliner, search, graph palette]
            Center[Center Workbench: viewport, prefab, scene, graph, material, data, source-linked tool]
            Right[Right Dock: inspector, details, components, selected node/material parameters]
            Bottom[Bottom Dock: asset browser, console, output log, diagnostics, asset jobs, build/package jobs]
        end
        Status[Status Bar: workspace, source control, runtime, asset processor, errors, progress, drawers]
    end

    Menu --> Center
    Left --> Center
    Center --> Right
    Center --> Bottom
    Status --> Left
    Status --> Bottom
```

Default loaded-project layout:

- Center: project home, last opened level, or a level/prefab viewport when the
  project has an obvious startup asset.
- Left: authored/entity outliner.
- Right: inspector/details.
- Bottom: asset browser and console tabs, with output/jobs/diagnostics joining
  this dock as the workflow grows.
- Status: persistent project/session health, source control state, asset
  processor progress, runtime state, errors/warnings, and drawer buttons.

The current `ProjectWorkflowPanel` does not belong in the loaded workspace
default dock. It belongs in launcher/project management, project settings, or a
specific maintenance workflow.

The current `VisualGraphPanel` should not be a permanent default center tab. It
should open when the user creates or opens a graph-backed asset.

## Workspace Component Map

Loaded workspace components are placed by task role. A component should move
only when its role changes.

```mermaid
flowchart TB
    subgraph Top[Top Command Surface]
        Menus[File/Edit/View/Project/Assets/Build/Run/Tools/Help]
        CommandPalette[Command Palette]
        ProjectSelector[Project and Session Selector]
        RunBuild[Run/Simulate/Build Controls]
        SourceControl[Source Control Status]
    end

    subgraph Left[Left Dock: Navigation]
        ProjectTree[Project Tree]
        Outliner[Authored/Level Outliner]
        Search[Search and Find]
        ContextPalette[Context Palette]
    end

    subgraph Center[Center Workbench]
        Router[Document Router]
        Viewport[Viewport/Level/Prefab]
        Graph[Visual Graph]
        Material[Material/Shader]
        Table[Data Table]
        Preview[Asset Preview]
    end

    subgraph Right[Right Dock: Details]
        Inspector[Inspector]
        ComponentDetails[Component Details]
        NodeDetails[Node/Port Details]
        AssetMetadata[Asset Metadata]
    end

    subgraph Bottom[Bottom Dock: Content and Diagnostics]
        AssetBrowser[Asset Browser]
        Console[Console]
        OutputLog[Output Log]
        Problems[Problems/Diagnostics]
        AssetJobs[Asset Processor Jobs]
        BuildJobs[Build/Package Jobs]
        SessionDetail[Session/Worktree Detail]
    end

    Status[Status Bar and Drawers]

    AssetBrowser -- open asset --> Router
    ProjectTree -- open file/document --> Router
    Outliner -- select object --> Inspector
    Router --> Viewport
    Router --> Graph
    Router --> Material
    Router --> Table
    Router --> Preview
    Center -- current selection --> Right
    Center -- validation/build output --> Bottom
    RunBuild --> Bottom
    SourceControl --> Status
    AssetJobs --> Status
    SessionDetail --> Status
    Status -- drawer buttons --> Left
    Status -- drawer buttons --> Bottom
```

Workspace UX rules:

- Left dock answers "what scene/entity structure can I select?"
- Center answers "what am I editing?"
- Right dock answers "what is selected and editable?"
- Bottom dock answers "what content can I open, what happened, what is building, what failed?"
- Status bar answers "what is the current project/session/runtime state?"
- Menus and command palette execute commands; they do not duplicate entire
  workflow surfaces.

## Interaction Flows

Project open flow:

```mermaid
sequenceDiagram
    actor User
    participant Launcher
    participant Daemon as azd
    participant Session as session-supervisor
    participant ProjectHost as project-host
    participant AssetProcessor as asset-processor
    participant Workspace

    User->>Launcher: Choose project folder or recent project
    Launcher->>Daemon: Register/resolve project root
    Daemon-->>Launcher: Project record and known sessions
    Launcher->>Daemon: Ensure editor session and services
    Daemon->>Session: Start or reuse session supervisor
    Session->>ProjectHost: Ensure project-host descriptor
    Session->>AssetProcessor: Ensure asset-processor descriptor
    Daemon-->>Launcher: Attach session
    Launcher->>Workspace: Switch shell to loaded workspace
    Workspace->>ProjectHost: Load schema/catalog projections
    Workspace->>AssetProcessor: Load workspace asset view
```

Asset open flow:

```mermaid
sequenceDiagram
    actor User
    participant Browser as Asset Browser
    participant Router as Document Router
    participant Registry as Workbench Registry
    participant Host as project-host
    participant Workbench as Center Workbench
    participant Details as Right Details Dock

    User->>Browser: Open asset/source document
    Browser->>Router: Asset identity and schema type
    Router->>Registry: Resolve document editor descriptor
    Registry-->>Router: Workbench descriptor
    Router->>Host: Load authoritative document snapshot
    Host-->>Router: Snapshot and edit capabilities
    Router->>Workbench: Open or focus editor tab
    Workbench->>Details: Publish current selection
```

Graph edit/build flow:

```mermaid
sequenceDiagram
    actor User
    participant Palette as Node Palette
    participant Canvas as Graph Canvas
    participant Inspector as Node Inspector
    participant Host as project-host
    participant Processor as asset-processor
    participant Results as Compiler Results
    participant Catalog as AssetCatalog

    User->>Palette: Add node
    Palette->>Host: AddGraphNode command
    Host-->>Canvas: Validated graph projection
    User->>Canvas: Connect ports or move node
    Canvas->>Host: GraphCommand
    Host-->>Inspector: Selection/edit projection
    User->>Canvas: Save and build
    Canvas->>Host: Save graph document
    Host->>Processor: Enqueue graph compiler job
    Processor-->>Results: Build status and diagnostics
    Processor-->>Catalog: Runtime products
```

These flows keep mutable project truth inside project services. The editor
mutates user intent and presentation state; project-host validates document
commands and asset-processor produces runtime products.

## Workbench Model

Azoth should treat major editing experiences as workbenches registered by
engine/editor code or by project-host/tool-host descriptors. A workbench owns
its document editor, contextual panels, commands, and default layout.

```mermaid
classDiagram
    class WorkbenchDescriptor {
        +WorkbenchId id
        +String display_name
        +Icon icon
        +DocumentKind[] documents
        +PanelContribution[] panels
        +CommandContribution[] commands
        +LayoutProfile default_layout
    }

    class DocumentEditorDescriptor {
        +DocumentKind kind
        +AssetKind asset_kind
        +open(DocumentRef)
        +save(GraphCommand|AssetCommand)
        +dirty_state()
    }

    class PanelContribution {
        +PanelId panel
        +PanelRole role
        +DockPosition[] allowed_positions
        +DockPosition default_position
        +VisibilityPolicy default_visibility
    }

    class EditorShell {
        +WorkbenchRegistry registry
        +DocumentRouter router
        +DockLayout layout
        +StatusModel status
    }

    class ProjectHost {
        +asset snapshots
        +graph snapshots
        +validation
        +save commands
    }

    EditorShell --> WorkbenchDescriptor
    WorkbenchDescriptor --> DocumentEditorDescriptor
    WorkbenchDescriptor --> PanelContribution
    DocumentEditorDescriptor --> ProjectHost
```

Workbench examples:

- Level/Prefab Workbench: viewport center, outliner left, inspector right,
  console/jobs bottom.
- Visual Graph Workbench: graph canvas center, node palette left, details right,
  compiler/results bottom.
- Material/Shader Workbench: material graph or property editor center, preview
  viewport center/split, palette left, parameters right, shader compile output
  bottom.
- Data Table Workbench: table/grid center, schema/properties right, validation
  results bottom.
- Asset Preview Workbench: model/texture/audio preview center, metadata/details
  right.

## Workbench Element Maps

Level/prefab workbench:

```mermaid
flowchart LR
    AssetBrowser[Asset Browser] --> Viewport[Viewport Center]
    Outliner[Outliner] --> Viewport
    Viewport --> Inspector[Inspector Right]
    Viewport --> Runtime[Runtime Host]
    Runtime --> Viewport
    Viewport --> Problems[Problems Bottom]
    Status[Status Bar] --> Runtime
```

Visual graph workbench:

```mermaid
flowchart TB
    Palette[Left: Node Palette and Templates] --> Canvas[Center: Graph Canvas]
    DocumentList[Left: Graph Documents] --> Canvas
    Canvas --> Selection[Selection Model]
    Selection --> Details[Right: Node/Port/Graph Details]
    Canvas --> Routes[Connection Router and Anchors]
    Routes --> Canvas
    Canvas --> HostCommands[project-host GraphCommands]
    HostCommands --> Projection[Validated Graph Projection]
    Projection --> Canvas
    Canvas --> Build[Build Graph]
    Build --> Results[Bottom: Compiler Results]
    Results --> SourceLink[Open Source / Generated Artifact Links]
```

Material/shader workbench:

```mermaid
flowchart TB
    Palette[Left: Material Nodes and Functions] --> Graph[Center: Material Graph]
    Graph --> Preview[Center Split: Material Preview Viewport]
    Graph --> Params[Right: Parameters and Properties]
    Params --> Graph
    Graph --> ShaderCompiler[Shader Compiler Backend]
    ShaderCompiler --> Results[Bottom: Compile Output]
    ShaderCompiler --> Products[Pipeline and Shader Products]
```

Data table workbench:

```mermaid
flowchart LR
    AssetBrowser[Asset Browser] --> Grid[Center: Table/Grid Editor]
    Grid --> Schema[Right: Schema and Column Details]
    Grid --> Validation[Bottom: Validation Results]
    Grid --> Host[project-host DB-owned Source Payload]
    Host --> Grid
```

UX improvements implied by these maps:

- Graph/material palettes are contextual left surfaces, not global left docks.
- Compiler output is bottom diagnostics for the active document, not a separate
  always-open global console.
- Selection drives the right inspector uniformly across viewport, graph,
  material, data, and asset preview workbenches.
- Status bar drawer buttons should open high-volume surfaces on demand.
- Empty center states should offer the next concrete action: open level, create
  asset, open graph, or run project. They should not explain the whole editor.

## Service Boundary

The editor process owns presentation and interaction state. Project truth stays
behind services.

```mermaid
flowchart TB
    UI[GPUI Shell and Workbenches] --> Controllers[Editor Controllers]
    Controllers --> Protocol[Cap'n Proto RPC]

    Protocol --> ProjectHost[project-host]
    Protocol --> AssetProcessor[asset-processor]
    Protocol --> RuntimeHost[runtime-host]
    Protocol --> SessionSupervisor[session supervisor]
    Protocol --> ToolHost[future tool-host]

    ProjectHost --> AssetDb[(Asset DB)]
    ProjectHost --> SourcePayloads[(DB-owned source payloads)]
    AssetProcessor --> ProductCache[(Dev product cache)]
    AssetProcessor --> AssetCatalog[(AssetCatalog)]
    RuntimeHost --> Viewport[Viewport GPU side channel]

    AssetDb --> Controllers
    SourcePayloads --> Controllers
    AssetCatalog --> RuntimeHost
    Viewport --> UI
```

Rules:

- The editor does not load project or gem code into the editor process.
- Project/gem-specific tools are surfaced through service descriptors and
  generated schemas/protocols.
- Project-specific tools, crates, and assets remain project-owned and appear
  through project or gem registration, not engine UI hardcoding.
- DB-owned authored source is the project authority. The product cache and
  packaged output are build products.

## Graph And Node Editing

Graph editing is an asset workflow, not a global editor mode. The authoring
document is editable; runtime products are compiled.

```mermaid
flowchart LR
    RustTypes[Rust traits, macros, schemas] --> Catalog[NodeTypeCatalog]
    Catalog --> Palette[Editor Node Palette]
    Catalog --> Validator[Project-host validator]

    User[User edits graph asset] --> Commands[GraphCommands]
    Commands --> Validator
    Validator --> Document[(VisualGraphDocument)]
    Document --> Compiler[GraphCompilerBackend]

    Compiler --> Gameplay[Generated Rust or typed IR]
    Compiler --> Materials[Shader bytecode and pipeline products]
    Compiler --> Animation[State-machine tables]
    Compiler --> Tools[Editor-only interpreted/debug products]

    Gameplay --> Products[(AssetCatalog products)]
    Materials --> Products
    Animation --> Products
    Tools --> Products
```

Hot graph categories must compile to domain-native products: generated Rust,
shader products, pipeline state, state-machine tables, or tightly packed typed
IR with resolved bindings. The node editor may use reflective metadata while
authoring, but gameplay, shader/material, animation, and other hot runtime
systems must not execute through editor metadata, Cap'n Proto messages, DB rows,
RON, or dynamic graph interpretation.

## Placement Rules

These are the rules to apply when adding any new editor UI:

- Project discovery, creation, import, repair, and initial health checks belong
  to the launcher/project manager.
- Active editing belongs to the center workbench.
- Structural scene/entity navigation belongs to the left dock.
- Content browsing belongs to the bottom dock with console/output workflows.
- Properties, selected-object details, and parameters belong to the right dock.
- Logs, diagnostics, compiler results, asset processor jobs, and package/build
  jobs belong to the bottom dock or status-bar drawers.
- Persistent low-noise state belongs to the status bar.
- Menus and command palette expose commands; they should not duplicate whole
  panels.
- Asset-specific editors own contextual palettes and inspectors; they should
  not become permanent global tabs.
- Project-specific UI reaches the editor through descriptors and project
  services, never by linking project code into the universal editor.
