# Domain language

This context names the engine/editor concepts used by Azoth. It exists so the
workspace stays focused on reusable engine infrastructure while individual games
live in project workspaces.

## Language

**Engine Workspace**:
The repository that owns Azoth engine crates, editor crates, reusable gems,
project workflow tooling, IPC schemas, runtime hosts, native formats, packaging,
and templates.
_Avoid_: game project, content repo, title-specific workspace

**Project Workspace**:
A game or application repository opened by the editor through project workflow.
It owns project manifests, authored assets, project gems, source roots, product
caches, services, tools, and release-specific data.
_Avoid_: engine subfolder, editor build, hardcoded project path

**Editor App**:
The standalone Azoth editor executable. It is built once as an engine/editor app
and connects to project services instead of being rebuilt per project.
_Avoid_: per-project editor binary, game runtime, webview shell

**Project Host**:
The project-owned sidecar service started by editor workflow for a specific
project/workspace/session. It exposes project catalogs, asset/source operations,
Bevy type-registry projections, typed source sessions, graph documents,
builders, runtime launch descriptors, and project-specific validation over the
editor IPC seam.
_Avoid_: in-process editor plugin, global daemon, file scanner fallback

**Workspace**:
A checkout Azoth can process assets for, identified by project id, canonical
workspace root, and branch. The project root is a workspace; so is any Lore
instance the user cloned themselves. Azoth discovers workspaces; it never
creates, clones, moves, or deletes one. Workspace identity keys the asset DB
view, so every session over the same checkout and branch shares one view
(Linear ADR 0039).
_Avoid_: git worktree, session-owned clone, temp copy, editor-only scratch dir

**Session**:
A named, durable supervision scope over a workspace. It owns service
generations, descriptors, process records, readiness files, capability grants,
leases, logs, and preservation state — and nothing else. It holds no branch,
base ref, or owned path, performs no commit/merge/publish, and may delete only
its own run directory under `~/.azoth`.
_Avoid_: worktree, branch, private checkout, VCS wrapper, anything that
commits, merges, or publishes

**Attached Editor Controller Set (`EditorControllers`)**:
The editor-owned, typed aggregate for one attached session generation. Its
named slots own effectful controller handles plus their tasks, subscriptions,
cancellation, readiness, and failures. Reattachment replaces the whole set;
same-session retry replaces one failed slot. Passive mode state and projection
publication belong to `ModeProjectionRegistry`, which composes through typed
controller outputs rather than sharing lifecycle ownership.
_Avoid_: per-domain controller globals, `Any`/`TypeId` service locator,
descriptor-order dependency, parallel controller-status registry, projection
publisher inside a controller descriptor

**Gem**:
A plugin-style Rust crate or crate family that registers engine, editor,
runtime, asset-builder, graph, schema, or project capabilities through explicit
traits and descriptors.
_Avoid_: hardcoded asset kind table, ad hoc registry, hidden convention

**Engine Gem**:
A reusable gem that belongs in the engine workspace because its capabilities are
not tied to one project.
_Avoid_: game-specific crate, imported release adapter

**Project Gem**:
A project-owned gem that registers game-specific data types, tools, source
formats, graph nodes, runtime systems, services, and builders through the same
engine contracts.
_Avoid_: engine crate, editor fork, path-specific special case

**Asset DB**:
The durable project authority for workspace views, source roots, asset identity,
path history, job attempts, products, and source-session recovery metadata.
Saved typed source files remain authoring authority. Exactly one process — the
project instance's `asset-processor` — opens it; every other service reaches it
through that service boundary.
_Avoid_: disposable cache, runtime catalog, folder scan, per-session database
owner

**Asset Catalog**:
The runtime product inventory for a built release. It maps asset IDs, paths,
types, product formats, hashes, sizes, dependencies, and loader facts to built
products.
_Avoid_: asset registry DB, source manifest, compatibility catalog, folder scan

**Source Asset**:
The editable typed file or source set that is the saved authoring authority
before a build. Project-host may retain an unsaved recovery snapshot of the same
typed value, but that snapshot is not another source model.
_Avoid_: product cache entry, cooked blob, imported evidence

