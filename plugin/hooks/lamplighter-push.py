#!/usr/bin/env python3
# Ported from night-watchman hooks/memory-push.py. Copyright (c) Mike Garrett, MIT License; see NOTICE.
#
# Reads one hook payload on stdin, calls `lampl recall --json`, and prints at most one JSON object on stdout.
# Each memory is injected once per session. Always exits 0. A failure is reported to the session that hit it; while
# the shared breaker is open other sessions skip quietly, and a new session's SessionStart probes once to close it.

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
DEFAULT_BUDGET = 4.0
DEFAULT_BREAKER = 300.0
RECALL_CAP = 3.5
EMBED_TIMEOUT_MS = 1500
GIT_BUDGET = 0.8
REAP_SECONDS = 0.5
DEFAULT_SKIP_WORDS = 3
DEFAULT_BODY_COSINE = 0.70
HEADLESS_QUERY = "claude -p headless trials cost prompt cache"
HEADLESS_FLAGS = frozenset(("-p", "--print"))

SEGMENT = re.compile(r"[;&|\n()`]|\$\(")
WRAPPERS = frozenset(("env", "time", "timeout", "nohup", "nice", "command", "exec", "sudo", "xargs", "do", "then",
                      "else", "!", "{"))
PREFIX_WORD = re.compile(r"^(?:[A-Za-z_][A-Za-z0-9_]*=\S*|-\S*|[0-9.]+[smhd]?)$")
WORD = re.compile(r"[^\W_]+")
TICKET_KEY = re.compile(r"(?<![\w-])[A-Za-z][A-Za-z0-9]*-[0-9]+\b")
NOT_OWNER = re.compile(r"\A\s*(?:<task-notification>|<teammate-message\b|\[SYSTEM NOTIFICATION\b"
                       r"|From [\w.]+(?:-[\w.]+)+(?: \([^)\n]*\))?:)")
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
    """Return the log record when this prompt is skipped (not the owner's, or too few content words), else None."""
    if event != "UserPromptSubmit":
        return None
    head = prompt_head(data)
    if NOT_OWNER.match(head):
        return {"event": event, "outcome": "not-owner", "query_chars": len(head)}
    limit = env_int("LAMPL_PUSH_SKIP_WORDS", DEFAULT_SKIP_WORDS)
    if limit <= 0:
        return None
    if not head or TICKET_KEY.search(head):
        return None
    words = content_words(head)
    if words > limit:
        return None
    return {"event": event, "outcome": "few-words", "content_words": words, "query_chars": len(head)}


def headless_claude(command):
    """True when a shell command line starts `claude` with -p or --print in any of its segments."""
    for segment in SEGMENT.split(command or ""):
        words = segment.split()
        for i, word in enumerate(words):
            if word in WRAPPERS or PREFIX_WORD.match(word):
                continue
            if os.path.basename(word.strip("'\"")) == "claude" and HEADLESS_FLAGS.intersection(words[i + 1:]):
                return True
            break
    return False


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
    """Return ([{id, title, content, cosine}], warning) from `lampl recall --json`, or None when the output is not that object."""
    try:
        data = json.loads(text)
    except ValueError:
        return None
    if not isinstance(data, dict) or not isinstance(data.get("hits"), list):
        return None
    warning = data.get("warning") if isinstance(data.get("warning"), str) else ""
    results = []
    for hit in data["hits"]:
        mem = hit.get("memory") if isinstance(hit, dict) else None
        if isinstance(mem, dict) and mem.get("id") and mem.get("title"):
            cosine = hit.get("cosine")
            results.append({"id": str(mem["id"]), "title": str(mem["title"]),
                            "content": str(mem.get("body") or ""),
                            "cosine": cosine if isinstance(cosine, (int, float)) else None})
    return results[:LIMIT], warning


