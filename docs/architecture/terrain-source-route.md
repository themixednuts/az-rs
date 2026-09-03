# Terrain Source Route

This note records the terrain source direction for Azoth. It is layered so the
top sections are enough for orientation, while the later sections capture the
reasoning and edge cases.

## Short Version

O3DE is the reference for terrain decomposition, not the source storage model.

Azoth should copy O3DE's domain split:

- world bounds, height range, and query/cook resolution
- region authority with priority and overlap rules
- separate height and surface providers
- weighted surface tags
- render material and physics material mappings
- dirty-region and dependency invalidation

Azoth should not copy O3DE's entity/component graph as the primary authored
terrain source. The source should be explicit typed Rust documents, reflected
through the composed Bevy registry for editing, that compile into deterministic
runtime products.

```text
terrain source docs + height/surface blobs
  -> asset processor
  -> tiled height, surface, render, and physics products
  -> runtime terrain components or subsystem handles
```

Runtime consumes Azoth products. Runtime does not parse legacy terrain files,
walk editor EntityId graphs, or evaluate gradient-component chains on terrain
query hot paths.

## O3DE Model

O3DE terrain is component-composed:

- `TerrainWorldComponent` owns terrain-wide height range and query resolution.
- `TerrainLayerSpawnerComponent` marks shape-bounded terrain areas with layer
  and sub-priority.
- `TerrainHeightGradientListComponent` references ordered gradient entities for
  height.
- `TerrainSurfaceGradientListComponent` maps gradient entities to weighted
  surface tags.
- `ImageGradientComponent` and other GradientSignal components provide image and
  procedural sample sources.
- `TerrainWorldRendererComponent` owns render mesh, detail material, and clipmap
  settings.
- `TerrainPhysicsColliderComponent` maps terrain samples and surface tags into
  physics heightfield data and physics materials.

That split is valuable. It separates the axes that really vary independently.
The problem is using the component graph itself as Azoth's durable source of
truth.

## Why Not Copy Storage

Exact O3DE-style storage makes terrain identity emergent. A region is not a
declared terrain asset; it is an entity that happens to have the right terrain,
shape, and gradient components wired together by entity references.

That creates concrete costs in Azoth:

- Editor UX becomes EntityId graph wiring instead of domain-shaped terrain
  editing. We would end up rebuilding a Landscape Canvas-style graph editor on
  top of generic property inspection.
- Validation becomes mostly cross-entity service validation: spawners need
  shapes, gradient lists need valid gradient providers, transforms need valid
  frames, lists must not cycle, and priorities must resolve deterministically.
- Asset DB dependency tracking becomes harder because invalidation edges are
  hidden inside intra-document entity references instead of typed source asset
  references.
- Cook determinism becomes harder because live gradient sampling, floating point
  order, component order, entity references, and runtime cache behavior all have
  to be pinned after the fact.
- Runtime terrain queries risk inheriting component, bus, and gradient
  indirection in hot paths. Azoth wants cooked, cache-friendly products there.
- Gem boundaries get muddy because canonical terrain source would depend on a
  whole family of GradientSignal, SurfaceData, and graph-authoring component
  schemas even when the source is just imported height and surface maps.

The core issue is source/runtime separation. O3DE lets runtime components double
as source. Azoth should keep source documents, asset DB dependency tracking,
cook products, and runtime loading separate.

## What To Copy

Copy the terrain semantics directly:

- A terrain world has explicit bounds, height range, and query/cook resolution.
- A region has explicit bounds or shape, priority, and deterministic overlap
  behavior.
- Height authority is separate from surface authority.
- Surface data is weighted tags, not just one material id per cell.
- Render material bindings and physics material bindings are separate products
  derived from the same source surface tags.
- Image-backed, baked, constant, and procedural height sources share one source
  abstraction.
- Procedural graphs are authoring inputs that cook to the same product family as
  imported rasters.

Do not copy these O3DE storage details into canonical source:

- EntityId references as source dependencies.
- Component activation order as terrain semantics.
- Runtime gradient resolution as the normal terrain query path.
- Legacy or importer-specific terrain schemas in runtime-facing source.

## Source Shape

The native source should be explicit and content-addressable:

```rust
pub struct TerrainWorldSource {
    pub bounds: Aabb3,
    pub height_range: RangeInclusive<f32>,
    pub cook_resolution: TerrainResolution,
    pub regions: Vec<AssetPathBuf>,
    pub layer_set: AssetPathBuf,
}

pub struct TerrainRegionSource {
    pub name: String,
    pub bounds: TerrainRegionBounds,
    pub priority: i32,
    pub height: TerrainHeightSource,
    pub surface: TerrainSurfaceSource,
    pub transform: TerrainSourceTransform,
}

pub enum TerrainHeightSource {
    Image(AssetPathBuf),
    Baked(AssetPathBuf),
    Generated(AssetPathBuf),
    Constant(f32),
}

pub struct TerrainLayerSetSource {
    pub layers: Vec<TerrainLayerSource>,
    pub default_render_material: AssetPathBuf,
    pub default_physics_material: AssetPathBuf,
}

pub struct TerrainLayerSource {
    pub surface_tag: SurfaceTag,
    pub render_material: Option<AssetPathBuf>,
    pub physics_material: Option<AssetPathBuf>,
    pub blend: TerrainLayerBlend,
}
```

Names are illustrative. The important part is that terrain source references are
typed asset references, not entity references.

## Cooked Products

The cooker resolves all source references and emits domain-native products:

- tiled height data
- tiled surface-weight data
- render clipmap descriptors and material bindings
- physics heightfield data and surface-to-material lookup
- dependency metadata for incremental rebuilds

Cook errors should be hard errors for missing assets, invalid refs, invalid
region overlap rules, unsupported raster formats, and non-deterministic
generator settings. Silent flat-terrain fallback is not acceptable for native
sources.

Runtime components can still exist. They should be thin handles over cooked
terrain products, not the durable authoring format.

## Import Route

Legacy terrain files are importer inputs only. A Lumberyard/O3DE importer
should decode legacy heightmaps, surface maps, material maps, and settings into
native terrain source documents plus source blobs.

```text
legacy terrain set
  -> importer-private parser
  -> TerrainWorldSource / TerrainRegionSource / TerrainLayerSetSource
  -> native terrain products
```

Do not add `azoth.compat.*` schemas for native runtime-facing terrain source.
Importer crates can keep raw evidence files when useful, but those files are not
the editable Azoth terrain model.

## Edge Cases

Runtime-mutable terrain may need an explicit runtime-evaluable source variant or
runtime terrain service. That should be opt-in and still have clear product and
dependency semantics.

O3DE project import may require a faithful O3DE component-graph importer. That
importer should translate into native terrain docs instead of making O3DE
component storage canonical.

If terrain authoring is dominated by external DCC tools such as Gaea, Houdini,
or World Machine, the first path should favor image and baked sources. The
generator graph can remain optional.

If projects create thousands of overlapping regions, the source model may need a
spatial index or region bundle format. That is a scaling representation detail,
not a reason to make EntityId graphs the source format.

## Decision Rules

- Source docs are typed files tracked by the Asset DB and edited through typed
  source sessions.
- Large raster/sample payloads are source blobs, not inline schema fields.
- Source refs are typed asset refs.
- Runtime products are deterministic and rebuildable.
- Runtime hot paths read cooked products.
- Legacy formats are importer inputs, not native source schemas.
