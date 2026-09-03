---
version: alpha
name: Azoth Editor
description: Dense, production-focused editor UI for the Azoth engine and universal project workflow.
colors:
  primary: "#1F6FEB"
  on-primary: "#FFFFFF"
  secondary: "#6B7280"
  tertiary: "#B45309"
  neutral: "#F7F8FA"
  surface: "#FFFFFF"
  surface-muted: "#F1F3F5"
  surface-panel: "#FAFBFC"
  canvas: "#F6F8FA"
  text: "#1F2328"
  text-muted: "#57606A"
  border: "#D0D7DE"
  focus: "#0969DA"
  success: "#1A7F37"
  warning: "#9A6700"
  error: "#CF222E"
typography:
  title:
    fontFamily: Segoe UI
    fontSize: 14px
    fontWeight: 600
    lineHeight: 1.2
    letterSpacing: 0
  panel-heading:
    fontFamily: Segoe UI
    fontSize: 13px
    fontWeight: 600
    lineHeight: 1.2
    letterSpacing: 0
  body:
    fontFamily: Segoe UI
    fontSize: 12px
    fontWeight: 400
    lineHeight: 1.35
    letterSpacing: 0
  label:
    fontFamily: Segoe UI
    fontSize: 11px
    fontWeight: 500
    lineHeight: 1.25
    letterSpacing: 0
  data:
    fontFamily: Cascadia Mono
    fontSize: 11px
    fontWeight: 400
    lineHeight: 1.25
    letterSpacing: 0
rounded:
  none: 0px
  sm: 4px
  md: 6px
  lg: 8px
spacing:
  xxs: 2px
  xs: 4px
  sm: 8px
  md: 12px
  lg: 16px
  xl: 24px
  panel: 8px
components:
  title-bar:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.text}"
    borderColor: "{colors.border}"
    typography: "{typography.title}"
    height: 30px
  menu-bar:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.text}"
    borderColor: "{colors.border}"
    typography: "{typography.label}"
    height: 28px
  dock-panel:
    backgroundColor: "{colors.surface-panel}"
    textColor: "{colors.text}"
    borderColor: "{colors.border}"
    rounded: "{rounded.none}"
    padding: "{spacing.panel}"
  status-bar:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.text-muted}"
    borderColor: "{colors.border}"
    typography: "{typography.label}"
    height: 30px
  button-primary:
    backgroundColor: "{colors.primary}"
    textColor: "{colors.on-primary}"
    rounded: "{rounded.sm}"
    padding: 8px
  button-secondary:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.text}"
    borderColor: "{colors.border}"
    rounded: "{rounded.sm}"
    padding: 8px
  input:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.text}"
    borderColor: "{colors.border}"
    rounded: "{rounded.sm}"
    padding: 8px
  graph-node:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.text}"
    borderColor: "{colors.border}"
    rounded: "{rounded.md}"
    padding: 8px
  viewport:
    backgroundColor: "{colors.canvas}"
    textColor: "{colors.text-muted}"
    borderColor: "{colors.border}"
---

# Azoth Editor Design System

## Overview

Azoth Editor is a utilitarian game-engine editor, not a landing page and not a
web app dashboard. It should feel like a professional content-production tool:
dense, stable, readable, and predictable under long editing sessions.

The target user is an engine or game developer repeatedly moving between
viewport work, asset inspection, graph editing, logs, jobs, source control, and
project/session state. The UI should reduce mode confusion by making the current
shell obvious: project launcher before attachment, loaded project workspace
after attachment, and asset workbench when a document is open.

## Colors

The palette uses quiet neutral surfaces with a restrained blue primary action.
State colors are semantic and should appear only where they carry information.

- **Primary:** actions that move the current workflow forward.
- **Secondary:** borders, metadata, inactive controls, and status context.
- **Tertiary:** caution or build/workflow attention states.
- **Neutral/surface:** the default editor chrome and panel foundation.
- **Canvas:** viewport, graph, and other spatial editing surfaces.

Avoid one-note color themes. The editor should not become all blue, all slate,
all beige, all purple, or all brown. Most chrome should be neutral; color should
explain state or action priority.

## Typography

Use the system UI font for editor chrome and controls. On Windows this is Segoe
UI. Technical identifiers, hashes, file paths, service names, and protocol
labels use Cascadia Mono or the configured monospace font.

Type scale is compact. Hero-scale text does not belong inside the loaded editor.
Large headings are reserved for the launcher and empty project states.

## Layout

The editor uses a fixed-shell layout:

- Launcher shell: title bar, project navigation, project list/browser, selected
  project health, and create/import/open workflows.
- Loaded workspace shell: title/menu, center workbench, left navigation dock,
  right details dock, bottom diagnostics dock, and status bar.
- Asset workbench: document-specific center surface with contextual palette,
  inspector, compiler/results, and status.

Spacing uses an 8px base rhythm with 4px micro spacing. Panels are dense but not
crowded. Toolbars and status bars use fixed heights so layout does not jump when
status text changes.

## Elevation & Depth

Depth comes from borders, tonal separation, and stable dock placement. Avoid
heavy shadows and decorative floating cards. Modals and transient popovers may
use a subtle shadow, but primary editor sections should read as docked panes.

## Shapes

Editor chrome is mostly square. Buttons, inputs, graph nodes, and repeated
items use 4px to 6px radius. Cards, when required for repeated launcher/project
items, may use up to 8px radius. Do not nest cards inside cards.

## Components

Launcher components:

- Recent project list: project name, path, last opened time, validation status,
  and one open action.
- Open existing project: directory picker first, then manifest/project health.
- Create project wizard: project identity, location, template, source-control
  policy, then review/create.
- Project health rail: engine compatibility, missing gems, source control, and
  services needed to attach.

Loaded workspace components:

- Top bar: project/session selector, main menus, command palette, run/build
  controls, and source-control status.
- Center workbench: viewport, prefab/scene, graph, material, data table, or
  asset preview document. It is never a project setup form.
- Left dock: asset browser, authored outliner, project tree, search, and
  context palettes.
- Right dock: inspector/details for current selection.
- Bottom dock: console, output log, diagnostics, asset processor jobs, compiler
  results, packaging/build jobs, and session/worktree detail.
- Status bar: low-noise project/session/runtime/asset/GPU state plus drawer
  buttons for high-volume surfaces.

Graph workbench components:

- Palette left: node categories, search, templates, and allowed node types.
- Canvas center: graph document, nodes, ports, routes, comments, zoom/pan.
- Details right: selected node, port, graph, or comment properties.
- Results bottom: validation, compiler status, generated artifact links, and
  source navigation.

## Do's and Don'ts

- Do keep launcher and loaded workspace visually distinct.
- Do keep the center reserved for active documents and workbenches.
- Do make persistent state visible in the status bar before opening large
  panels.
- Do show file paths, service names, hashes, and protocol data in monospace.
- Do keep graph/material/asset editors contextual to the document being edited.
- Don't put project setup inside the loaded editor's default dock layout.
- Don't default-open graph, logs, and session panels unless the user requested
  them or there is a blocking problem.
- Don't duplicate actions in multiple neighboring buttons unless one is a menu
  entry and one is the primary visible action.
- Don't use decorative hero sections, floating marketing cards, or oversized
  headings in the loaded editor.
- Don't let text resize or wrap in a way that changes toolbar, status-bar, or
  dock dimensions.
