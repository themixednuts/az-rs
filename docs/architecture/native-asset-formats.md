# Native Asset Formats

This note records the asset-format split behind Linear ADRs 0005, 0006, and
0022.
It replaces the older import-era idea that an offline importer should directly
create final runtime products.

## Domains

Azoth asset data has four separate domains:

```text
legacy input -> source/dev authority -> dev product cache -> release package
```

| Domain | Owner | Examples | Authority |
| --- | --- | --- | --- |
| Legacy input | import tools only | ObjectStream, Cry chunks, Lumberyard/O3DE XML and packages | read-only source material |
| Source/dev | project workspace files plus project-host sessions | typed RON, source blobs, imported native source files | saved file authority; typed unsaved session state |
| Dev product cache | asset-processor/build workflow | `~/.azoth/projects/<project-key>/Cache/<platform>/...` | rebuildable processed products |
| Release package | `azoth build` package workflow | `AZPKGM` build receipt, native `azpack` index/chunks, loose payload tree, CryPak-compatible `.pak` backend, later install layout | immutable shipped projection |

The asset DB is durable authority for source roots, asset identity, path
history, job attempts, product lineage, and crash-recovery/session metadata. A
saved typed source file remains source authority; a DB recovery snapshot is not
a second authored value model. The DB is not a disposable cache and it is not
the shipped package format.

The product cache follows the Lumberyard/O3DE platform sublayout under
Azoth's user-level project state:
`~/.azoth/projects/<project-key>/Cache/<asset-platform>`. It is
rebuildable from the source authority and job state. `azoth new` and
`azoth init` ignore legacy `Cache/` trees but do not expose `paths.cache`
and do not create repo-local cache directories.

## Import Policy

An offline converter is a legacy-data importer into
Azoth source/dev formats. It should:

- read legacy paks and legacy object formats;
- write normalized source/dev payloads under the chosen import root;
- register source roots, asset identity, content hashes, path history,
  and worktree entries in the import asset DB;
- attach a registered source format/type identity when the importer can infer
  one.

It should not:

- write final Azoth runtime product binaries in the normal transform
  path;
- create product rows as if it had run the project asset pipeline;
- choose release package layout, Oodle settings, signing/encryption, or
  runtime AssetCatalog policy.

Those later choices belong to `azoth build` or the editor/session build
workflow.

## Source Types vs Product Types

Source/dev format and runtime product format are separate type axes.

Texture sources follow the same split: editable pixels are PNG/EXR files and
alpha is part of those image pixels. Legacy attached-alpha inputs such as Cry
`.dds.a` are importer grouping details only; runtime products are rebuilt from
the PNG/EXR source plus `.texture.ron` settings.

Source identity records may carry a registered source format/type identity, for
example a typed Prefab, material, or scene-import descriptor. Legacy
scene/model/animation formats are importer inputs only; they do not become
generic authored documents.
RASC/RAOC imports may still be kept as compatibility support/evidence
inputs because those files are not authored source and not the native runtime
catalog. The source-format registry is also the editor workflow registry: a
typed file registration owns accepted extensions, default asset-tree placement,
create/edit/import capabilities, and its initial template. Legacy/raw formats
normally disable creation until the owning crate models a native authoring
path. The GameData gem registers a typed table source workflow; a project
GameData crate claims those sources with type/path-filtered build rules and
emits native `AZTBL` products. The asset processor passes that source identity through
`CreateJobsRequest`, leased job attempts, and `ProcessJobRequest` so
builders can decide which jobs apply to the source file.
Registered builders can declare source-type filters in addition to
path patterns. Dispatch is not a first-path-match rule; every builder
whose path pattern and source-type filter match receives `CreateJobs`.
That lets project/gem `.ron` builders stay schema-specific while binary
or imported compatibility formats remain path-driven.
Compatibility builders are still product builders: for example, the New
World datasheet builder consumes legacy `.datasheet` source evidence and
emits native `AZTBL` GameData table products under `tables/**/*.aztbl`.
It does not copy legacy datasheet bytes into the product cache and it
does not create a separate GameData catalog beside `assetcatalog.bin`.
Scene/model/animation import follows the companion route in
[`scene-gltf-blender-animation-route.md`](scene-gltf-blender-animation-route.md):
`glTF`/`GLB` is a tracked interchange blob, the editable Azoth source is a
small schema-backed sidecar, and runtime consumes only Azoth products.
Prefab source is direct typed `*.prefab.ron`; `az-prefab-builder` resolves its
complete composition graph into one AZSCENE `*.scn.bin` `DynamicWorld` product.
There is no Spawnable or scene-manifest intermediate product. Terrain import follows the companion route in
[`terrain-source-route.md`](terrain-source-route.md): O3DE is the reference for
terrain decomposition, but native Azoth terrain source is explicit schema-backed
documents plus source blobs that cook to runtime terrain products.

