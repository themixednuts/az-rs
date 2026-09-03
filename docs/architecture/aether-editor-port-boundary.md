# Aether Editor Port Boundary

The Aether editor UI in `crates/editor/core/src/app/aether_editor_view.rs` is
owned runtime code. It started from DC/htmlswap output, but it is no longer a
generated file and must not be replaced wholesale by a regeneration.

Future DC/htmlswap runs are reference material:

- Compare by named region or component area, such as titlebar, toolbar, scene
  hierarchy, viewport, inspector, bottom dock, status bar, material editor,
  script editor, animation/mannequin, game data, settings, and modal flows.
- Port only the region that changed, preserving the existing editor state,
  action handlers, theme mapping, asset projections, dock behavior, and
  engine/project integrations.
- Keep DC source comments and region markers when they help locate the
  matching area, but do not let generated placeholder data become runtime
  truth.

The runtime view-model in `aether_editor_model.rs` must not use generated
render-value seed data. Fixed chrome belongs in typed Rust
constants/builders; dynamic regions should read live projections from project
host, asset browser, authored inspection, graph documents, runtime state, and
editor settings.

When an area is not implemented yet, show an explicit placeholder in the UI
copy or data model. The placeholder should say what backing source is missing,
for example "Script editor placeholder: no script buffer is open" or
"Mannequin placeholder: animation graph projection is not connected". Do not
hide missing behavior behind realistic-looking sample rows.

The project manager follows the same rule once adopted: regenerate only to
compare design changes, then port intentionally into the owned view and
view-model.
