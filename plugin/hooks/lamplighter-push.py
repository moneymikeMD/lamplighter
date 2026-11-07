#!/usr/bin/env python3
# Ported from night-watchman hooks/memory-push.py. Copyright (c) Mike Garrett, MIT License; see NOTICE.
#
# Reads one hook payload on stdin, calls `lampl recall --json`, and prints at most one JSON object on stdout.
# Each memory is injected once per session. Always exits 0. Failures are reported once per session, never silently.

import hashlib
import json
import os
import re
import signal
import subprocess
import sys
import tempfile
import time

LIMIT = 5
BODIES = 3
BODY_CHARS = 1250
QUERY_CHARS = 400
DEFAULT_BUDGET = 3.0
DEFAULT_BREAKER = 300.0
RECALL_CAP = 1.5
REAP_SECONDS = 0.5
DEFAULT_SKIP_WORDS = 3

WORD = re.compile(r"[^\W_]+")
TICKET_KEY = re.compile(r"(?<![\w-])[A-Za-z][A-Za-z0-9]*-[0-9]+\b")
# PostgreSQL's english.stop verbatim: the list the store's full-text recall drops.
STOP_WORDS = frozenset("""
i me my myself we our ours ourselves you your yours yourself yourselves he him his himself she her hers
herself it its itself they them their theirs themselves what which who whom this that these those am is are
was were be been being have has had having do does did doing a an the and but if or because as until while
of at by for with about against between into through during before after above below to from up down in
out on off over under again further then once here there when where why how all any both each few more
most other some such no nor not only own same so than too very s t can will just don should now
""".split())


class Deadline:
    def __init__(self, seconds):
        self.end = time.monotonic() + seconds

    def left(self):
        return self.end - time.monotonic()


def env_float(name, default):
    try:
        return float(os.environ.get(name) or default)
    except ValueError:
        return default


def env_int(name, default):
    try:
        return int(os.environ.get(name) or default)
    except ValueError:
        return default


def content_words(text):
    return len({w for w in WORD.findall(text.lower()) if w not in STOP_WORDS})


def prompt_head(data):
    return (data.get("prompt") or "").strip()[:QUERY_CHARS].strip()


def gate_record(event, data):
    """Return the log record when the content-word gate skips this prompt, else None."""
    limit = env_int("LAMPL_PUSH_SKIP_WORDS", DEFAULT_SKIP_WORDS)
    if event != "UserPromptSubmit" or limit <= 0:
        return None
    head = prompt_head(data)
    if not head or TICKET_KEY.search(head):
        return None
    words = content_words(head)
    if words > limit:
        return None
    return {"event": event, "outcome": "few-words", "content_words": words, "query_chars": len(head)}


def run(argv, budget, cwd=None):
    """Run argv for at most budget seconds; return (status, stdout, stderr), status ok, error, timeout or missing."""
    if budget <= 0:
        return "timeout", "", ""
    try:
        proc = subprocess.Popen(argv, cwd=cwd, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                stderr=subprocess.PIPE, start_new_session=True)
    except FileNotFoundError:
        return "missing", "", ""
    except OSError:
        return "error", "", ""
    try:
        out, err = proc.communicate(timeout=budget)
    except subprocess.TimeoutExpired:
        try:
            os.killpg(proc.pid, signal.SIGKILL)
        except OSError:
            pass
        try:
            proc.communicate(timeout=REAP_SECONDS)
        except subprocess.TimeoutExpired:
            pass  # a descendant that left the process group still holds the pipe
        return "timeout", "", ""
    status = "ok" if proc.returncode == 0 else "error"
    return status, out.decode("utf-8", "replace"), err.decode("utf-8", "replace")


def parse_hits(text):
    """Return [{id, title, content}] from `lampl recall --json`, or None when the output is not that object."""
    try:
        data = json.loads(text)
    except ValueError:
        return None
    if not isinstance(data, dict) or not isinstance(data.get("hits"), list):
        return None
    results = []
    for hit in data["hits"]:
        mem = hit.get("memory") if isinstance(hit, dict) else None
        if isinstance(mem, dict) and mem.get("id") and mem.get("title"):
            results.append({"id": str(mem["id"]), "title": str(mem["title"]),
                            "content": str(mem.get("body") or "")})
    return results[:LIMIT]


