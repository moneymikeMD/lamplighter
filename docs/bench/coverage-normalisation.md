# Normalised coverage for the relevance floor (LAMP-22)

LAMP-21 found that IDF coverage is diluted on long questions: a LongMemEval question has many query terms, a relevant round matches only some of them, and so its coverage stays under a floor that short
off-topic hook queries need in order to abstain. This page tests two ways of normalising coverage against the default floor, on the stored LongMemEval run and on the private gold set. Only the
coverage that the floor tests changes; survivors are still ordered by cosine plus plain coverage, and the coverage reported on each hit is still plain. "Default" below means the LAMP-23 default
the grid was measured against (plain 0.52 / 0.35); `capped:3` at 0.52 / 0.52 replaced it as the shipped default on 2026-11-08 ([Adoption](#adoption)).

**Result:** one variant, `capped:3` at cosine 0.52 / coverage 0.52, does better than the default 0.52 / 0.35 on all four measures below: half the LongMemEval empties (3 of 470 against 6), higher
LongMemEval recall@5 and MRR, and one more no-answer item abstaining on the gold set (33 of 57 rows against 30). The margins are small and in-sample (see [Limits](#limits)). **The owner adopted it
as the shipped default on 2026-11-08.**

## Variants

`k` is a count of query terms; the query's terms are ranked by IDF weight, ties broken by term.

| Variant | Floor coverage | Equal to plain when |
| --- | --- | --- |
| `plain` (LAMP-23 default) | matched IDF weight / weight of all query terms | always |
| `salient:<k>` | matched weight among the top `k` terms / weight of the top `k` terms | the query has `k` terms or fewer |
| `capped:<k>` | all matched weight / weight of the top `k` terms, capped at 1 | the query has `k` terms or fewer |

`salient` ignores matches on the lower-weight terms. `capped` keeps them in the numerator, so a memory matching the two rarest terms of a 12-term question scores as if the question were about three
terms long.

The switch is bench-only, on `lampl serve`, next to `LAMPL_RECALL_FLOOR`:

```bash
LAMPL_RECALL_COVERAGE=salient:5 LAMPL_RECALL_FLOOR=0.52,0.31 lampl serve ...   # salient:<k>, capped:<k> or plain
LAMPL_RECALL_COVERAGE=plain LAMPL_RECALL_FLOOR=0.52,0.35 lampl serve ...       # the LAMP-23 default, exactly
```

Unset keeps the shipped default, `capped:3` at 0.52 / 0.52 (`Floor::default()`). An unparsable value is logged as a warning and ignored. Local mode (SQLite) applies the same default floor and
has no switch. While the grid below was measured the default was plain 0.52 / 0.35, and the variants were served with the switch set.

## Method

- **LongMemEval:** the stored full run ([public/README.md](public/README.md#restarting-the-stored-run-for-a-floor-sweep), 470 questions, 116,728 memories, Metal build). Median query length 7 terms after stop
  words, maximum 24.
- **Gold set:** the 138-item gate set on the 2,669-memory snapshot (memory-pg, snapshot 2026-11-08T14:50Z), scored like homelab's LAB-693 gate: hook recall@5 over the answerable items and
  **negatives empty** as rows of 57 (19 no-answer items in three case forms). Recall is case-invariant at this build, so 3 rows are one item.
- One pass per workload against a server with the floor off (limit 100) records every fused candidate's cosine and coverage. Postgres then gives each question's lexemes, their document frequencies
  and which lexemes each candidate matches, from which every variant is computed offline. The recomputed plain coverage equals the server's to within 4e-16 on every hit of both workloads.
- Grid: cosine 0.48 to 0.60 in steps of 0.02, coverage 0.30 to 0.70 in steps of 0.01, for `plain`, `salient` and `capped` at k = 3, 4, 5, 6 and 8.
- Baseline check: the default 0.52 / 0.35 reproduces the LAMP-23 figures exactly, offline (checked before any variant was read) and on live servers of the grid build with the switch unset (gate
  recall@5 0.9592, 30 of 57 negatives empty, 10 of 19 items, case variance 0; LongMemEval 0.949 / 0.921 / 6 empty, median 5 results).
- The chosen point was then served live with the switch set and matched the offline figures to every digit: LongMemEval 0.955 / 0.926 / 3 empty (median 8 results) through
  `lampl-bench run --reuse-run`, gate 0.9613 / 33 of 57 (11 of 19 items, case variance 0) through homelab's gate harness with its adapter floor opened so the server's floor decides. These two
  passes and the LongMemEval default pass used a CPU build, the gate baseline a Metal build; both embed with the same model digest.

## Result

Each cell: gold recall@5 / negatives empty (of 57), then LongMemEval recall@5 / MRR / questions returning nothing (of 470).

Best point per variant that keeps the gate (gold negatives empty at least 30, as the default has, and gold recall@5 at least memorygraph's 0.9487), chosen for the fewest LongMemEval empties:

| Variant | cosine / coverage | Gold | LongMemEval |
| --- | --- | --- | --- |
| **plain, LAMP-23 default** | **0.52 / 0.35** | **0.9592 / 30** | **0.949 / 0.921 / 6** |
| plain | 0.56 / 0.34 | 0.9571 / 30 | 0.947 / 0.919 / 3 |
| salient:3 | 0.52 / 0.32 | 0.9508 / 30 | 0.934 / 0.906 / 10 |
| salient:4 | 0.52 / 0.31 | 0.9592 / 30 | 0.947 / 0.919 / 6 |
| salient:5 | 0.52 / 0.31 | 0.9592 / 30 | 0.957 / 0.929 / 4 |
| salient:6 | 0.56 / 0.34 | 0.9571 / 30 | 0.943 / 0.915 / 8 |
| salient:8 | 0.56 / 0.34 | 0.9571 / 30 | 0.938 / 0.912 / 5 |
| capped:3 | 0.52 / 0.50 | 0.9613 / 30 | 0.960 / 0.932 / 2 |
| capped:4 | 0.52 / 0.42 | 0.9613 / 30 | 0.957 / 0.928 / 3 |
| capped:5 | 0.56 / 0.40 | 0.9613 / 30 | 0.936 / 0.914 / 4 |
| capped:6 | 0.56 / 0.35 | 0.9613 / 30 | 0.951 / 0.925 / 3 |
| capped:8 | 0.56 / 0.34 | 0.9613 / 30 | 0.949 / 0.921 / 3 |

`capped:3`, cosine 0.52, varying coverage:

| coverage | Gold | LongMemEval |
| --- | --- | --- |
| 0.35 | 0.9613 / 12 | 0.981 / 0.944 / 1 |
| 0.40 | 0.9613 / 18 | 0.972 / 0.937 / 1 |
| 0.45 | 0.9613 / 21 | 0.970 / 0.936 / 1 |
| 0.49 | 0.9613 / 21 | 0.962 / 0.933 / 2 |
| 0.50 | 0.9613 / 30 | 0.960 / 0.932 / 2 |
| 0.51 | 0.9613 / 30 | 0.957 / 0.929 / 2 |
| **0.52** | **0.9613 / 33** | **0.955 / 0.926 / 3** |
| 0.53 | 0.9613 / 33 | 0.953 / 0.924 / 4 |
| 0.54 | 0.9613 / 33 | 0.949 / 0.920 / 5 |
| 0.55 | 0.9613 / 33 | 0.943 / 0.916 / 7 |
| 0.56 | 0.9613 / 33 | 0.936 / 0.913 / 9 |
| 0.60 | 0.9613 / 33 | 0.919 / 0.901 / 13 |

`capped:3`, coverage 0.52, varying cosine:

| cosine | Gold | LongMemEval |
| --- | --- | --- |
| 0.48 | 0.9655 / 21 | 0.957 / 0.927 / 3 |
| 0.50 | 0.9655 / 21 | 0.955 / 0.926 / 3 |
| **0.52** | **0.9613 / 33** | **0.955 / 0.926 / 3** |
| 0.56 | 0.9613 / 33 | 0.949 / 0.920 / 3 |
| 0.60 | 0.9571 / 33 | 0.945 / 0.916 / 3 |

Reading the tables:

- `salient` loses at every k: dropping matches on the lower-weight terms takes coverage away from relevant LongMemEval rounds as well as from noise, so it never beats the default on both workloads.
- `capped` helps, and more at small k. At the default's own abstention (30 of 57) `capped:3` returns nothing on 2 questions instead of 6; at 33 of 57 it returns nothing on 3, where plain coverage
  needs 6 empties and LongMemEval recall@5 0.940 (plain 0.56 / 0.35) to abstain as often.
- Re-tuning the plain floor recovers part of the LongMemEval gain (plain 0.56 / 0.34: 3 empties) but at lower recall on both workloads (0.947 and 0.9571) and no more abstention.
- `capped:3` sits on plateaus at 0.52: coverage 0.52 to 0.54 keeps 33 of 57 and 3 to 5 empties, and cosine 0.52 to 0.58 keeps 33 of 57 and 3 empties. The abstention cliff is below it, at coverage
  0.49 to 0.50 (21 to 30 of 57) and at cosine 0.50 to 0.52 (21 to 33).

## Recommendation

`capped:3` with the floor at 0.52 / 0.52, which against the LAMP-23 default gives: LongMemEval empties 6 to 3 of 470, recall@5 0.949 to 0.955, MRR 0.921 to 0.926; gold negatives empty 30 to 33
of 57 (10 to 11 of 19 items), gate recall@5 0.9592 to 0.9613.

## Adoption

The owner accepted the recommendation on 2026-11-08 (LAMP-22). `Floor` now carries its `coverage_norm`, and `Floor::default()` is cosine 0.52, coverage 0.52, `capped:3`, so both stores and
both modes use it without opting in. Confirmed on a CPU build of that change with no `LAMPL_RECALL_*` variable set, against the stored LongMemEval run: 0.955 / 0.926 / 3 empty
(median 8 results), identical to the switched pass above; the same build with `LAMPL_RECALL_COVERAGE=plain LAMPL_RECALL_FLOOR=0.52,0.35` gave 0.949 / 0.921 / 6, the LAMP-23 figures.
On the gate (snapshot of memory-pg 2026-11-08T16:16:21Z, 2,669 memories, embedded on that build, homelab's harness with its adapter floor opened so the server's floor decides): the
default build gives recall@5 0.9613 and 33 of 57 negatives empty (11 of 19 items; recurrence 0.9686, update 0.9176), and the override gives 0.9592 and 30 of 57 (10 of 19), both identical to the grid.

## Limits

- In-sample: both workloads chose the point and both report it. The grid held 3,157 points per workload, so the best of them is biased upwards.
- The differences are small: 3 LongMemEval questions and one gold no-answer item (3 rows), with 19 no-answer items in the gold set.
- `capped:k` changes only queries longer than k terms. After stop words, gold hook queries have a median of 9 terms (137 of 138 longer than three) and LongMemEval questions a median of 7.
- Item-level gold results are private (homelab); only these aggregates are published.

## Reproduce

Build with the switch, then per workload: one floor-off pass with limit 100 to record candidates, the per-candidate lexeme matches from Postgres, and the grid computed offline. The dump and grid
scripts used here are not in the repository; the aggregates above are their output. A chosen point is checked by serving it:

```bash
LAMPL_RECALL_COVERAGE=capped:3 LAMPL_RECALL_FLOOR=0.52,0.52 ./target/release/lampl serve --database-url $DB --listen 127.0.0.1:7883 &
./target/release/lampl-bench run --subset full --reuse-run <run id> --server http://127.0.0.1:7883 --checkpoint capped3.jsonl
```
