# Viewport compositor repro harness

This tool captures the composed editor window through Windows Graphics Capture
(WGC), not window framebuffer/GDI capture. It sees both the Bevy and GPUI
DirectComposition sibling visuals.

1. Launch the editor with `az-editor --viewport-diagnostic`. The equivalent
   environment binding is `AZOTH_EDITOR_VIEWPORT_DIAGNOSTIC=1`.
2. Start the 20,000-frame rapid-resize gate:

   ```sh
   cargo run --manifest-path crates/editor/viewport-compositor/Cargo.toml --release -- \
     capture --title Aether --fps 120 --frames 20000 \
     --out-dir target/viewport-compositor/resize-dcomp
   ```

   The tool is a member of the root workspace, so Cargo uses the repository's
   root `target/` directory and lockfile.

`capture` runs on Windows only because Windows Graphics Capture is a Windows
API. On any other platform it prints that reason and exits with status 2.
`compare` runs everywhere.

The Bevy diagnostic texture contains a one-pixel magenta border, checker grid,
binary producer frame id, and binary render width/height. Four lime GPUI corner
markers encode the independently current layout rectangle. `capture.json`
classifies every WGC frame as `good`, `scaled_complete`, `stretched`,
`overcopied`, or `uninitialized` and reports `artifact_frames / total_frames`.
`scaled_complete` is the Phase-4 policy state: the last complete producer
surface is temporarily transformed, and its render border still matches the
independently current layout rectangle exactly. It is reported separately and
is not counted as an artifact.
