# AZoth Engine

`az-engine` is the lean runtime-facing engine crate for the Azoth
workspace. It provides the shared Bevy ECS plugin surface and a small set
of engine-owned utilities that are safe to use from projects, gems, and
runtime hosts.

The crate intentionally does not contain project services, editor UI,
networking backends, physics backends, project-specific compatibility code, or
authored-data schema reflection. Those live in project/gem crates,
service crates, or `az-schema` and are selected through the project
workflow.

## Current Surface

```text
az-engine/
├── core/    # engine error type plus Bevy math/time re-exports
├── entity   # EntityFlags and EntityPlugin reflection registration
└── asset/   # Bevy asset type re-exports only
```

The root `EnginePlugin` currently installs `EntityPlugin`. Runtime,
networking, rendering, physics, scripting, and game-specific plugins are
expected to be project/gem contributions rather than base-engine
dependencies.

## Usage

```rust,no_run
use bevy::prelude::*;
use az_engine::prelude::*;

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, EnginePlugin))
        .run();
}
```

Spawn entities with Bevy components plus Azoth metadata:

```rust,no_run
use bevy::prelude::*;
use az_engine::prelude::*;

fn spawn_entities(mut commands: Commands) {
    commands.spawn((
        Name::new("SavedEntity"),
        EntityFlags::new(),
    ));

    commands.spawn((
        Name::new("RuntimeHelper"),
        EntityFlags::temporary(),
    ));
}
```

Runtime asset inventory is not a base-engine concern. Launch snapshots
carry package roots, `az-framework` mounts the selected Asset Catalog, and
Bevy loaders resolve product bytes through that catalog. Do not add
parallel bundle manifests or folder scans to `az-engine`.

## Project And Gem Integration

Azoth's project workflow is the composition point. Generated projects and
gems depend directly on the focused crates they need:

- `az-schema` for authored-data descriptors consumed by project-host.
- `az-asset-builder` for project/gem asset-builder registrations.
- `az-runtime-host` for runtime projection registrations.
- Project or gem backend crates for networking, physics, rendering,
  platform, or game-specific systems.

Those registrations are force-linked into project-owned service binaries
and discovered through Rust inventory. `az-editor` stays a universal IPC
client and does not link project/gem native code.

## Boundary Rules

- Do not add editor, daemon, session, project-host, runtime-host,
  asset-db, project-specific, legacy-format, or gem dependencies to
  `az-engine`.
- Do not add runtime bundle manifests, release indexes, or asset folder
  scans to `az-engine`; package roots plus the Asset Catalog are the
  runtime inventory.
- Do not add a second authored-schema or editor value system to `az-engine`.
  Project and gem types register normal Bevy reflection and narrow owning-domain
  TypeData through their plugins; project-host projects the composed registry to
  the universal editor.
- Do not add dormant `network/` or `physics/` modules to the base engine.
  Backend implementations belong in project/gem crates selected by
  manifests.
- Keep public examples limited to APIs that exist in this crate today.

Architecture tests in `src/lib.rs` enforce these rules.

## Building

```bash
cargo build -p az-engine
```

## Testing

```bash
cargo test -p az-engine
```
