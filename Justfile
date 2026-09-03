set dotenv-load := true
set unstable := true

mod editor 'just/editor.just'

# List the engine-owned recipes and live editor submodule.
default:
    @just --list --list-submodules

# Slow scaffold regression: generates a standalone project and runs `cargo check -j4` inside it.
scaffold-check:
    cargo test -p az-project-scaffold generated_project_template_compiles_as_standalone_package -- --ignored
