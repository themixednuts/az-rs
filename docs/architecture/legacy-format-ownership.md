# Legacy format ownership

## Decision

Azoth owns only reusable legacy format support. Pure codecs live under
`crates/formats/legacy/`. Source transforms and packages that link the asset
builder live under `crates/import/legacy/`. `crates/engine/builders/` registers
the transforms. Canonical runtime products live in `crates/engine/assets/`, and
commands under `tools/` are thin front ends over those libraries.

Legacy codecs are build-time dependencies by default. Runtime crates consume
cooked products. A compatibility gem may parse an original runtime format only
when its manifest lists each package under
`package.metadata.azoth.legacy-runtime-compatibility`. The publication audit
rejects undeclared and stale entries.

## Package layout

Group format crates by the format family that owns the bytes:

- `crates/formats/legacy/cry/` for Cry file and archive formats.
- `crates/formats/legacy/az/` for AZ serialization and asset envelopes.
- `crates/formats/legacy/lumberyard/` for Lumberyard-specific formats that do
  not belong to either lower-level family.

The directory groups packages; it does not create facade crates. Package names
stay specific, such as `cry-chunk` or `az-objectstream`. A package should parse
or write one coherent format family and expose values that do not depend on an
asset processor, editor, renderer, or game.

Group import packages the same way under `crates/import/legacy/`. The import
tree also has a `lua/` family for bytecode conversion. A package belongs in the
import tree if it depends on `az-asset-builder` or `az-gem-contract`, even when
the package still contains reusable parser modules. Extract a parser into the
format tree only when another caller needs the parser without the import layer.

## Dependency direction

```text
tools
  -> engine builders
       -> legacy import adapters
            -> legacy codecs
            -> canonical engine assets

runtime
  -> canonical engine assets
```

The runtime-to-codec edge is forbidden. Builders may depend on codecs and
canonical product types. Codecs may depend on small utility crates, but not on
builders, engine hosts, gems, or tools.

`az-gem-lyshine` is the current exception. It reads original Lua bytecode and
LyShine sprite sidecars because those files are runtime inputs to the
compatibility gem. Its manifest declares both package edges. Texture-atlas
source conversion stays in the builder path and is not a runtime dependency.

## Intake rule

Move code into this repository only when it is engine-neutral and has a clear
owner in the layout above. Bring tests and provenance with it. Do not copy a
second implementation when an equivalent package already exists; merge the
missing behavior into the existing owner and delete the duplicate surface.

Acceptable intake includes reusable chunk parsing, loss-preserving XML,
format-to-canonical conversion, and serialization code generation. Project
schemas, captures, extraction recipes, and content-specific transforms remain
outside the engine.

## Sibling-tool intake decisions

The sibling tool workspace is an input for migration, not a dependency. Do not
probe it from Cargo, build scripts, tests, or runtime code.

| Capability | Engine owner | Disposition |
|---|---|---|
| Loss-preserving XML element tree | `az-xml` | Absorbed into the existing XML owner as `XmlElement` and `parse_tree`; keep format-specific projections in their format crates. |
| Cry chunk container and geometry payloads | `crates/formats/legacy/cry/cry-chunk` | The package supports both public signatures, the 16-byte file header, the 16-byte table entry, and the high-bit endian flag. Synthetic little-endian and big-endian tests cover the 0x746 container and the supported geometry payloads. Project-defined chunk variants and content policy remain excluded. |
| Lumberyard-compatible PAK and AZCS reading | `az-pak` | Already absorbed, including stored, deflate, optional Oodle, AZCS, mmap, extraction, inspection, and search paths. Install discovery and project extraction policy stay outside. |
| AZ ObjectStream | `az-objectstream` | Keep the general codec, query, overlay, and visitation model in the existing owner. Project type projections and extraction schemas stay outside. |
| Source identity sidecars | `az-asset-builder::source_meta` | The engine owner carries the canonical sidecar type, path rule, reader, identity resolution, `uncataloged` constructor, and compact JSON serializer with one trailing newline. Do not create a writer-only duplicate. |
| Bounded blocking jobs and cancellation | `az-jobs` | Already absorbed as the bounded producer-worker-consumer pipeline and cancellation primitives. Tool commands should call this package instead of carrying a second scheduler. |
| Safe paths and atomic publication | `az-filesystem` | The existing owner already provides virtual-path validation, source-path normalization, transactions, and atomic replacement. Merge missing behavior there instead of adding a tool utility crate. |
| Cry mesh to glTF conversion | `crates/import/legacy/cry/cry-to-gltf` over `cry-chunk` | The deterministic GLB 2.0 converter handles nodes, geometry streams, subsets, materials, and compiled-bone skinning. Conversion tests cover empty meshes, optional normals, doubled weight streams, and subset-local remapping. Project asset graphs, reflected project types, and export layout stay outside. |
| LmbrCentral tag projection | `crates/import/legacy/lumberyard/lmbr-central-assets` | Read the ordinary Lumberyard `TagComponent` from ObjectStream into the existing runtime-owned type. Keep project tag components and project source schemas outside the engine. |
| SerializeContext compiler core | a future `az-serialize-codegen` package | Deferred outside the public-release intake. The reusable boundary is the JSON/schema parser, semantic model and catalog, diagnostics, dependency and layout analysis, and neutral Rust emission. Extract it only after protocol emitters, generated project type maps, component scaffolding, native-analysis evidence, and Go and TypeScript backends are caller-owned. No `az-rs` build depends on the sibling package. |
| Exporter-root artifact publication | project or offline tool | Do not merge this with `azpack`. The sibling writer chooses case-folded paths and hash-suffixed collision names for one export workflow; that policy is not an engine package format. |
| Physics, character, animation, Mannequin, NvCloth, and LmbrCentral adapters | existing format packages | Compare tests and merge missing reusable behavior into the existing owner; do not import duplicate packages. |
| Asset catalogs, install discovery, game data, scenes, terrain, vegetation, localization, and project resources | project workspace | Reject from engine intake. These packages encode project schemas, content paths, or extraction policy. |

The Cry chunk package is the only new codec created by this inventory. The XML,
archive, ObjectStream, source-metadata, jobs, filesystem, and package-writing
capabilities already have engine owners. The accepted Cry geometry conversion
now lives behind those boundaries rather than in a top-level tool.
Project-specific payloads and real content fixtures are not part of the engine
package.

## Migration gate

A legacy package is correctly placed when all of these are true:

1. Its input format and output type have one documented owner.
2. `cargo tree` shows no runtime package depending on the legacy codec.
3. Builders and tools call the same library implementation.
4. Tests use synthetic or redistributable fixtures with recorded provenance.
5. Package metadata and public documentation contain no machine-local paths.