def truncate(text, limit=BODY_CHARS):
    text = text.strip()
    if len(text) <= limit:
        return text
    cut = text[:limit]
    space = cut.rfind(" ")
    return (cut[:space] if space > limit // 2 else cut).rstrip() + " [truncated]"


def payload(bodies, titles):
    parts = ["Stored memories that may be relevant. Close matches are shown in full or truncated, weaker ones by "
             "title; `lampl get <id>` prints one in full.\n"]
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


_GIT_READERS = {}


def git_reader(cwd, deadline):
    """One memoised `git -C cwd` reader per cwd, sharing a single GIT_BUDGET across every call this run."""
    if cwd in _GIT_READERS:
        return _GIT_READERS[cwd]
    git_deadline = Deadline(min(GIT_BUDGET, deadline.left()))
    seen = {}

    def git(*args):
        if args not in seen:
            status, out, _ = run(["git", "-C", cwd, *args], git_deadline.left())
            seen[args] = out.strip() if status == "ok" else ""
        return seen[args]

    _GIT_READERS[cwd] = git
    return git


def repo_name(git):
    """The origin remote's repository name, else the work tree's directory name, else "" outside a git repo."""
    top = git("rev-parse", "--show-toplevel")
    if not top:
        return ""
    url = git("remote", "get-url", "origin")
    repo = re.sub(r"\.git$", "", url.rstrip("/").rsplit("/", 1)[-1].rsplit(":", 1)[-1]) if url else ""
    return repo or os.path.basename(top.rstrip("/"))


def session_project(cwd, deadline):
    """`LAMPL_PROJECT` when set (empty means unscoped), else the session's repo name, else None outside a git repo."""
    if "LAMPL_PROJECT" in os.environ:
        return os.environ["LAMPL_PROJECT"].strip() or None
    return repo_name(git_reader(cwd, deadline)) or None


def session_query(cwd, deadline):
    git = git_reader(cwd, deadline)
    repo = repo_name(git) or os.path.basename(cwd.rstrip("/"))
    branch = git("rev-parse", "--abbrev-ref", "HEAD")
    return " ".join(p for p in (repo, branch if branch not in ("HEAD", "") else "") if p)


class State:
    """Per-session record of answered queries (q), injected memory ids (m), shown notices (n), breaker probes (p)
    and one-shot triggers already fired (t)."""

    KINDS = ("q", "m", "n", "p", "t")

    def __init__(self, session_id):
        self.dir = os.environ.get("LAMPL_PUSH_STATE") or os.path.join(tempfile.gettempdir(), "lamplighter-push")
        name = re.sub(r"[^A-Za-z0-9_-]", "_", session_id) if session_id else ""
        self.path = os.path.join(self.dir, "session-" + name) if name else None
        self.seen = {k: set() for k in self.KINDS}
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
        self.seen = {k: set() for k in self.KINDS}
        if self.path:
            self._write("", "w")

    def record(self, kind, values):
        values = [v for v in values if v not in self.seen[kind]]
        self.seen[kind].update(values)
        if self.path and values:
            self._write("".join("%s %s\n" % (kind, v) for v in values), "a")

    def breaker(self, window):
        """Return the failure that opened the breaker as {kind, message}, or None when it is closed."""
        path = os.path.join(self.dir, "breaker")
        try:
            if time.time() - os.path.getmtime(path) >= window:
                return None
            with open(path) as fh:
                info = json.load(fh)
            return info if isinstance(info, dict) else {}
        except (OSError, ValueError):
            return {} if os.path.exists(path) else None

    def trip(self, kind, message):
        try:
            os.makedirs(self.dir, mode=0o700, exist_ok=True)
            with open(os.path.join(self.dir, "breaker"), "w") as fh:
                json.dump({"kind": kind, "message": message}, fh)
        except OSError:
            pass

    def close_breaker(self):
        try:
            os.remove(os.path.join(self.dir, "breaker"))
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


def query_for(event, data, cwd, deadline):
    if event == "SessionStart":
        return session_query(cwd, deadline).strip()
    if event == "UserPromptSubmit":
        return prompt_head(data)
    if event == "PostToolUseFailure":
        if data.get("is_interrupt") or data.get("tool_name") != "Bash":
            return ""
        text = (text_of(data.get("error")) or text_of(data.get("tool_response"))).strip()
        return text[-QUERY_CHARS:].strip()
    if event == "PreToolUse":
        return HEADLESS_QUERY
    return ""


def remedy(text):
    """Pick the fix lampl itself named; otherwise point at `lampl doctor`, which reports the server's embedder."""
    if "init --local --prepare" in text:
        return "Run `lampl init --local --prepare` once"
    if "fetch-model" in text:
        return "Run `lampl doctor --fetch-model` once"
    return "Run `lampl doctor` to see the server's embedder"


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
        return "model", "lamplighter: the embedding model is missing. %s; memory push is off until then." % remedy(first)
    return "error", "lamplighter: recall failed (%s). Memory push is paused for a few minutes." % (
        first.removeprefix("lampl: ") or "no error text")


def warning_notice(warning):
    """Turn a recall warning (exit 0, hits present or not) into a (kind, message) notice."""
    if "timed out" in warning:
        return "slow", "lamplighter: embedding the query timed out, so this recall used full text only."
    return "degraded", "lamplighter: %s. Hits are full-text only. %s." % (warning.rstrip("."), remedy(warning))


def hold(text):
    """Deny the first headless `claude` command once, carrying the memories as the reason the model reads."""
    reason = (text + "\nHeld once so you can read these before a paid headless run starts. This is not a refusal: "
              "adjust the run if they apply, then run the command again; it will not be held a second time.")
    sys.stdout.write(json.dumps({"hookSpecificOutput": {"hookEventName": "PreToolUse", "permissionDecision": "deny",
                                                        "permissionDecisionReason": reason}}) + "\n")


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
    if event == "PreToolUse":
        command = (data.get("tool_input") or {}).get("command") if isinstance(data.get("tool_input"), dict) else ""
        if data.get("tool_name") != "Bash" or not headless_claude(command or "") or "headless" in state.seen["t"]:
            return
        state.record("t", ["headless"])
    if event == "SessionStart" and data.get("source") in ("compact", "clear"):
        state.reset()
    skipped = gate_record(event, data)
    if skipped:
        log(skipped)
        return
    tripped = state.breaker(env_float("LAMPL_PUSH_BREAKER", DEFAULT_BREAKER))
    if tripped is not None:
        if event != "SessionStart" or "breaker" in state.seen["p"]:
            log({"event": event, "outcome": "breaker"})
            return
        state.record("p", ["breaker"])
    query = query_for(event, data, cwd, deadline)
    if not query:
        return
    digest = hashlib.sha256((event + ":" + query).encode("utf-8", "replace")).hexdigest()
    if event != "SessionStart" and digest in state.seen["q"]:
        log({"event": event, "outcome": "repeat"})
        return
    project = None if event == "PreToolUse" else session_project(cwd, deadline)
    budget = min(deadline.left(), env_float("LAMPL_PUSH_RECALL_CAP", RECALL_CAP))
    record = {"event": event, "query_chars": len(query)}
    if budget <= 0:
        log(dict(record, outcome="budget"))
        return
    embed_ms = str(env_int("LAMPL_PUSH_EMBED_MS", EMBED_TIMEOUT_MS))
    argv = ["lampl", "recall", "--query", query, "--limit", str(LIMIT), "--embed-timeout-ms", embed_ms, "--json"]
    if project:
        argv += ["--project", project]
        record["project"] = project
    status, out, err = run(argv, budget, cwd)
    parsed = parse_hits(out) if status == "ok" else None
    if parsed is None:
        if status == "ok":
            status, err = "error", "unexpected output from `lampl recall --json`"
        kind, message = classify(status, err)
        state.trip(kind, message)
        record.update(outcome=kind, seconds=round(time.monotonic() - started, 2))
        log(record)
        if kind not in state.seen["n"]:
            state.record("n", [kind])
            emit(event, "", message)
        return
    results, warning = parsed
    if tripped is not None:
        state.close_breaker()
    state.record("q", [digest])
    notice = ""
    if warning:
        wkind, wmessage = warning_notice(warning)
        if wkind not in state.seen["n"]:
            state.record("n", [wkind])
            notice = wmessage
    fresh = [r for r in results if r["id"] not in state.seen["m"]]
    floor = env_float("LAMPL_PUSH_BODY_COSINE", DEFAULT_BODY_COSINE)
    strong = [r for r in fresh if r["cosine"] is not None and r["cosine"] >= floor]
    top = strong[:BODIES]
    rest = [r for r in fresh if r not in top]
    record.update(ids=[r["id"] for r in results], new=[r["id"] for r in fresh], full_bodies=len(top),
                  seconds=round(time.monotonic() - started, 2))
    if not fresh:
        record["outcome"] = "nothing-new" if results else "empty"
        log(record)
        emit(event, "", notice)
        return
    text = payload(top, rest)
    state.record("m", [r["id"] for r in top + rest])
    record.update(outcome="pushed", payload_chars=len(text))
    log(record)
    if event == "PreToolUse":
        hold("\n".join(p for p in (notice, text) if p))
    else:
        emit(event, text, notice)


if __name__ == "__main__":
    try:
        main()
    except Exception:  # a push hook must never fail the session it serves
        pass
    sys.exit(0)
