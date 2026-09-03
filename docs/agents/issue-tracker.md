# Issue tracking in Linear

Active plans, Wayfinder maps, and architectural decisions live in Linear.
Repository documentation records current behavior and operating guidance, not
planning state.

Use the configured shared Linear team for this repository. Create a project
for each finite, multi-session effort; do not create a team per repository or
effort. This keeps unrelated projects separate without spending the free
plan's second team.

## Wayfinder structure

- Create one parent issue for the map and label it `wayfinder:map`.
- Put `Destination`, `Notes`, `Decisions so far`, `Not yet specified`,
  and `Out of scope` in the parent description.
- Create a child issue only for actionable work or an explicit decision. Give
  it exactly one of `wayfinder:research`, `wayfinder:prototype`,
  `wayfinder:grilling`, or `wayfinder:task`.
- Store long plans and research as project documents, then link them from the
  relevant issue.
- Use Linear's native blocked-by relation. Do not encode dependencies in prose
  or labels.
- The frontier is the set of open, unassigned, unblocked child issues. Assign
  an issue before starting it.
- Resolve an issue with a concise evidence comment, a named result link, and
  the Done status. Add the durable decision to the map description.
- Refer to issues by a descriptive name. A bare identifier is not enough
  context for a reader.

The `Azoth planning and Wayfinder` project document records these operating
rules in Linear. The `Azoth public release` project is the migration example.

## ADR workflow

The `Azoth architecture decisions` Linear project contains the canonical
`Azoth ADR index` and one document per decision.

When an open issue would amend an accepted ADR:

1. Add an **Open amendments in flight** note to the Linear ADR document.
2. Link the amendment issue by name from that note.
3. Update the ADR and remove the note when the issue is resolved.
4. Claim ADR numbers from the Linear index, never from memory.

## Ticket types

`research` is evidence gathering. `prototype` is a bounded experiment.
`grilling` is a live decision with the owner. `task` is implementation or
verification work. Human-in-the-loop tickets resolve only through an actual
exchange with the user.

## Repository boundary

Do not create ticket trees under `docs/adr/`, `plans/`, or `.scratch/`.
Keep current technical guidance in `docs/`; keep work state and decision
history in Linear.
