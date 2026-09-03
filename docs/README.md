# Documentation

Start with the repository [README](../README.md) for installation and the
shortest working path. Use this index when you need design context or operating
rules.

## Work with the engine

- [Agent issue tracking](agents/issue-tracker.md) explains how active,
  multi-session work is recorded.
- [Native asset formats](architecture/native-asset-formats.md) defines the
  authoring, build, and runtime asset boundary.
- [Legacy format ownership](architecture/legacy-format-ownership.md) explains
  where compatibility codecs, importers, builders, and tools belong.
- [Terrain source route](architecture/terrain-source-route.md) traces terrain
  data from authoring input to runtime use.
- [Scene, glTF, Blender, and animation route](architecture/scene-gltf-blender-animation-route.md)
  traces scene and animation ownership.

## Understand the architecture

The [domain language](reference/domain-language.md) defines the engine, editor,
project, asset, session, graph, and GameData terms used across the codebase.

The [architecture directory](architecture/) contains current cross-component
contracts and call-stack descriptions. Read the page for the subsystem you are
changing; follow its links to the relevant decision records and code.

- [Aether editor port boundary](architecture/aether-editor-port-boundary.md)
  defines how generated design references may influence the adopted editor UI.
- [Editor UI shell design](architecture/editor-ui-shell-design.md) defines the
  editor regions, invalidation boundaries, and interaction flow.
- [Engine secret-resolution call stack](architecture/engine-secret-resolution-call-stack.md)
  traces secret references from trusted hosts to provider adapters.
- [Certified federation authority atom call stack](architecture/certified-federation-authority-atom-call-stack.md)
  traces one authoritative state mutation and its durable evidence.
- [Certified federation signing call stack](architecture/certified-federation-signing-call-stack.md)
  defines role-bound signing and verification ownership.
- [World instance lifecycle ownership](architecture/world-instance-lifecycle-ownership.md)
  fixes the identities, placement fence, and consumer-owned effect ports that
  make dynamic world instances executable without enabling a runtime path.

The [Azoth architecture decisions project](https://linear.app/openworldserver/project/azoth-architecture-decisions-f6496e23b9e3)
in Linear is the authority for accepted, superseded, and proposed decisions.
Its [Azoth ADR index](https://linear.app/openworldserver/document/azoth-adr-index-dc99abd6cc4f)
links every record.

## Follow active work

Active, multi-session execution maps and their project documents live in
Linear. Repository documentation describes current behavior rather than
tracking work.

## Documentation rules

- Keep current instructions and reference material under `docs/`.
- Keep architectural decisions and active execution plans in Linear.
- Keep implementation documentation under `docs/`.
- Link to repository-relative files and stable symbols, not line numbers or
  machine-local paths.
- Cite only Lumberyard or O3DE when an external engine source is necessary.
- Put generated data, captures, logs, and local analysis outside the tracked
  documentation tree.