Runtime products carry an `AssetId` (`asset_guid` plus `sub_id`),
`asset_type`, product path, content hash, byte length, builder
id/version, job key, and platform. Product `asset_type` is not the
source schema type and is not the asset id. It is the runtime/catalog
type emitted by a builder for a specific job.

The current release/product-catalog input is DB-derived: asset-processor
answers `currentProducts` for a worktree/session and platform.
That projection chooses products from the latest relevant successful
attempt and excludes deleted, conflicted, failed, pending, or stale
worktree state before package tooling validates cache bytes. The
command-line diagnostic for this package input is `azoth session
current-products --platform <name>`, which reads through the
asset-processor service rather than opening the asset DB. `azoth build
--profile <profile> --session <name>` uses the resolved package
profile's asset platform to read the same projection after Cargo build
targets finish, then writes a deterministic `AZPKGM` package manifest
under `target/azoth/packages/<profile>/<session>/package-manifest.azpkg`.
That manifest records package policy plus the product rows to package.
It is a build receipt/input, not a second runtime catalog. The payload
writer validates project-state `Cache/<platform>` bytes against it before writing the
selected backend, then `azoth build` emits the single runtime
`assetcatalog.bin` for that package output from the same validated
manifest rows. The native package release id is a BLAKE3 fingerprint of
the stable `AZPKGM` manifest bytes. `azoth build` prints it as
`package_release_id`, and `az-sessiond` carries it on runtime package
roots so launch, cache invalidation, and later GameRelease-facing UI can
name the exact built projection without introducing a second release
manifest or catalog.

The default release backend is native `azpack`: a small
`package.azpack.index` (`AZPKIDX` magic plus a separate v1 header
version) and content-addressed `.azchunk` files under `chunks/<prefix>/`.
`az-asset::azpack` owns the typed index ABI and reader/writer API.
`az-asset::package_payload` owns payload production from validated cache
bytes. Products are split into independently compressed chunks; the
index records product paths, runtime `AssetId` (`asset_guid`/`sub_id`),
`asset_type`, hashes, sizes, compression method, and chunk paths; and
unreferenced chunk files are pruned after a successful index write. This
makes game startup
streamable by index lookup and independent chunk reads, makes
compression/decompression parallelizable, and lets patch delivery send
only the changed chunk files plus the small index instead of a rewritten
monolithic archive range.

Runtime mounting starts with `AzPackReader::open(<package-root>)`. The
reader loads `package.azpack.index`, builds product-path and
`AssetId` lookup maps, safe-joins chunk paths under the package root,
verifies encoded chunk hashes, decodes stored or Oodle chunks, verifies
raw chunk hashes, and finally verifies the assembled product hash. Later
Bevy/engine asset readers wrap this reader rather than parsing package
files directly. `az-framework::asset::AssetCatalog::open_native` opens
native `assetcatalog.bin`; `open_compatibility` is the explicit
Lumberyard/O3DE RASC/RAOC compatibility path. There is no implicit
native-to-compatibility catalog fallback.

Runtime launch snapshots carry package mounts separately from DB-owned
source roots. Source roots prove project/worktree/gem provenance for the
session. Package roots name built runtime inputs:
`target/azoth/packages/<profile>/<session>/<container>` for loose and
native `azpack`, or the output root plus pak payload/catalog paths for a
compatible pak backend. `az-sessiond` reports only package outputs with
a valid `package-manifest.azpkg` plus the expected `assetcatalog.bin` and
payload/index files, and each reported root carries the package
release id derived from the manifest; editor and CLI runtime launch
consume that Cap'n Proto session boundary instead of parsing package
outputs locally.
Runtime asset readers use the reported container to select the payload
backend explicitly via `AssetCatalogPlugin::try_native_package(...)` or
the equivalent reader constructor. Native `azpack` roots must have both
`assetcatalog.bin` and `package.azpack.index`; loose roots use the
catalog plus filesystem payloads; compatible pak roots use the reported
pak `payload_path` and the shared mmap/Oodle reader. A missing backend
file is a launch/root error, not an invitation to fall back to scanning
the output directory. A half-written package output is an invalid launch
input, while an editable dev session may still have no package roots
until the build workflow has produced them.

