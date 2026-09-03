# ADR 0022 data parity

Phase 4b-1 compares semantic inspector data, not serialized identifiers. The
Phase-0 RON files remain the independent oracle and are not rewritten.

## Normalization

- Field names, order, labels, category, and icon compare exactly.
- Old `schema_type` strings select a semantic family only. `core.vec3`,
  `core.quat`, `core.list<...>`, `core.map<...>`, and legacy schema names are
  normalized to vector, quaternion, list, map, struct, enum, scalar, asset, or
  object families. They are never compared to Bevy `TypePath` strings.
- Old widget spellings normalize as follows: `vecN:*` to vector(N), `quat:*`
  to quaternion, `slider:*` to slider, `number:*` to number, `dropdown` to
  enum, `color` to color, `asset:*` to typed asset, `textarea:*` to multiline,
  and checkbox/toggle to bool. A `default` widget derives its family from the
  reflected kind.
- Numeric values compare after f32-compatible decimal normalization. This
  treats `0.10000000149011612` and typed-RON `0.1` as the same value.
- Legacy numeric struct defaults for vectors/quaternions normalize to ordered
  tuples. `Vec3::ZERO`, `Vec3::ONE`, and `Quat::IDENTITY` therefore compare by
  values, not field IDs.
- Legacy enum/variant IDs normalize to the named registered variant and its
  recursively normalized payload. Empty lists/maps and `null` normalize to
  empty list/map and `Option::None` respectively.
- Range minimum/maximum compare numerically. Length bounds, allowed strings,
  and allowed variants compare against the projected typed field constraints;
  legacy variant IDs normalize to their registered variant names first.
- Reflected paths and commands compare by component `TypePath`, named field or
  variant segments, and tuple/list indices. Map operations additionally
  compare the separately encoded typed key envelope.

## Typed sources

- `sources-typed/component-baseline.prefab.ron` carries the Phase-0 Transform,
  Mesh, Material Assignment, and Camera intent plus directional/point light,
  nested struct/enum/list/string-map, integer-keyed map, asset/object reference,
  scalar, multiline, hidden, and read-only cases.
- `sources-typed/reflected-defaults.prefab.ron` is a typed default-control
  source whose envelopes exercise the normalizer through the same Cap'n Proto
  snapshot path.
- `sources-typed/validation.prefab.ron` carries invalid Camera and Spot Light
  values for named-path diagnostics.
- `sources-typed/nested-override.prefab.ron` carries the same nested instance,
  instance chain, source asset, parent, Set override, and Clear override intent
  as the legacy nested source.

Tests copy these committed files to a temporary source root before opening or
editing them. The committed fixtures are never mutated by the RPC harness.

## Matrix status

Data-level pass now: `scalar_editing`, `slider_range`, `color_vector`,
`enum_variant`, `nested_struct`, `list_editing`, `map_editing`,
`typed_map_keys`, `asset_object_refs`, `undo_redo`, `visibility_read_only`,
`validation`, and `add_component`. The same gate also compares nonempty length,
allowed-string, and allowed-variant constraints, and proves a Clear override
survives snapshot to edit to snapshot beside the Set override. Wire and server
tests cover Set, Clear, Insert, Remove, and Move override operations.

Deferred to Phase 4b-2 with named ignored tests: `multiline_text` (rendering),
`mixed_selection` (selection interaction state), `actions` (button/UI command
wiring), `gamedata` (domain renderer), and `graph_ports` (domain renderer).

The Phase 4a.1 contract extension closes the four frozen-vocabulary gaps:
applicability (including Bevy required components), reflected defaults, the
full override operation set, and typed field constraints. The supported RPC
routing switch remains disabled; this data parity does not cut the editor over.