**Source Session**:
A project-host-owned `SourceSession<T>` for one open typed source file. It owns
the current typed value, source revision, dirty state, undo/redo, validation,
atomic save, and crash recovery without wrapping `T` in a generic authored
document/value tree.
_Avoid_: AuthoredDocument, DB-over-file authority, editor-private value model

**Typed Format Descriptor**:
A Rust type that owns an asset-pipeline source schema or product byte-format
identity through `SourceFormat` or `ProductFormat` associated constants.
Registries, DB rows, RPC messages, and product manifests still carry string or
UUID wire values, but builder and registration authoring derives those values
from descriptor types instead of hand-written call-site strings.
_Avoid_: raw format string, extension table, compatibility fallback

**GameData Table Source**:
An editable GameData source asset containing a typed row set, usually a
top-level RON `Vec<Row>` selected by source-path routing and a table marker.
Editor reflection plus GameData descriptors wrap this source with table/grid semantics, validation,
row identity, foreign-key metadata, templates, and compile inputs; it does not
turn the serialized file into a prefab-style authored document wrapper. The
default template for an empty file is `[]`, and a source such as
`gamedata/items/master.ron` builds to a catalog table product such as
`tables/items/master.aztbl`. A table source's product depends on registered
foreign-key target sources; unknown table routes or missing foreign-key sources
are build errors. The GameData gem registers this source as a
creatable/editable typed file workflow; initial bytes for new table
files come from source-file template registrations, and project/gem build
rules claim the sources through type/path-filtered dispatch.
_Avoid_: object-rooted RON wrapper, table header in RON, project-specific table registry,
implicit foreign-key source discovery

**GameData Row Type**:
The Rust struct that owns an authored GameData row shape. It derives `Reflect`,
normal serialization traits, and `GameDataRow`; the primary key is declared on
the field with a row attribute. Editor structure comes from the composed Bevy
type registry while table/column identity remains GameData-specific.
_Avoid_: separate RowType marker, hand-maintained field selector type, manual field ids,
column-level identity collapsed into untyped row metadata

**GameData Column**:
A typed accessor for one field of a GameData row. It owns the authored field
name, physical source column name, row-key flag, required/optional contract, and
any per-column foreign-key or typed-list metadata. Required cells surface as
fallible values; optional cells surface as `Option`. Foreign keys target a
typed key column, not a string table name, and physical foreign-key columns are
distinct from authored `RowRef` values.
_Avoid_: row-level foreign-key metadata, one accessor shape for required and
optional cells, stringly foreign-key target names

**GameData Table Marker**:
A zero-sized Rust marker for one concrete GameData table source/product. It
names the table, points at a `GameData Row Type`, and carries the asset-pipeline
table source route; it does not own the row schema shape.
_Avoid_: duplicated row struct, global table registry entry in source RON, schema owner

**GameData Table Family**:
A named set of `GameData Table Marker`s that share one `GameData Row Type`.
It is the domain of `RowFamilyRef<Row>` and table-family manager views. Family
resolution must declare how duplicate primary keys across member tables behave,
for example error, first wins, overwrite/last wins, or multi-value collection.
_Avoid_: implicit base/override merge, silent last-wins, hidden canonical table
on the row type

**GameData Row Reference**:
An authored exact reference to a row in one concrete table marker, written as
`RowRef<Table>`. The source stores the row type's primary-key value as a bare
value; builders validate it against the target table and lower it through the
target table or manager policy. Lowering may normalize the key first (trim,
lowercase, remove-space-and-tab, reject zero CRC), may resolve duplicates under
an explicit duplicate-key policy, and may produce the target key, a CRC key, or
a resolved table-local row handle depending on the projection. Optional and list
forms are explicit with `Option<RowRef<_>>` or collection types.
_Avoid_: stringly `#[gamedata(ref = "...")]`, family-wide lookup by default,
persisted RowIndex, assuming the stored key value is already the runtime lookup
key, assuming every runtime manager uses one key kind

