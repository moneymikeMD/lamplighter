# Claude Code plugin

`plugin/` is the lamplighter Claude Code plugin (install id `lamplighter@moneymike-plugins`; the marketplace entry lives in the moneymike-plugins repo). It needs `lampl` on PATH, configured with `lampl init`.

| Part | What it does |
| --- | --- |
| `hooks/lamplighter-push.py` | On SessionStart, UserPromptSubmit and PostToolUseFailure (Bash) runs `lampl recall --json` and injects the top memories as context, each memory once per session |
| `.mcp.json` | Registers `lampl mcp` as an MCP server, resolved from PATH |
| `skills/lamplighter/SKILL.md` | The usage protocol: what to store, what not to, how to link and supersede |

## Hook behaviour

- Budget: 3 s total, recall capped at 1.5 s, hook timeout 5 s. A failure opens a circuit breaker for 5 minutes.
- Prompts with three or fewer content words are skipped unless they contain a ticket key.
- Empty recall (`{"hits": []}`) is silent. Failures are never silent: the first of each kind per session prints one message (`lampl` missing, API version mismatch, embedding model missing, timeout, other error) and later ones stay quiet.
- The hook never downloads the model; `lampl doctor --fetch-model` does.

| Variable | Default | Meaning |
| --- | --- | --- |
| `LAMPL_PUSH=0` | on | Turn the push off |
| `LAMPL_PUSH_TIMEOUT` | 3 | Total budget in seconds |
| `LAMPL_PUSH_RECALL_CAP` | 1.5 | Cap on the recall call in seconds |
| `LAMPL_PUSH_BREAKER` | 300 | Breaker window in seconds |
| `LAMPL_PUSH_SKIP_WORDS` | 3 | Skip prompts with this many content words or fewer; 0 disables |
| `LAMPL_PUSH_STATE`, `LAMPL_PUSH_LOG` | temp dir, off | Session state directory; JSON-lines decision log |

## Tests

`python3 plugin/hooks/selftest.py` drives the hook with a stub `lampl` on PATH: no network, no server, no real store. CI runs it in the lint job.

## Licence

The hook is ported from night-watchman's `hooks/memory-push.py` (MIT, Mike Garrett); the notice is in the file header and the repo `NOTICE`.
