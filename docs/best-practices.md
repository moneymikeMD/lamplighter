# Best practices

These rules are the ones the bundled skill (`plugin/skills/lamplighter/SKILL.md`) teaches the agent, with the reasoning spelled out.

## When to store

Store when the knowledge would otherwise be lost with the session and a future session would act worse without it.

- A non-obvious decision and why it was made.
- A root cause and the fix that resolved it, as two linked memories.
- A gotcha, footgun or environment quirk that cost real time to discover.
- An architectural invariant or constraint a later session must not violate.
- At session end, a short summary of what was decided and left open, tagged with the project.

## What not to store

- Anything the code, the git history or the repo docs already record. Recall is for what those cannot say.
- Transient state: today's branch name, a running job's status.
- Secrets, credentials and tokens. The store is plaintext; there is no redaction.
- Speculation not yet validated. A wrong memory is recalled with the same confidence as a right one.

## Write memories that can be found

- One fact per memory. A search that matches half a memory returns all of it.
- A specific title. `Redis eviction drops sessions` beats `Redis notes`.
- A body that stands alone, with the symptom words a future query would use. For a `problem`, include the exact error text.
- Pick the closest of the six types; `fact` is the default, not a dumping ground.
- `--importance` is stored with the memory but does not affect ranking: the full-text index covers title and body only (`migrations/0001_init.sql`) and fusion uses rank, not importance. Put the words
  you want matched in the title and body.

## Tagging

Tag with the project, the component and the kind of change, for example `billing,webhooks,fix`. Tags are free-form and are not part of the full-text index or the embedding; they are for `audit` (which
reports untagged memories) and for exports. Keep one spelling per tag.

## Link, do not duplicate

- Store the problem, then the fix, then `lampl link FIX PROBLEM SOLVES`.
- Read a link back with `lampl related ID`; do not trust the success line alone.
- Use `CAUSES` for cause and effect and `CONTRADICTS` when two memories disagree and you cannot yet say which is right.

## Retire stale memories with SUPERSEDES

When a memory turns out wrong or out of date, do not edit history and do not delete it:

1. Store the corrected memory. Name the old id in its body so the replacement is legible without following the link.
2. `lampl link NEW OLD SUPERSEDES`.
3. Check: recall no longer returns `OLD`, and `lampl audit` counts it under `superseded excluded`.

The old memory stays in the database and in exports. A recalled memory records what was true when written; verify any file, flag or endpoint it names before acting on it.

## Keep the store healthy

- Run `lampl audit` now and then: it lists duplicate titles and bodies, memories older than `--stale-days` (default 365) and untagged memories, without changing anything.
- Run `lampl doctor` after changing the embedder. Vector coverage below 0.95 marks the server degraded.
- Back up with `lampl export -o backup.ndjson`. An export carries vectors, so a restore needs no re-embedding if the model is unchanged.

## Recall before claiming

Recall before telling anyone that something is impossible, unsupported or absent, and before reporting a failure. An empty result is information: nothing relevant is stored.
