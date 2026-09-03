# Project Instructions

## Git safety (agents)

**Never run `git checkout` in this repo** — including `git checkout -- <path>`,
`git checkout .`, branch switches to read old files, or any other `checkout`
subcommand. It has discarded large uncommitted working-tree changes during
agent recovery.

Also forbidden unless the user explicitly requests that exact command:

- `git restore`
- `git stash` / `git stash push`
- `git reset --hard` / `git reset --merge`
- `git clean -fd`
- `git push --force` (especially to `main` / `master`)

**Safe alternatives when you need prior file content:**

- `git show <rev>:path/to/file` (read-only; does not touch the working tree)
- `git diff path` / `git log -1 -- path` for context
- Edit the working tree in place with the editor tools (`Read` / `StrReplace` / `Write`)
- Ask the user before any operation that could drop uncommitted changes

If a file looks corrupted or truncated, stop and report it; do not "fix" it by
checking out an older revision.

## Shared-tree coordination

Use the existing main working tree and the repository's default root Cargo
target. Concurrent tasks must exchange exact file leases and receive explicit
approval before editing or expanding a lease. Release the lease when the
focused checks finish.

Do not create a task branch, worktree, clone, alternate `CARGO_TARGET_DIR`, or
nested `target/` directory for coordinated work in this repository.

## Workspace boundary

This repository is the Azoth engine/editor workspace. Keep it focused on:

- reusable engine crates, framework crates, and engine gems
- the standalone editor app and project-host/session tooling
- project templates and project workflow plumbing
- reusable asset formats, packaging, graph, reflection, runtime-loading, and IPC/ABI surfaces
- generic compatibility adapters when they are useful across projects

Project-specific game code, data, captures, reverse-engineering notes, release
evidence, and one-off import output belong in the owning project workspace. Do
not add project paths, resources, protocol notes, or one-off tools here. Promote
shared behavior into a reusable engine abstraction with project-neutral tests.

Engine/editor code must discover projects through manifests, lockfiles, service
inventory, gems, and explicit project workflow inputs. Do not hardcode local
project paths or add parallel lookup paths.

## Architecture references

Use Lumberyard and O3DE as architecture references for editor, asset-pipeline,
graph, material, shader, and runtime-loading work. Reference upstream files by
repository-relative path. Do not record local checkout paths.

Visual graphs are authoring inputs. Compile hot runtime graphs to domain-native
products such as generated Rust, shader bytecode, pipeline products, or state
tables. Do not add interpreted runtime fallbacks for hot graph categories.

## Documentation

- When creating or updating architecture/design docs, use progressive
  disclosure: start with a short decision or summary, then add context, details,
  implementation shape, and edge cases below it.
- Use clean, simple file names and headings. Prefer direct names such as
  `terrain-source-route.md` over long or clever titles.
- Keep overview docs as indexes or summaries. Put detailed rationale in focused
  companion docs and link to them.

## Planning and decisions

- Linear is the sole planning and decision system. The `Azoth architecture
  decisions` project contains the ADR index and individual decision records.
- Claim ADR numbers from the Linear index, never from memory. Before
  implementing a decision, check its status and any open amendment issue.
- Large multi-session efforts are planned in Linear as Wayfinder map issues,
  child issues, and project documents. Conventions for projects, blockers,
  frontier work, resolution evidence, and ADR ownership are in
  `docs/agents/issue-tracker.md`. Do not create planning trees under
  `docs/adr/`, `plans/`, or `.scratch/`.

## Build and validation

- Tests must create scratch files with `tempfile` or the platform temporary
  directory. Do not encode a developer checkout, drive letter, home directory,
  or retained repository path in a test.
- Use the repository's normal root Cargo target directory.
- Prefer bounded parallelism for local checks, for example `cargo check -j4`.
- Do not leave cargo, rustc, editor, daemon, or service processes you started running after a task is complete.
- Do not manually edit generated Drizzle migration output; use the crate build/generation path that owns it.

## Observability and debugging

- Use `#[instrument]` from `tracing` on startup/bootstrap, project/session planning, level-selection, and other setup/teardown paths that establish runtime wiring.
- Emit `info!` logs for meaningful flow milestones, resolver outcomes, and bootstrap/setup failures.
- Use `debug_assert!` for setup invariants where assumptions should always hold.
- Do not add `#[instrument]`, `debug_assert!`, or frequent tracing calls in hot paths such as update systems, render loops, fixed/tick systems, packet loops, or per-asset inner loops.
