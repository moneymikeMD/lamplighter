# MCP

`lampl mcp` serves the memory tools over MCP on stdio. It exists so an agent receives the memory type and link type enums with the tool definitions instead of guessing them.

Run it as a local process; the plugin wiring (`.mcp.json`) is separate. Streamable HTTP is not offered: an HTTP MCP endpoint on `serve` would need its own auth story.

The tools use the same code as the CLI and the same configuration (`lampl init`). A failure, such as a server that is down, an unconfigured client or a bad argument, is returned as an MCP tool error (`isError: true`) with the CLI's message. Nothing returns an empty result in place of an error, and a server-mode client never falls back to local mode.

| Tool | Arguments | Mode |
| --- | --- | --- |
| `store` | `title`, `body`; optional `type` (enum), `tags`, `project`, `importance` | server or local |
| `recall` | `query`; optional `project`, `limit` | server or local |
| `get` | `id` | server |
| `link` | `from`, `to`, `type` (enum); optional `strength` | server |
| `related` | `id` | server |

The `type` enums are generated at runtime from `MemoryType` and `LinkType` in `lampl-core`, so the schema cannot drift from what the store accepts. Unknown arguments are rejected.

`recall` returns `{"hits": [...], "warning": ...}`; `warning` is set when the server's embedder is down and the result is full-text only. Export, import and audit are CLI-only.
