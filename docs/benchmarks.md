# Benchmarks

Every number lamplighter publishes, with how it was measured and what it does not show. Gold sets are private; only aggregates are published, never item text or per-item results. Where a figure
belongs to another project it is marked self-reported and linked.

## Summary

| Measurement | Result | Public? | Status |
| --- | --- | --- | --- |
| Stage A retrieval, hook recall@5 | 0.961 postgres_hybrid, 0.913 mem0, 0.811 Graphiti, 0.562 ripgrep | Aggregates only | Done, 2026-09-27/28 |
| Ingest cost per memory | mem0 $0.0135, Graphiti $0.0516, others none | Aggregates only | Done |
| Relevance floor calibration | gate recall@5 0.9613 vs memorygraph 0.9487, negatives empty 33/57 vs 27/57 at the default 0.52/0.52 `capped:3` (in-sample) | Aggregates only | Done, LAMP-19, LAMP-21, LAMP-23, LAMP-22 |
| Stage B end to end | correct 12.9% to 78.5% (Opus 5.5) | Aggregates only | Done, 2026-09-28 |
| In-process embedding latency | 117 ms p50 short query on an M2 | Yes, [bench/embed](bench/embed/report.md) | Done |
| LongMemEval retrieval (public, retrieval only) | all 470 questions: recall@5 0.949, MRR 0.921 at the default floor (in-sample); 0.987 / 0.949 with the floor off | Yes, reproducible | Done; see [bench/public/README.md](bench/public/README.md) |

## Stage A: retrieval on a frozen corpus

- **Set-up.** A private gold set of 35 items (31 with a relevant memory, 4 that should return nothing) over a frozen corpus of 1,469 real memories from one author's store. 208 queries per arm: 35 hook
  texts in three case forms plus 103 model-written queries. Seven arms, two clients, zero API calls.
- **Metric.** Hook recall@5, averaged across the original, lower-case and UPPER-CASE forms of each hook query, then across items. Ties are broken by MRR.
- **Result.**

| Arm | Case-averaged hook recall@5 | Hook MRR | Ingest cost per memory |
| --- | --- | --- | --- |
| postgres_hybrid | 0.961 | 0.836 | none |
| falkordb_indexed | 0.916 | 0.723 | none |
| mem0 | 0.913 | 0.719 | $0.0135 |
| Graphiti | 0.811 | 0.656 | $0.0516 |
| memorygraph_fixed (memorygraph with a cheap fix) | 0.772 | 0.616 | none |
| ripgrep over markdown | 0.562 | 0.454 | none |
| memorygraph_asis (memorygraph as shipped) | 0.000 | 0.000 | none |

- **What it means.** `postgres_hybrid` is Postgres full-text plus pgvector, fused, the design lamplighter productises. It is **not** `lampl`: lamplighter's own fusion, floor and embedder were not in
  this run. Ingest cost is Haiku 4.5 list price ($1/M input, $5/M output) over all 1,469 memories including failed attempts: mem0 $19.90 and Graphiti $75.76 in total.
- **Caveats.** One author's memories and queries. Thirty-one answerable items is a small set. Graphiti's recall fell to 0.741 on UPPER-CASE queries. No arm abstained on the 4 no-answer items: every
  arm except the broken memorygraph baseline returned five results for all four. mem0 and Graphiti were run with our adapters and settings; their maintainers may tune them better.

## Relevance floor (LAMP-19, LAMP-21, LAMP-23, LAMP-22)

On a private set of 138 real hook-style queries against about 2,670 memories (102 recurrence, 17 update, 19 no-answer), with Qwen3 vectors, scored like homelab's LAB-693 gate against the production memorygraph:
the default floor (cosine 0.52 or `capped:3` coverage 0.52, LAMP-22) gives hook recall@5 0.9613 against memorygraph's 0.9487, and returns nothing on 33 of 57 no-answer rows against its 27 of 57;
recurrence recall@5 is 0.9686 against 0.9539 and update 0.9176 against 0.9176.
The LAMP-23 default it replaced (cosine 0.52 or plain coverage 0.35) gave 0.9592 and 30 of 57, with recurrence recall@5 0.9662 against 0.9539 and update 0.9176 against 0.9176.
With the floor off, recall@5 is 0.9571 and no no-answer query returns nothing. **These numbers are in-sample: the floor was tuned on this same gold set and on the same LongMemEval run.** The history is in [ranking.md](ranking.md): LAMP-19 chose
0.70 / 0.24 on the gold set alone, LAMP-21 lowered coverage to 0.18 after LongMemEval showed it returning nothing on 37% of questions, and LAMP-23 fixed how query terms are built (stop words no longer count against coverage), which moved
coverage values up and the floor to 0.52 / 0.35. LAMP-22 then measured coverage against the query's three rarest terms (`capped:3`) and moved the floor to 0.52 / 0.52, the best of 3,157 grid
points on both workloads. With 19 no-answer queries a single query moves the result by about five points, and the corpus is one author's store.

