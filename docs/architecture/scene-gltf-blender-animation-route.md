# Scene, glTF, Blender, and Animation Route

This note records the native scene/model/animation direction for Azoth.
It is intentionally layered so the top sections are enough for orientation,
while the later sections spell out implementation details and traps.

## Short Version

`glTF`/`GLB` is the native interchange and DCC round-trip blob. It is not a
Prefab document and it is not a runtime format.

The editable Azoth source is a small schema-backed sidecar that references the
blob and says what to extract:

```text
model.glb
model.scene.ron        # azoth.scene.SceneImportManifest
  -> asset processor
  -> Azoth runtime products
```

Runtime loads Azoth products only. It does not parse `glTF`, call Blender, or
carry legacy compatibility formats.

## Core Split

Scene assets have three separate roles:

| Role | Example | Typed reflected source | Runtime input |
| --- | --- | --- | --- |
| Scene blob | `model.glb`, imported `.gltf` plus dependencies | no | no |
| Authored sidecar | `model.scene.ron` | yes | no |
| Cooked products | mesh, skeleton, animation clip, material binding products | no | yes |

The scene blob is tracked source data. The sidecar is authored editor data. The
products are rebuildable cache/release data.

This follows the useful part of O3DE/Lumberyard SceneAPI: import a scene graph,
apply a manifest of groups and rules, then emit products. Azoth should not copy
the legacy exporter-plugin model, and it should not treat imported byte formats
as editor/runtime schemas.

## glTF and Blender

Prefer `.glb` for generated interchange because it is self-contained, stable to
hash as one source payload, and easy for users to open in Blender.

Accept `.gltf` imports, but treat external `.bin` and image URIs as source
dependencies. Editing an external buffer or texture must invalidate the right
asset-processor jobs.

Blender integration should start with normal file round-tripping:

```text
Azoth/editor opens source folder
user opens model.glb in Blender
user exports model.glb
asset processor rebuilds from model.glb + model.scene.ron
```

A Blender add-on or IPC/RPC bridge can come later, but it should talk to the
editor/project services and update the same files. It must not become a runtime
dependency or a required build dependency.

## Native Schemas

The first native scene schema should be:

```rust
pub struct SceneImportManifest {
    pub source_scene: AssetPathBuf,
    pub import_settings: SceneImportSettings,
    pub groups: Vec<SceneGroup>,
    pub rules: Vec<SceneRule>,
}
```

`SceneGroup` should cover the product families extracted from one blob:

- `MeshGroup`
- `SkinnedMeshGroup`
- `SkeletonGroup`
- `AnimationGroup`
- `MorphTargetGroup`

`SceneRule` should cover import behavior:

- `CoordinateSystemRule`
- `LodRule`
- `MaterialAssignmentRule`
- `TangentRule`
- `BlendShapeRule`
- `RootMotionRule`
- `EventTrackRule`
- `CompressionRule`

Animation authored schemas should be native concepts, not legacy names:

- `azoth.anim.BlendSpace`
- `azoth.anim.EventTrack`
- `azoth.anim.AnimationGraph`
- `azoth.anim.AnimationController`

Do not put animation curve data, mesh vertex data, skeleton joint arrays, or
other large scene payloads into RON just because the sidecar is a reflected Rust
type. Only the user's import choices and authored graph/controller data belong
in typed source documents.

## Legacy Import Boundary

Legacy formats are importer inputs only:

```text
.cgf / .skin / .chr / .caf / .adb / actions.xml / tags.xml / controllerdefs.xml
  -> private parser structs
  -> glTF/GLB blob where useful
  -> native Azoth sidecars
  -> Azoth runtime products
```

Legacy parser structs should stay private to importer crates unless a test or
diagnostic tool needs a focused interface. They should not register as native
editor/source types, should not define native runtime product formats, and should not appear as
`azoth.compat.*` source schemas.

Import provenance still matters, but it belongs in the asset DB and job/product
lineage: original path, source hash, importer id/version, emitted sidecar path,
rule hash, diagnostics, and unsupported legacy fields.

## Mannequin Route

CryAction Mannequin is not a model format. It is an animation selection,
fragment, tag, scope, and action-install system.

Azoth should not clone Mannequin. It should import Mannequin concepts into
native animation documents:

| Mannequin concept | Azoth target |
| --- | --- |
| Fragments | animation graph states, blend nodes, or named actions |
| Tags / tag states | graph parameters, state tags, transition predicates |
| Scopes / scope contexts | layers and bone masks |
| ADB clip lookup | animation controller clip set |
| Procedural clips | animation graph nodes such as IK, look-at, sound/event emitters |
| Fragment blends | graph transitions and blend rules |

The runtime target is a compiled animation graph/controller product, not a
Mannequin interpreter.

## Source vs Product Policy

File-backed, editable source schemas:

- `*.scene.ron`
- material source docs
- blend-space docs
- event-track docs
- animation graph docs
- animation controller docs

Tracked binary source blobs:

- `.glb`
- `.gltf` plus external buffers/images
- optional future `.blend` inputs if a Blender importer worker exists

Runtime products:

- mesh products
- skeleton products
- skinned mesh products
- animation clip products
- event track products
- material products
- compiled animation graph/controller products

Products carry `ProductFormatId` and product format versions. They are not
generic reflected authoring documents.

## Traps

Coordinate systems must be explicit. glTF is right-handed, Y-up, meters.
Cry/Lumberyard data may be Z-up with different forward-axis conventions. Put
the conversion in `CoordinateSystemRule` and bake it into products once.

Material conversion should target native Azoth material types. Legacy material
parameters should map into a known PBR template where possible, with unmapped
values recorded as diagnostics/provenance.

Skeletons and clips must be separated into products with stable joint identity.
Clip imports should reference skeleton identity rather than duplicating joint
layout assumptions.

LOD rules need to be authored. Do not silently infer all LOD behavior from file
names without surfacing it in `model.scene.ron`.

Event tracks should be separate enough that editing events does not rebuild
geometry.

Root motion needs one authored extraction rule. Double-applying Blender root
transforms and runtime root motion is a common failure mode.

Partial rebuilds should be per group/rule product where possible. One sidecar
can produce many products, and each product should carry enough rule/source hash
data to invalidate independently.