**GameData Row Family Reference**:
An explicit authored reference to any table source sharing one row type, written
as `RowFamilyRef<Row>`. Because compiled row indexes are table-local, a family
reference lowers to a table-tagged handle: a table discriminant plus a
table-local row index or key under an explicit family resolver/lowering policy.
It never silently lowers to a bare `RowIndex`.
_Avoid_: implicit cross-table lookup, canonical primary table hidden on the row type,
lowering that drops the concrete table discriminant

**GameData Localized Field**:
A display field whose authored value is a localization tag. Runtime display text
comes from a validated localization dependency, not from an ad hoc string map or
an authored display string baked into the table row.
_Avoid_: storing resolved display strings in rows, local string maps beside a
manager, treating a loc tag as already-localized user-facing text

**GameData Manager**:
A project/gem-registered editor and build workflow projection over one domain
slice of GameData. A manager groups one or more `GameData Table Marker`s and
may require typed product assets (ObjectStream, XML, binary, or another product
format), other managers, related prefab/static source schemas, joins, row
templates, validation rules, import/build policy, and editor views/actions. Its
descriptor can declare manager-to-manager dependency edges for authoring
validation and build/readiness ordering without dictating the runtime resource
code shape. The manager behavior/descriptor references registered Rust row and
config `TypePath`s but is not itself a row or table structural schema.
_Avoid_: deriving a second reflection schema for manager behavior, global table registry object,
runtime DataManager port, hidden canonical table, root manager trait, fallback
row API, single-associated-table base abstraction

**GameData Provider**:
A resolved editor/build catalog node that provides a table or table-family
projection to other managers. A provider can be synthesized as a read-only
single-table/table-family default from registered table descriptors, or claimed
by an explicit `GameData Manager Shape` when custom projection policy is needed.
Provider dependencies target the data provision rather than a concrete manager
name so dependents survive moving between an automatic provider and an explicit
manager.
_Avoid_: generated source file as source authority, mutable single-table
manager, dependency on a historical runtime manager name

**GameData Runtime Table Provider**:
A borrowed runtime view over one strict cooked table product. It may be given a
project-facing compatibility name such as `*DataManager` through a thin wrapper
or macro, but that name only validates/reads already-built table products. It
does not define editor/build manager policy; real transforms, joins, indexes,
asset inputs, or dependency behavior belong in `GameData Manager Shape` and
project projection code.
_Avoid_: treating compatibility resource names as source authority, Bevy-only
engine contracts, generating mutable table managers for simple table reads

**GameData Manager Shape**:
The structural definition of a GameData manager projection. It names the shape
archetype (single-table index, table-family index, CRC/enum/numeric/string key
projection, partitioned projection, fallback projection, row projection,
product-asset resource, or composed resource), key policy (target key kind,
ordered normalization transforms, zero-CRC rejection, optional stored key
text), duplicate-key policy, row filters, default/exclude overlays, projection
transforms, secondary indexes (each with its own key kind, storage, and
duplicate-key policy), dependency lookups, source-row/source-handle
affordances, and diagnostics. The projected semantic row can differ from the physical row type in
units, optionality, key representation, and derived fields.
_Avoid_: treating manager projected fields as row-type fields, putting manager
projection transforms on the authored row schema, one universal manager key type,
service-dependent runtime methods exposed as fake table-only editor APIs

**GameData Source Transform**:
Build/manager-side logic that filters rows, applies default or exclude overlays,
splits token columns, resolves delimited lists or operation strings, derives
fields, normalizes keys, or lowers foreign keys. Source transforms are projection
logic over authored rows; they are not the row schema and do not change the
serialized source shape. In descriptors this is not one struct: it is composed
from the manager shape's row filters (inclusion predicates over authored rows)
plus projection transforms (per-field mappings from source columns), declared
separately so the editor can diagnose each.
_Avoid_: derived data stored as authored row fields, token parsing in hot runtime
paths, hidden manager filters that the editor cannot diagnose

**GameData Manager Config**:
An optional authored or project-persisted data struct for manager settings, such
as default filters, import policy, view layout, validation severity, or preload
requirements. This struct uses normal Rust serialization and Bevy reflection
when users edit it through the editor; per-user UI state remains editor
preference data, not GameData source authority.
_Avoid_: baking UI state into table rows, making every manager have a config
file, stringly manager settings

