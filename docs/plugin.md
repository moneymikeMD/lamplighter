# Claude Code plugin

`plugin/` is the lamplighter Claude Code plugin (install id `lamplighter@moneymike-plugins`; the marketplace entry lives in the moneymike-plugins repo). It needs `lampl` on PATH, configured with `lampl init`.

| Part | What it does |
| --- | --- |
| `hooks/lamplighter-push.py` | On SessionStart, UserPromptSubmit and PostToolUseFailure (Bash) runs `lampl recall --json` and injects the top memories as context, each memory once per session |
| `.mcp.json` | Registers `lampl mcp` as an MCP server, resolved from PATH |
| `skills/lamplighter/SKILL.md` | The usage protocol: what to store, what not to, how to link and supersede |

## Hook behaviour

- Budget: 4 s total (git lookups inside it), hook timeout 5 s. The hook passes `--embed-timeout-ms 1500` to `lampl recall`, which then answers full-text only with a warning; killing the process at 3.5 s is the outer guard. A failure opens a machine-wide circuit breaker for 5 minutes and tells only the session that hit it. While the breaker is open, other sessions skip quietly, and each new session's SessionStart makes one probe recall: success closes the breaker, failure keeps it open and tells that session. Running out of budget before `lampl` starts is not a `lampl` failure and does not open the breaker.
- Prompts with three or fewer content words are skipped unless they contain a ticket key.
- Empty recall (`{"hits": []}`) is silent. Failures are never silent: the first of each kind per session prints one message (`lampl` missing, API version mismatch, embedding model missing, timeout, other error) and later ones stay quiet. A recall that succeeds with a warning (the server's embedder is down, or embedding timed out) shows the warning once per session and still injects the hits. The missing-model message names the real fix: `lampl init --local --prepare` or `lampl doctor --fetch-model` in local mode, `lampl doctor` for a server's embedder.
- The hook never downloads the model; `lampl doctor --fetch-model` does.

| Variable | Default | Meaning |
| --- | --- | --- |
| `LAMPL_PUSH=0` | on | Turn the push off |
| `LAMPL_PUSH_TIMEOUT` | 4 | Total budget in seconds |
| `LAMPL_PUSH_RECALL_CAP` | 3.5 | Outer kill for the recall process in seconds |
| `LAMPL_PUSH_EMBED_MS` | 1500 | Embed timeout passed to `lampl recall` |
| `LAMPL_PUSH_BREAKER` | 300 | Breaker window in seconds |
| `LAMPL_PUSH_SKIP_WORDS` | 3 | Skip prompts with this many content words or fewer; 0 disables |
| `LAMPL_PUSH_STATE`, `LAMPL_PUSH_LOG` | temp dir, off | Session state directory; JSON-lines decision log |

## Tests

`python3 plugin/hooks/selftest.py` drives the hook with a stub `lampl` on PATH: no network, no server, no real store. CI runs it in the lint job.

## PATH spike

A plugin `.mcp.json` command is resolved from PATH, so no `plugin/bin/` shim is needed. Checked with a stub `lampl` (a shell script that logs its invocation and runs `cat`) first on PATH and a scratch config dir:

```bash
CLAUDE_CONFIG_DIR=$(mktemp -d) PATH=$STUB_DIR:$PATH claude --plugin-dir plugin mcp list
```

Result: the stub ran (it logged twice) and the list showed `plugin:lamplighter:lampl: lampl mcp - Failed to connect -32601: Method not found`. The failure is the stub not speaking MCP; the point is that Claude Code found and started `lampl` from PATH.

## End-to-end check

Against a scratch `lampl serve` (docker pgvector on a scratch port) seeded with a memory whose body contains a unique marker, with no live lab store involved:

```bash
MARK=lamplighter-e2e-$(date +%s)
lampl store --title "e2e probe" --body "$MARK"
CLAUDE_CONFIG_DIR=$(mktemp -d) claude --plugin-dir plugin -p "what do you remember about e2e probe?" --output-format stream-json --verbose | grep -c "$MARK"
```

Expected: a count of at least 1. The marker only appears if the hook injected the stored memory, whereas the plugin name appears in every init event, so grepping for `lamplighter` proves nothing. Also confirm the MCP tools are listed (`grep -c mcp__plugin_lamplighter_lampl__recall`) in the same stream.

## Licence

The hook is ported from night-watchman's `hooks/memory-push.py` (MIT, Mike Garrett); the notice is in the file header and the repo `NOTICE`.
