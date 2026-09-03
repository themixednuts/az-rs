---
title: Dock
description: Fixed side and bottom panel layout for editor workspaces.
---

# Dock

The dock component arranges fixed left, right, and bottom panels around a
center work area. It is intended for editor-style workspaces where side panels
and bottom tool panes remain anchored while the center view changes.

## Bottom Dock Extent

Use `DockArea::set_bottom_dock_extent` to choose how the bottom dock spans
relative to side docks.

```rust
use gpui_component::dock::{BottomDockExtent, DockArea};

dock_area.update(cx, |dock_area, cx| {
    dock_area.set_bottom_dock_extent(BottomDockExtent::BoundedByRight, cx);
});
```

`BottomDockExtent` has four layouts:

- `BetweenLeftAndRight`: bottom dock is constrained between left and right
  docks.
- `BoundedByRight`: bottom dock extends under the left dock and stops at the
  right dock.
- `BoundedByLeft`: bottom dock starts after the left dock and extends under the
  right dock.
- `FullWidth`: bottom dock spans the full dock area width.

Use `BetweenLeftAndRight` for IDE layouts where both side docks own the full
height. Use `BoundedByRight` for game-editor layouts where the right inspector
owns the outer corner and the asset/console bottom panel should stretch from
the left edge up to the inspector.
