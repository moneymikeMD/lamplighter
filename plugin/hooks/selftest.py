#!/usr/bin/env python3
# Selftest for lamplighter-push.py. Uses a stub `lampl` on PATH: no network, no real server, no real memory store.

import json
import os
import shutil
import subprocess
import sys
import tempfile

HOOK = os.path.join(os.path.dirname(os.path.abspath(__file__)), "lamplighter-push.py")

STUB = """#!/bin/sh
echo "$@" >> "$STUB_CALLS"
case "$STUB_MODE" in
  hits) cat "$STUB_HITS" ;;
  empty) echo '{"hits":[],"warning":null}' ;;
  version) echo "lampl: API version mismatch: http://x speaks version 2, this client speaks 1; upgrade the older side" >&2; exit 1 ;;
  model) echo "lampl: embedding model not downloaded; run \\`lampl init --local --prepare\\` once, or pass --no-embed" >&2; exit 1 ;;
  garbage) echo 'not json' ;;
  slow) sleep 10 ;;
  *) echo "lampl: boom" >&2; exit 1 ;;
esac
"""

HITS = {"hits": [{"score": 1.0, "memory": {"id": "m%d" % i, "title": "Title %d" % i, "body": "Body %d" % i}}
                 for i in range(1, 6)], "warning": None}

failures = []


def check(name, cond, detail=""):
    if not cond:
        failures.append("%s %s" % (name, detail))
        print("FAIL", name, detail)
    else:
        print("ok  ", name)


class Rig:
    def __init__(self):
        self.root = tempfile.mkdtemp(prefix="lamplighter-selftest-")
        self.bin = os.path.join(self.root, "bin")
        os.mkdir(self.bin)
        stub = os.path.join(self.bin, "lampl")
        with open(stub, "w") as fh:
            fh.write(STUB)
        os.chmod(stub, 0o755)
        self.hits = os.path.join(self.root, "hits.json")
        with open(self.hits, "w") as fh:
            json.dump(HITS, fh)
        self.calls = os.path.join(self.root, "calls")
        self.state = os.path.join(self.root, "state")

    def env(self, mode, **extra):
        env = {k: v for k, v in os.environ.items() if not k.startswith(("LAMPL_", "STUB_"))}
        env.update(PATH=self.bin + os.pathsep + "/usr/bin:/bin", STUB_MODE=mode, STUB_HITS=self.hits,
                   STUB_CALLS=self.calls, LAMPL_PUSH_STATE=self.state, LAMPL_PUSH_TIMEOUT="2",
                   LAMPL_PUSH_RECALL_CAP="1")
        env.update(extra)
        return env

    def fire(self, mode, event="UserPromptSubmit", session="s1", env=None, path=None, **payload):
        data = {"hook_event_name": event, "session_id": session, "cwd": self.root}
        data.update(payload)
        run_env = self.env(mode, **(env or {}))
        if path is not None:
            run_env["PATH"] = path
        proc = subprocess.run([sys.executable, HOOK], input=json.dumps(data), capture_output=True, text=True,
                              env=run_env, timeout=20)
        out = json.loads(proc.stdout) if proc.stdout.strip() else None
        return proc.returncode, out

    def calls_made(self):
        try:
            with open(self.calls) as fh:
                return len(fh.read().splitlines())
        except OSError:
            return 0

    def clear(self):
        shutil.rmtree(self.state, ignore_errors=True)
        try:
            os.remove(self.calls)
        except OSError:
            pass


def context(out):
    return out["hookSpecificOutput"]["additionalContext"] if out else ""


