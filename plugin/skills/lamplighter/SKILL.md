---
name: lamplighter
description: When and how to store, recall, link and supersede durable agent memories with lamplighter. Use when a decision, root cause, gotcha or constraint is worth keeping beyond this session, or before claiming something is impossible or absent.
---

# Using lamplighter

Memories are durable knowledge that would otherwise be lost when the session ends. They are not code structure; the code and the repo's docs already hold that.

The plugin pushes matching memories into the session at start, on each prompt and after a failed Bash call, each once per session. Use the `lampl` MCP tools (`store`, `recall`, `get`, `link`, `related`) for anything the push missed; the same operations exist as `lampl` CLI commands.

## Store, without being asked

- A non-obvious decision and why it was made.
- A root cause and the fix that resolved it, as two linked memories.
- A gotcha, footgun or environment quirk that cost real time to find.
- An architectural invariant or constraint a later session must not violate.

## Do not store

- Transient state, secrets, credentials or tokens.
- Speculation not yet validated.
- Anything the code, the git history or the repo docs already record.

## How to store

One fact per memory. `type` is one of `fact`, `decision`, `problem`, `fix`, `procedure`, `gotcha`. Give a specific title, a body that stands alone, `tags` naming the project, the component and the kind of change, and an `importance` between 0 and 1.

Store the problem first, then the fix, then link the fix to the problem with `SOLVES` (strength 0 to 1).

## Recall

Run `recall` before telling the owner something is impossible, unsupported or absent, and before reporting a failure. A recalled memory records what was true when written: verify any file, flag or endpoint it names before acting on it.

## Supersede, do not duplicate

When a memory turns out wrong or stale, store the corrected one, then link it to the old one with `SUPERSEDES` and name the old id in the new body. Use `CONTRADICTS` when two memories disagree and you cannot yet tell which is right, and `CAUSES` for cause and effect. Read the link back with `related`; a success line alone is not evidence that it persisted.

## When the plugin speaks up

The hook prints one message per session when memory is not working: `lampl` missing from PATH, an API version mismatch between client and server, a missing embedding model (`lampl doctor --fetch-model` fixes it; the hook never downloads), or a failing recall. Tell the owner rather than carrying on as if memory were empty. `LAMPL_PUSH=0` turns the push off.

## Session end

Store a short summary of what was decided and left open, tagged with the project.
