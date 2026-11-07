# lamplighter

Agent memory as one binary. `lampl` stores what a coding agent learns (decisions, problems, fixes, gotchas) in Postgres with pgvector, recalls it with hybrid full-text and vector search, and reaches the agent three ways: hooks, a CLI and an MCP server. A Claude Code plugin wires all three together.

Status: pre-release. The repo is a scaffold; nothing is installable yet.

## Lineage

lamplighter is a fresh Rust codebase, but the ideas and some ported code come from two places:

- [memory-graph/memory-graph](https://github.com/memory-graph/memory-graph) (MIT), through the owner's fork [moneymikeMD/memory-graph](https://github.com/moneymikeMD/memory-graph), based on v0.14.0.
- The memory-push hooks in [night-watchman](https://github.com/moneymikeMD/night-watchman) (MIT), which inject matching memories at session start and on each prompt.

Ported code keeps its original MIT notice, recorded in a `NOTICE` file.

## License

Apache-2.0. See `LICENSE`.