def main():
    rig = Rig()
    try:
        prompt = "how does the embedding queue retry failed batches"

        code, out = rig.fire("hits", prompt=prompt)
        check("push exits 0", code == 0)
        check("push injects three full bodies", out and context(out).count("## Title") == 3, str(out))
        check("push lists the rest by title", "- Title 4 [m4]" in context(out))
        check("push names the event", out["hookSpecificOutput"]["hookEventName"] == "UserPromptSubmit")
        check("recall called with --json", "--json" in open(rig.calls).read())

        code, out = rig.fire("hits", prompt=prompt)
        check("same prompt again is a repeat, no output", out is None and rig.calls_made() == 1)

        code, out = rig.fire("hits", prompt="a different question about the embedding queue backlog")
        check("already-injected memories are not repeated", out is None, str(out))

        code, out = rig.fire("hits", prompt=prompt, session="s2")
        check("a new session injects again", out and "## Title 1" in context(out))

        rig.clear()
        code, out = rig.fire("empty", prompt=prompt)
        check("empty recall is silent", code == 0 and out is None)
        check("empty recall does not trip the breaker", not os.path.exists(os.path.join(rig.state, "breaker")))

        rig.clear()
        code, out = rig.fire("hits", prompt="fix it please")
        check("trivial prompt skips recall", out is None and rig.calls_made() == 0)
        code, out = rig.fire("hits", prompt="LAMP-10 status")
        check("ticket key bypasses the skip gate", out is not None)

        rig.clear()
        code, out = rig.fire("hits", event="SessionStart", source="startup")
        check("SessionStart queries by repo", out is not None and rig.calls_made() == 1)

        rig.clear()
        code, out = rig.fire("hits", event="PostToolUseFailure", tool_name="Bash",
                             error="cargo: command not found on this machine")
        check("Bash failure recalls", out is not None)
        rig.clear()
        code, out = rig.fire("hits", event="PostToolUseFailure", tool_name="Edit", error="nope nope nope nope")
        check("non-Bash failure is ignored", out is None and rig.calls_made() == 0)

        rig.clear()
        code, out = rig.fire("hits", prompt=prompt, env={"LAMPL_PUSH": "0"})
        check("LAMPL_PUSH=0 opts out", out is None and rig.calls_made() == 0)

        rig.clear()
        code, out = rig.fire("version", prompt=prompt)
        check("version mismatch is loud", out and "version" in out["systemMessage"].lower()
              and "version" in context(out).lower(), str(out))
        code, out = rig.fire("version", prompt="another question about the embedding queue retries")
        check("version mismatch is quiet the second time", out is None and rig.calls_made() == 1)

        rig.clear()
        code, out = rig.fire("model", prompt=prompt)
        check("missing model is loud and points at doctor", out and "doctor --fetch-model" in out["systemMessage"],
              str(out))
        check("hook never downloads the model", "doctor" not in open(rig.calls).read())

        rig.clear()
        code, out = rig.fire("boom", prompt=prompt)
        check("generic failure is loud once", out and "boom" in out["systemMessage"], str(out))
        code, out = rig.fire("boom", prompt="another question about the embedding queue retries")
        check("breaker holds recall off after a failure", out is None and rig.calls_made() == 1)
        code, out = rig.fire("hits", prompt="yet another question about retries and queues",
                             env={"LAMPL_PUSH_BREAKER": "0.001"}, session="s9")
        check("breaker closes after its window", out is not None)

        rig.clear()
        code, out = rig.fire("garbage", prompt=prompt)
        check("unparsable output is loud, not a silent no-op", out and "unexpected output" in out["systemMessage"],
              str(out))

        rig.clear()
        code, out = rig.fire("slow", prompt=prompt)
        check("slow recall is cut off and reported", code == 0 and out and "timed out" in out["systemMessage"],
              str(out))

        rig.clear()
        code, out = rig.fire("hits", prompt=prompt, path="/usr/bin:/bin")
        check("missing lampl is loud", out and "not on PATH" in out["systemMessage"], str(out))
    finally:
        shutil.rmtree(rig.root, ignore_errors=True)

    if failures:
        print("\n%d failed" % len(failures))
        return 1
    print("\nall passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
