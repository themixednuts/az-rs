set dotenv-load := true
set unstable := true
set working-directory := '..'

# Run the AZoth editor.
#
# The editor needs two workspace binaries at runtime: `az-editor` (the UI) and
# `azd` (the per-project daemon it spawns). The per-project engine services
# (project-host, asset-processor, runtime-host, sessiond) are built on demand by
# `azd` into the project's normal cargo target directory, so they are NOT built
# here; `warm-services` just primes the shared dep cache to make that first
# build fast.
#
# Cargo links the `dev` profile into `target/debug` and every other profile into
# a directory named after it, so each recipe that runs a built binary resolves
# `target / (if build_profile == 'dev' { 'debug' } else { build_profile })`.
# That mapping cannot become a variable: a just variable may not read a recipe
# parameter, and `build_profile` is one.

profile := env_var_or_default('AZOTH_EDITOR_PROFILE', 'dev')
default-session := env_var_or_default('AZOTH_SESSION', 'main')
exe := if os() == 'windows' { '.exe' } else { '' }

shutdown-reason := 'just editor build'
shutdown-timeout-ms := '30000'

# Ask running project services and editor daemon to shut down before Cargo overwrites Windows binaries.
#
# `azoth_bin` is a local binding, not an input. Build the CLI once and invoke
# the produced binary: `cargo run` per stop call would re-check (and on a stale
# target dir, recompile) the whole CLI N+1 times before the real build starts.
[private]
stop-services path='' session=default-session build_profile=profile azoth_bin=('target' / (if build_profile == 'dev' { 'debug' } else { build_profile }) / 'azoth' + exe):
    cargo build --profile {{ build_profile }} -p azoth --bin azoth
    -{{ if path == '' { 'echo "no editor project given; sweeping every workspace session supervisor"' } else { azoth_bin + ' session services stop --project ' + quote(path) + ' ' + quote(session) + ' --reason ' + quote(shutdown-reason) + ' --service-shutdown-timeout-ms ' + shutdown-timeout-ms } }}
    {{ azoth_bin }} session sweep --reason {{ quote(shutdown-reason) }} --service-shutdown-timeout-ms {{ shutdown-timeout-ms }}
    -{{ azoth_bin }} daemon stop --reason {{ quote(shutdown-reason) }}

# Build the editor, daemon, and session supervisor binaries in lockstep (azd spawns az-sessiond from this target dir).
build path='' session=default-session build_profile=profile: (stop-services path session build_profile)
    cargo build --profile {{ build_profile }} -p az-editor --bin az-editor -p az-daemon --bin azd -p az-sessiond --bin az-sessiond

# Run the editor; no PATH opens the project manager, while PATH opens that project.
run path='' session=default-session build_profile=profile: (build path session build_profile)
    {{ 'target' / (if build_profile == 'dev' { 'debug' } else { build_profile }) / 'az-editor' + exe }} {{ if path == '' { '' } else { quote(path) } }}

# Run the editor on $AZOTH_PROJECT (set it in .env).
open build_profile=profile: (run env_var_or_default('AZOTH_PROJECT', '') default-session build_profile)

# Pre-compile the per-project services once to warm the first project open.
warm-services build_profile=profile:
    cargo build --profile {{ build_profile }} -p az-sessiond -p az-project-host -p az-runtime-host -p az-asset-processor