## Stage B: end to end

- **Set-up.** 4,117 headless Claude Code sessions (1,938 question sessions, 240 SSH-task sessions, 1,939 judge sessions) on 2026-09-28. Arms ranged from no memory (M0) through push-hook memory over
  each retrieval backend to an oracle that injects the gold memories (M6). Push arms injected the top five titles and ids; the agent had to read a memory body itself.
- **Metric.** A Haiku 4.5 judge graded each answer against the gold memories: correct means the same cause and the same fix. 31 answerable items, three repeats, never counted as separate units.
- **Result (correct rate).**

| Model | No memory (M0) | Push, postgres_hybrid (M3) | Oracle (M6) |
| --- | --- | --- | --- |
| Opus 5.5 | 12.9% | 78.5% | 95.7% |
| Sonnet 5 | 8.6% | 52.7% | 83.9% |
| Haiku 4.5 | 0.0% | 30.1% | 90.3% |

- **Caveats.** The questions ask about this author's own incident history, so a no-memory baseline is structurally low: the model could not know the answers. The result shows memory pushes work for
  that kind of question, not that memory lifts general coding ability. The design detects differences of roughly 20 points; the best push arm was chosen after the fact. The judge was a model.

## Public LongMemEval harness

`lampl-bench` measures retrieval of the right past conversation on the public [LongMemEval](https://github.com/xiaowu0162/LongMemEval) dataset, with no LLM judge and no API key. Method, metrics and
the run command are in [bench/public/README.md](bench/public/README.md). The licence finding is in [bench/public/datasets.md](bench/public/datasets.md).

**Status: the full 470-question run is done** (retrieval only: no LLM answer step, so it is not comparable to end-to-end LongMemEval accuracy). At the default floor (cosine 0.52, `capped:3` coverage
0.52) recall@5 is 0.955 and MRR 0.926, with 3 of 470 questions returning nothing; with the floor off, 0.987 and 0.949. **The default floor was tuned on this same run, so these are in-sample figures.**
The LAMP-23 default (plain 0.52 / 0.35) scored 0.949 / 0.921 with 6 empty. Before LAMP-23 the same run scored 0.811 / 0.796 with 64 questions empty (coverage 0.18) and 0.591 with 176 empty (coverage
0.24). By-type tables, configuration and caveats are in [bench/public/README.md](bench/public/README.md).

## Embedding latency

Reproducible by anyone: `lampl bench-embed --texts docs/bench/embed/texts-47.jsonl --prepare`. Results in [embeddings.md](embeddings.md) and [bench/embed/report.md](bench/embed/report.md).

## What other projects report

Other projects publish their own figures. We repeat none: they are self-reported, usually on different datasets, judges and configurations, and several mix a managed platform's numbers with the
open-source repository. Disputes are public: for example Zep's rebuttal of mem0's LoCoMo claims
([blog.getzep.com](https://blog.getzep.com/lies-damn-lies-statistics-is-mem0-really-sota-in-agent-memory/)). Comparing across benchmark versions or judges is not meaningful. What lamplighter can say
is what it measured itself, in the tables above.

## Reproduce

| What | How |
| --- | --- |
| Embedding latency | `lampl bench-embed --texts docs/bench/embed/texts-47.jsonl --prepare` |
| LongMemEval smoke run | Steps in [bench/public/README.md](bench/public/README.md); CI runs it nightly and on manual dispatch |
| Stage A, Stage B, floor sweep | Run against private gold sets; the method is written out above and in [ranking.md](ranking.md) but the items are not published |