def truncate(text, limit=BODY_CHARS):
    text = text.strip()
    if len(text) <= limit:
        return text
    cut = text[:limit]
    space = cut.rfind(" ")
    return (cut[:space] if space > limit // 2 else cut).rstrip() + " [truncated]"


def payload(bodies, titles):
    parts = ["Stored memories that may be relevant. The top ones are shown in full or truncated; "
             "`lampl get <id>` prints one in full.\n"]
    for r in bodies:
        parts.append("## %s [%s]\n%s\n" % (r["title"], r["id"], truncate(r["content"])))
    if titles:
        parts.append("Also possibly relevant (title [id]):\n"
                     + "\n".join("- %s [%s]" % (r["title"], r["id"]) for r in titles))
    return "\n".join(parts).rstrip() + "\n"


def text_of(value):
    if isinstance(value, str):
        return value
    if isinstance(value, dict):
        return "\n".join(text_of(value.get(k)) for k in ("stderr", "stdout", "error", "output") if value.get(k))
    if isinstance(value, list):
        return "\n".join(text_of(v) for v in value)
    return ""


def session_query(cwd):
    def git(*args):
        status, out, _ = run(["git", "-C", cwd, *args], 1.0)
        return out.strip() if status == "ok" else ""

    url = git("remote", "get-url", "origin")
    top = git("rev-parse", "--show-toplevel")
    branch = git("rev-parse", "--abbrev-ref", "HEAD")
    repo = re.sub(r"\.git$", "", url.rstrip("/").rsplit("/", 1)[-1].rsplit(":", 1)[-1]) if url else ""
    repo = repo or os.path.basename(top or cwd.rstrip("/"))
    return " ".join(p for p in (repo, branch if branch != "HEAD" else "") if p)


class State:
    """Per-session record of answered queries (q), injected memory ids (m) and shown notices (n), plus the breaker."""

    def __init__(self, session_id):
        self.dir = os.environ.get("LAMPL_PUSH_STATE") or os.path.join(tempfile.gettempdir(), "lamplighter-push")
        name = re.sub(r"[^A-Za-z0-9_-]", "_", session_id) if session_id else ""
        self.path = os.path.join(self.dir, "session-" + name) if name else None
        self.seen = {"q": set(), "m": set(), "n": set()}
        if self.path:
            try:
                with open(self.path) as fh:
                    for line in fh:
                        kind, _, value = line.strip().partition(" ")
                        if kind in self.seen and value:
                            self.seen[kind].add(value)
            except OSError:
                pass

    def _write(self, text, mode):
        try:
            os.makedirs(self.dir, mode=0o700, exist_ok=True)
            with open(self.path, mode) as fh:
                fh.write(text)
        except OSError:
            pass

    def reset(self):
        self.seen = {"q": set(), "m": set(), "n": set()}
        if self.path:
            self._write("", "w")

    def record(self, kind, values):
        values = [v for v in values if v not in self.seen[kind]]
        self.seen[kind].update(values)
        if self.path and values:
            self._write("".join("%s %s\n" % (kind, v) for v in values), "a")

    def breaker_open(self, window):
        try:
            return time.time() - os.path.getmtime(os.path.join(self.dir, "breaker")) < window
        except OSError:
            return False

    def trip(self):
        try:
            os.makedirs(self.dir, mode=0o700, exist_ok=True)
            with open(os.path.join(self.dir, "breaker"), "w") as fh:
                fh.write("%d\n" % time.time())
        except OSError:
            pass


def log(record):
    path = os.environ.get("LAMPL_PUSH_LOG")
    if not path:
        return
    try:
        with open(path, "a") as fh:
            fh.write(json.dumps(record) + "\n")
    except OSError:
        pass


def query_for(event, data, cwd):
    if event == "SessionStart":
        return session_query(cwd).strip()
    if event == "UserPromptSubmit":
        return prompt_head(data)
    if event == "PostToolUseFailure":
        if data.get("is_interrupt") or data.get("tool_name") != "Bash":
            return ""
        text = (text_of(data.get("error")) or text_of(data.get("tool_response"))).strip()
        return text[-QUERY_CHARS:].strip()
    return ""


def classify(status, err):
    """Name the failure kind and the one message the owner sees, once per session."""
    first = next((ln.strip() for ln in err.splitlines() if ln.strip()), "")
    if status == "missing":
        return "missing", "lamplighter: `lampl` is not on PATH, so no memories are recalled this session. Install it from the lamplighter release."
    if status == "timeout":
        return "timeout", "lamplighter: recall timed out, so memories are paused for a few minutes."
    low = first.lower()
    if "api version mismatch" in low:
        return "version", "lamplighter: %s. Memory push is off until the client and server match." % first.removeprefix("lampl: ")
    if "model" in low and ("not downloaded" in low or "cache missing" in low or "fetch-model" in low):
        return "model", "lamplighter: the embedding model is missing. Run `lampl doctor --fetch-model` once; memory push is off until then."
    return "error", "lamplighter: recall failed (%s). Memory push is paused for a few minutes." % (
        first.removeprefix("lampl: ") or "no error text")


def emit(event, context, notice):
    out = {}
    if notice:
        out["systemMessage"] = notice
    text = "\n".join(p for p in (notice, context) if p)
    if text:
        out["hookSpecificOutput"] = {"hookEventName": event, "additionalContext": text}
    if out:
        sys.stdout.write(json.dumps(out) + "\n")


def main():
    started = time.monotonic()
    if os.environ.get("LAMPL_PUSH") == "0":
        return
    deadline = Deadline(env_float("LAMPL_PUSH_TIMEOUT", DEFAULT_BUDGET))
    data = json.load(sys.stdin)
    event = data.get("hook_event_name") or ""
    cwd = data.get("cwd") or os.getcwd()
    if not os.path.isdir(cwd):
        cwd = os.getcwd()
    state = State(data.get("session_id") or "")
    if event == "SessionStart" and data.get("source") in ("compact", "clear"):
        state.reset()
    skipped = gate_record(event, data)
    if skipped:
        log(skipped)
        return
    if state.breaker_open(env_float("LAMPL_PUSH_BREAKER", DEFAULT_BREAKER)):
        log({"event": event, "outcome": "breaker"})
        return
    query = query_for(event, data, cwd)
    if not query:
        return
    digest = hashlib.sha256((event + ":" + query).encode("utf-8", "replace")).hexdigest()
    if event != "SessionStart" and digest in state.seen["q"]:
        log({"event": event, "outcome": "repeat"})
        return
    status, out, err = run(["lampl", "recall", "--query", query, "--limit", str(LIMIT), "--json"],
                           min(deadline.left(), env_float("LAMPL_PUSH_RECALL_CAP", RECALL_CAP)), cwd)
    results = parse_hits(out) if status == "ok" else None
    record = {"event": event, "query_chars": len(query)}
    if results is None:
        if status == "ok":
            status, err = "error", "unexpected output from `lampl recall --json`"
        state.trip()
        kind, message = classify(status, err)
        record.update(outcome=kind, seconds=round(time.monotonic() - started, 2))
        log(record)
        if kind not in state.seen["n"]:
            state.record("n", [kind])
            emit(event, "", message)
        return
    state.record("q", [digest])
    fresh = [r for r in results if r["id"] not in state.seen["m"]]
    top, rest = fresh[:BODIES], fresh[BODIES:]
    record.update(ids=[r["id"] for r in results], new=[r["id"] for r in fresh], full_bodies=len(top),
                  seconds=round(time.monotonic() - started, 2))
    if not top:
        record["outcome"] = "nothing-new" if results else "empty"
        log(record)
        return
    text = payload(top, rest)
    state.record("m", [r["id"] for r in top + rest])
    record.update(outcome="pushed", payload_chars=len(text))
    log(record)
    emit(event, text, "")


if __name__ == "__main__":
    try:
        main()
    except Exception:  # a push hook must never fail the session it serves
        pass
    sys.exit(0)
