# Concepts

A memory is one durable fact an agent would otherwise lose when the session ends. The model lives in `crates/lampl-core/src/model.rs`; the CLI and MCP surface the same enums.

## Memory

| Field | Meaning |
| --- | --- |
| `id` | UUID, time-ordered (v7) |
| `memory_type` | One of the six types below |
| `title`, `body` | A specific title and a body that stands alone |
| `tags` | Free-form labels |
| `project` | A namespace; recall can be scoped to one (default `default`) |
| `importance` | 0 to 1 (default 0.5) |
| `created_at`, `updated_at` | UTC timestamps |
| `embedding_model`, `embedding_digest` | Which model embedded it; empty until embedded |

## Memory types

The type set is closed. An unknown type is rejected with the valid list.

| Type | Use it for |
| --- | --- |
| `fact` | Default. A durable statement about a system, a person's preference or a constraint |
| `decision` | A choice and why it was made, so it is not re-litigated |
| `problem` | A symptom or failure, written so a future search for the symptom finds it |
| `fix` | What resolved a problem; link it to the problem with `SOLVES` |
| `procedure` | A repeatable sequence of steps or a command that works |
| `gotcha` | A footgun or environment quirk that cost real time to find |

## Link types

Links are typed and directed (`from` to `to`) with a strength between 0 and 1 (default 1.0).

| Link | Reads as | Effect |
| --- | --- | --- |
| `SUPERSEDES` | `from` replaces `to` | `to` is excluded from recall and from audit; it is kept, not deleted |
| `SOLVES` | `from` solves `to` | None on ranking; navigable with `related` |
| `CAUSES` | `from` causes `to` | None on ranking |
| `CONTRADICTS` | `from` and `to` disagree | None on ranking; use it when you cannot tell which is right |

Links need a server. `related ID` returns links in both directions, labelled `outgoing` or `incoming`. A memorygraph `RELATED_TO`, `SIMILAR_TO` or `BLOCKS` link has no equivalent and is not imported.

## Projects and tags

`--project` partitions memories; recall without `--project` searches all of them. Tags are for people and for filters in your own tooling: name the project, the component and the kind of change. See
[best-practices.md](best-practices.md).

## How a memory is found

Recall is hybrid and can return nothing. Full-text and vector search each propose 50 candidates, reciprocal-rank fusion merges them, and a relevance floor drops candidates that are neither close in
meaning (cosine 0.52) nor covered by the query's terms (IDF coverage 0.35). The reasoning and the sweep behind those numbers are in [ranking.md](ranking.md). In server mode, a new memory is stored
first and embedded by a background worker within seconds; until then it is found by full text.

## Where the agent meets it

| Door | Direction | Page |
| --- | --- | --- |
| Hooks | Push: matching memories are injected at session start, on each prompt and after a failed Bash call | [plugin.md](plugin.md) |
| MCP | Pull: `store`, `recall`, `get`, `link`, `related` | [mcp.md](mcp.md) |
| CLI | Pull and admin: also `doctor`, `audit`, `export`, `import` | [cli.md](cli.md) |
| Skill | Teaches when to store and how to supersede | [plugin.md](plugin.md) |
