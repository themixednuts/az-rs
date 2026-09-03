# Azoth Engine and Editor

Azoth is an engine and editor workspace. Projects remain separate and integrate
through manifests, gems, services, the asset registry, and generated runtime
products.

## Development

```sh
cargo metadata --no-deps --format-version 1
cargo check -p azoth -j4
cargo check -p az-editor -j4
```

Oodle Data compression is optional. The [Oodle SDK setup](vendor/oodle/README.md)
is the setup guide for every platform, and it also covers the Network and
Texture SDK pieces for future integrations.

The root Justfile exposes only live engine/editor workflows:

```sh
just --list --list-submodules
just editor::run
just editor::run <project-path> <session> <profile>
just editor::warm-services release
```

Editor recipe parameters are `<PROJECT> <SESSION> <PROFILE>`; the profile
defaults from `AZOTH_EDITOR_PROFILE` (`dev` when unset).

## Architecture references

Lumberyard and O3DE are the approved upstream architecture references. Use
repository-relative paths when citing their source.