The `pak` backend remains a peer backend for Lumberyard/O3DE/CryPak
compatibility, not the native Azoth default and not a "legacy" branch in
the package code. Manifest strings are parsed once into
`PackagePayloadPolicy`; typed writers implement the shared
`PackagePayloadWriter` abstraction for `loose`, native `azpack`, and
compatible `pak`, with marker traits and `PackageBackendCapabilities`
describing what each backend can do. `AzPackContainer` is
chunk-addressable and patch-friendly, while `PakContainer` is a
compatibility backend with parallel entry preparation.
`az-asset::package_payload_layout` is the authoritative layout contract
for all three backends: build code, session discovery, and runtime launch
preflight use the same mount root, payload path, and catalog path
calculation instead of reconstructing paths independently.
Unsupported policy pairs are rejected at the project manifest, daemon
plan, package manifest, package writer/layout, and session discovery
boundaries; for example, `loose` products can be copied with `none`
compression, but `loose` plus `oodle` is not a valid package backend
because there is no single compressed payload container to address at
runtime.
Compatible pak entries keep the ZIP local-header and
central-directory shape, using method `0` for stored bytes or method
`15` for raw Oodle streams. Runtime reads parse the central directory
once, memory-map the pak, and decompress by entry slice, so unrelated
asset loads do not contend on a single seek handle. Pak archives are
written with ZIP64 metadata when ZIP32 size/count/offset fields overflow,
but remain less patch-friendly than `azpack` because central-directory
offsets and archive byte ranges can shift when earlier entries change.

## Native Header Rule

Native Azoth product binaries keep format family identity and version
separate.

The magic is a stable fixed-width family tag. It does not include a
version digit. The version is a separate header field, `u32` by default
unless a specific format proves a narrower field is worth the cost.

```text
[8] magic      ASCII family id, for example AZSCENE\0
[4] version    u32 little-endian layout version
... family-specific header/payload ...
```

Current family tags follow that rule:

| Product | Magic |
| --- | --- |
| AssetCatalog | `AZCATAL\0` |
| Package manifest | `AZPKGM\0\0` |
| Package chunk index | `AZPKIDX\0` |
| GameData table | `AZTBL\0\0\0` |
| Scene/entity product | `AZSCENE\0` |
| Material | `AZMATRL\0` |
| Material override | `AZMTLOV\0` |
| UI canvas | `AZUICAN\0` |
| Vegetation distribution | `AZVEGD\0\0` |
| Vertex shape | `AZVSHAP\0` |
| Legacy terrain world | `AZTRWLD\0` |
| Legacy terrain region | `AZTRGN\0\0` |
| Wwise controls | `AZWWCTL\0` |
| Texture atlas | `AZTEXAT\0` |
| Rock shape | `AZRNRSH\0` |

Readers should reject old version-in-magic spellings such as
`AZCATAL1` instead of treating them as compatible aliases. Before a
format ships, incompatible layout changes can bump the header version
and regenerate the product cache. After a format ships, compatibility
policy belongs in that reader/writer family, not in a second magic.

## Package Profiles

Package/build profiles live in `azoth.toml` and are snapshotted into
`azoth.lock`. They name:

- asset platform;
- Cargo profile;
- package container backend (`loose`, native `azpack`, or compatible `pak`);
- compression family;
- Oodle compressor and effort when Oodle is selected;
- future signing/encryption and package-layout policy.

`azoth new` and `azoth init` create `pc-dev` as a loose uncompressed
debug profile and `pc-release` as native `azpack` plus Oodle
Kraken/normal. `azd` resolves `azoth build --profile <name>` against
these package profiles first and returns the resolved policy beside the
Cargo command plan. That keeps user-visible build and package choices in
reviewable project configuration while leaving source documents and
import output format-neutral.