**Product Asset**:
A validated build output emitted by asset builders and addressed by the Asset
Catalog.
_Avoid_: editable source, staging output, DB row as file

**Product Cache**:
The dev-time cache of built products for a project/session/platform. It mirrors
the Lumberyard/O3DE role of `Cache/<platform>` products, but it is not the
Asset DB and not source authority.
_Avoid_: registry DB, shipping package, source tree

**Packaged Release**:
A shipping/installable output built from validated product rows, product bytes,
compression policy, packaging policy, and the Asset Catalog.
_Avoid_: dev cache, source project, one-off pak dump

**Native Format**:
An Azoth-owned source or product format with a stable magic, separate header
version field, explicit endianness, and streamable/parallelizable layout where
the domain needs it.
_Avoid_: legacy extension rename, version in magic, opaque passthrough

**Compatibility Adapter**:
A trait-backed bridge that imports, reads, validates, or packages legacy or
third-party formats into Azoth source/products without making those formats the
native runtime path.
_Avoid_: legacy fallback, permanent runtime bypass, hardcoded project path

**Visual Graph Document**:
The editor-owned authoring document for a node graph. It stores nodes, ports,
values, links, placement, and route anchors, but it is not the hot runtime
execution model.
_Avoid_: runtime VM state, Rust type mutation, UI-only state

**Node Type Catalog**:
The project-host-published descriptor catalog for graph node types, ports,
editable values, validation, and palette metadata. Rust/project/gem code
publishes descriptors; editor graph edits mutate documents, not Rust types.
_Avoid_: runtime dispatch table, hardcoded editor palette, graph source file

**Graph Compiler Backend**:
The registered compiler for a graph domain. It lowers authored graph documents
into domain-native runtime products such as generated Rust, shader bytecode,
pipeline metadata, compact state-machine tables, or other optimized artifacts.
_Avoid_: universal interpreted gameplay VM, reflective hot path

**Runtime Graph Product**:
The optimized product loaded by runtime systems after graph compilation. Hot
graph categories must avoid reflective lookup, schema walking, DB reads, and
editor graph traversal in their per-frame path.
_Avoid_: authoring graph, Cap'n Proto message, inspector schema

**Type Registry Projection**:
The project-host's process-safe projection of the composed Bevy
`AppTypeRegistry`, including type structure, `TypePath` identity, typed editor
attributes, documentation, validation results, and values needed by the
universal editor. It extends missing Bevy Remote projections without becoming a
second type authority.
_Avoid_: AzSchema catalog, editor-loaded project code, handwritten per-panel field plumbing

**Prefab Document**:
The engine-owned typed source model saved as `*.prefab.ron`. It stores stable
entity aliases, hierarchy, registered reflected component templates, nested
Prefab instances, and sparse semantic overrides; it is not a generic authored
value graph or cooked runtime scene.
_Avoid_: AuthoredDocument, project-owned duplicate Prefab, runtime Spawnable, ObjectStream dump

**Prefab Instance**:
A stable authored record inside a Prefab Document that references another
Prefab source by typed asset reference and records semantic overrides against
that source. Instance ids are stable authoring identity and are distinct from
document ids, object ids, asset ids, and runtime instance ids.
_Avoid_: duplicated entity copy, runtime entity, asset identity

**Prefab Semantics Module**:
The engine module that validates and transforms Prefab Documents beyond
field-level Bevy reflection: type/tag resolution, migrations, instance graph
resolution, cycle checks, override target resolution, dependency extraction,
in-memory expansion, and temporary typed-world construction for preview/cook.
_Avoid_: project-host special case, UI widget, runtime reflection surface

**AZSCENE Product**:
The sole cooked Prefab runtime product: an engine-owned `AZSCENE\0`
`*.scn.bin` containing a Bevy `DynamicWorld`, its deterministic type records,
asset dependencies, and entity-remapping data. Runtime loads and materializes
it without Prefab, override, RON, or ObjectStream interpretation.
_Avoid_: Spawnable, AZSPWN, root-spawnable manifest, editable scene source
