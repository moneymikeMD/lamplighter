# Ranking

Recall is hybrid: Postgres `tsvector` full-text and pgvector cosine (`halfvec(1024)`, `<=>`) each return their top 50 candidates among non-superseded memories, which are merged by reciprocal-rank fusion
(`score = Σ 1 / (60 + rank)`, in `lampl-core::rrf_fuse`), then cut by a relevance floor. The survivors are ordered by cosine plus IDF coverage (`lampl-core::relevance`); fusion only chooses the candidates. A memory that is the target of a `SUPERSEDES` link is excluded from both arms.

The full-text arm ORs the query's lexemes and orders by `ts_rank_cd`. Postgres takes the lexemes from `to_tsvector('english', query)`: stop words dropped, stemmed, deduplicated, a dotted host such as `a.example.com` kept whole. The SQLite store builds the same term list in `lampl-core::query_terms` (the same stop-word list, Snowball English stemming, dotted tokens kept), and a test checks it against Postgres's lexemes, so the stores do not drift. Hyphenated and URL tokens are not covered by that test. A stop word can never match, so it carries no coverage weight.

Without a query embedding, recall runs full-text only.

The query is lower-cased, so recall does not depend on query case: full-text matching already folds case, and the embedding leg embeds `Instruct: <task>\nQuery:` followed by the lower-cased query, the form Qwen3-Embedding is trained
for. Stored memories are embedded as written, without an instruction, so changing either does not need a re-embed. On the private 138-item gold set the top 5 changed with query case on 56 items before this change and on none after.

## Relevance floor

A fused candidate is kept when `cosine >= Floor.cosine` **or** its floor coverage `>= Floor.coverage`, where the floor coverage is computed under `Floor.coverage_norm`.

- `cosine` is `1 - (embedding <=> query)`; absent when the query carries no embedding.
- `coverage` is the share of the query's total IDF weight carried by the query terms the memory matches. `idf(N, df) = ln((N - df + 0.5) / (df + 0.5) + 1)`, with `N` and `df` counted over the active store
  (scoped to the project when one is given). This plain coverage orders the survivors and is the coverage reported on each hit.
- The floor coverage under the default `capped:3` is all the matched IDF weight over the weight of the query's three highest-IDF terms, capped at 1. It equals plain coverage for a query of three
  terms or fewer and is never below it ([LAMP-22](#normalised-coverage-lamp-22)).

## Calibration

The default floor is `cosine 0.52`, `coverage 0.52` with `capped:3` coverage (LAMP-22), chosen against the private gold set and the public LongMemEval run together; see [Calibration (LAMP-22,
owner-approved 2026-11-08)](#calibration-lamp-22-owner-approved-2026-11-08). **Both figures are in-sample: the floor was tuned on the same gold set and the same LongMemEval run that report it.** The
LAMP-23 default it replaced, `cosine 0.52`, plain `coverage 0.35`, is kept below as history, as are the earlier calibrations (LAMP-19, then LAMP-21). Those two were measured before the LAMP-23 fix to
how query terms are built, so their coverage figures are not comparable to the current ones and their defaults (0.70 / 0.24, then 0.70 / 0.18) are no longer in use. The earlier `0.5 / 0.5` was an
untested placeholder, and the production memorygraph numbers (0.58 / 0.5) were tuned on different vectors and a different coverage definition, so they are not copied.

### Data

A private gold set of 138 real hook-style queries against a store of 2,658 memories: 102 recurrence (one or more relevant memories), 17 update (a newer memory replaces an older claim), 19 abstain (no relevant memory). Only the aggregates below are published; items, memory text and per-item results are kept private. Queries are cut at 400 characters, recall is unscoped by project, and no `SUPERSEDES` links exist in the corpus.

### Method

1. Load the store into a scratch `lampl-store-pg` and embed with `lampl-embed` in-process (Qwen3-Embedding-0.6B, no instruction prefix; memories as title, blank line, body, 512 tokens).
2. For every query, run recall with the floor open and a limit of 100, recording each fused candidate's cosine and coverage in rank order. Applying a floor then keeping the first five survivors is exactly what the hook's `limit 5` recall does, so one dump serves the whole sweep.
3. Sweep `cosine` over 0.30..0.80 and `coverage` over 0.20..0.80 in steps of 0.02, plus each arm switched off.
4. Metrics: **recall@5** is the mean over the 119 answerable queries of the share of relevant memories in the top five. **Abstain-correct** is the number of the 19 no-answer queries that return nothing. **Abstain precision** is the share of empty results that are no-answer queries.
5. Choose the pair that maximises abstain-correct while recall@5 stays within 0.01 of the no-floor run. Ties go to the centre of the cosine plateau, then to the highest recall@5. This produced (0.70, 0.24), the default until LAMP-21.

### Result

LAMP-19 result. No floor: recall@5 0.828, abstain-correct 0 of 19. LAMP-19's floor (0.70, 0.24): recall@5 0.840, abstain-correct 9 of 19, abstain precision 0.75, and 3 of the 119 answerable queries return nothing. Floors cut some noise from the top five, so recall@5 rises slightly instead of falling.

Sweep excerpt, recall@5 / abstain-correct (of 19), Qwen3 in-process; `off` disables that arm:

| cosine \ coverage | 0.20 | 0.22 | 0.24 | 0.30 | 0.50 | off |
|---|---|---|---|---|---|---|
| 0.50 | 0.855 / 0 | 0.855 / 0 | 0.855 / 0 | 0.855 / 0 | 0.855 / 0 | 0.855 / 0 |
| 0.60 | 0.842 / 4 | 0.845 / 7 | 0.851 / 7 | 0.834 / 8 | 0.821 / 9 | 0.813 / 9 |
| 0.66 | 0.843 / 5 | 0.843 / 8 | 0.847 / 8 | 0.798 / 9 | 0.777 / 13 | 0.743 / 15 |
| 0.68 | 0.832 / 6 | 0.828 / 9 | 0.840 / 9 | 0.789 / 10 | 0.742 / 15 | 0.700 / 17 |
| **0.70** | 0.840 / 6 | 0.836 / 9 | **0.840 / 9** | 0.765 / 10 | 0.666 / 15 | 0.607 / 17 |
| 0.72 | 0.840 / 6 | 0.836 / 9 | 0.840 / 9 | 0.753 / 10 | 0.616 / 16 | 0.536 / 18 |
| 0.74 | 0.836 / 6 | 0.832 / 9 | 0.833 / 9 | 0.737 / 10 | 0.557 / 16 | 0.473 / 18 |
| 0.80 | 0.828 / 6 | 0.824 / 9 | 0.823 / 9 | 0.678 / 10 | 0.353 / 17 | 0.219 / 19 |
| off | 0.811 / 6 | 0.798 / 9 | 0.777 / 9 | 0.608 / 10 | 0.164 / 17 | 0.000 / 19 |

Reading the table:

- Cosine 0.68 to 0.72 gives the same result at coverage 0.24, so 0.70 sits on a plateau, not a cliff.
- Abstaining on more than 9 of 19 costs more than 0.01 of recall@5 at every pair tried (checked on both the Qwen3 and Ollama runs): the recall constraint, not the sweep range, is what stops at 9.
- The same sweep on the live store's own Ollama Q8_0 vectors (queries embedded through an Ollama endpoint) gives the same plateau and the same chosen pair: no floor recall@5 0.815, floor (0.70, 0.24) recall@5 0.829, abstain-correct 9 of 19. The floor therefore holds across both runtimes of the same model.

### Re-calibration against LongMemEval (LAMP-21)

The full 470-question LongMemEval run ([docs/bench/public](bench/public/README.md); 116,728 memories, one project per question, every question answerable) showed that (0.70, 0.24) returns nothing on 176 of 470 questions (37%), with recall@5 0.591 against 0.977 with the floor off. The hook-style gold set above and LongMemEval want different things: LongMemEval questions are long sentences, which dilute IDF coverage, and every one has an answer, so an empty result is always a miss. LAMP-21 swept the floor on both and chose the pair that keeps most of the abstentions while cutting the LongMemEval empty rate.

Cosine 0.70, varying coverage. LongMemEval: recall@5 / MRR / questions returning nothing (of 470) / median rounds returned. Gold: recall@5 / abstain-correct (of 19) / abstain precision.

| coverage | LongMemEval | Gold set |
|---|---|---|
| off (no floor) | 0.977 / 0.914 / 0 / 83 | 0.828 / 0 / none |
| 0.12 | 0.934 / 0.895 / 6 / 6 | 0.870 / 0 / none |
| 0.14 | 0.906 / 0.872 / 17 / 4 | 0.845 / 1 / 1.00 |
| 0.16 | 0.864 / 0.840 / 40 / 3 | 0.852 / 3 / 1.00 |
| **0.18 (default)** | **0.811 / 0.796 / 64 / 2** | **0.845 / 5 / 0.83** |
| 0.20 | 0.757 / 0.745 / 96 / 2 | 0.840 / 6 / 0.86 |
| 0.22 | 0.674 / 0.667 / 138 / 1 | 0.836 / 9 / 0.82 |
| 0.24 (LAMP-19) | 0.591 / 0.588 / 176 / 1 | 0.840 / 9 / 0.75 |

Cosine, at coverage 0.18:

| cosine | LongMemEval recall@5 / empty | Gold abstain-correct / recall@5 |
|---|---|---|
| 0.50 | 0.883 / 28 | 0 / 0.855 |
| 0.60 | 0.830 / 59 | 3 / 0.849 |
| 0.65 | 0.817 / 62 | 4 / 0.854 |
| **0.70** | **0.811 / 64** | **5 / 0.845** |

Disabling the coverage arm (cosine only) is worse on both workloads: at cosine 0.60 it returns nothing on 292 questions, and it only abstains well at cosines (0.70: 17 of 19) that cost gold recall@5 (0.607).

Method. The floor is applied to the ranked candidate list after fusion, so one pass with the floor open (limit 100) gives every setting exactly; checked by running servers with `LAMPL_RECALL_FLOOR` at three settings, which reproduced the simulation to every digit. LongMemEval used the stored run with `lampl-bench run --reuse-run`; the gold set used the LAMP-19 candidate dump (same Qwen3 vectors). At the LAMP-21 default both were re-run: LongMemEval 0.811 recall@5, 64 empty, from a live server; gold 0.845, 5 of 19, precision 0.83.

Trade-off. The LAMP-21 default gave up 4 of the 9 abstentions LAMP-19 reached (9 of 19 down to 5 of 19) to cut the LongMemEval empty rate from 37% to 14% and lift its recall@5 from 0.591 to 0.811. Coverage 0.20 keeps 6 of 19 but leaves 96 empty (20%); no coverage below 0.20 abstains more than 3 of 19. The gold set has 19 no-answer queries, so one query moves abstain-correct by about 5 points. A length-normalised coverage might separate the two workloads better; LAMP-22 tested it later ([below](#normalised-coverage-lamp-22)).

`LAMPL_RECALL_FLOOR` (`off` or `<cosine>,<coverage>`) on `lampl serve` overrides the default thresholds for benchmarking and keeps the coverage norm; unset keeps the default above.
`LAMPL_RECALL_COVERAGE` (`plain`, `salient:<k>` or `capped:<k>`) changes how the floor's coverage is computed, also for benchmarking only ([LAMP-22](#normalised-coverage-lamp-22)).
`LAMPL_RECALL_COVERAGE=plain LAMPL_RECALL_FLOOR=0.52,0.35` reproduces the LAMP-23 default exactly. Local mode has no override.

### Re-calibration after the coverage fix (LAMP-23)

History: this was the default from LAMP-23 until LAMP-22's `capped:3` floor replaced it on 2026-11-08.

Two defects distorted the floor until LAMP-23. Query terms kept stop words, which can never match a memory yet carried the highest IDF weight in the coverage denominator, so coverage ran well below memorygraph's; and the gate set (below) showed recall that varied with query case. With stop words dropped (the term list now matches Postgres's `to_tsvector('english', ..)`), case folded, a Qwen3 query instruction and survivors ordered by cosine plus coverage, coverage values run higher and the old 0.70 / 0.18 floor became nearly inert (gate recall@5 0.9655 but only 9 of 57 negatives empty).

Data. The gate set is the 138-item gold set above grown to 2,669 memories, scored like homelab's LAB-693 gate: hook recall@5 over the 119 answerable items, and **negatives empty** as rows of 57 (19 no-answer items in three case forms). The gate passes when recall@5 is at least memorygraph's production 0.9487 **and** negatives empty are at least its 27 of 57. LongMemEval is the full 470-question run, retrieval only.

Method. One dump with the floor open (limit 100) gives every setting exactly, because the floor is a per-hit filter after fusion; the chosen default was then confirmed on live servers at the final build. The grid was cosine 0.52 to 0.60 against coverage 0.18 to 0.26 and 0.50, widened to coverage 0.27 to 0.60 and cosine 0.48 to 0.70 when no point of the first grid met both gate criteria with a small LongMemEval cost.

Each cell: gate recall@5 / gate negatives empty (of 57), then LongMemEval recall@5 / MRR / questions returning nothing (of 470).

Cosine 0.52, varying coverage:

| coverage | gate | LongMemEval |
|---|---|---|
| 0.18 | 0.9655 / 9 | 0.983 / 0.945 / 0 |
| 0.20 | 0.9655 / 15 | 0.983 / 0.945 / 0 |
| 0.24 | 0.9613 / 21 | 0.974 / 0.940 / 1 |
| 0.30 | 0.9613 / 21 | 0.964 / 0.933 / 2 |
| 0.32 | 0.9613 / 21 | 0.962 / 0.933 / 3 |
| 0.34 | 0.9613 / 27 | 0.955 / 0.927 / 3 |
| **0.35 (LAMP-23 default)** | **0.9592 / 30** | **0.949 / 0.921 / 6** |
| 0.36 | 0.9592 / 30 | 0.938 / 0.915 / 7 |
| 0.40 | 0.9592 / 30 | 0.919 / 0.898 / 16 |
| 0.45 | 0.9592 / 36 | 0.889 / 0.871 / 30 |
| 0.50 | 0.9592 / 36 | 0.845 / 0.830 / 53 |
| off (no floor) | 0.9571 / 0 | 0.987 / 0.949 / 0 |

Coverage 0.35, varying cosine:

| cosine | gate | LongMemEval |
|---|---|---|
| 0.50 | 0.9655 / 24 | 0.953 / 0.926 / 6 |
| **0.52 (LAMP-23 default)** | **0.9592 / 30** | **0.949 / 0.921 / 6** |
| 0.56 | 0.9529 / 33 | 0.940 / 0.914 / 6 |
| 0.60 | 0.9445 / 36 | 0.934 / 0.909 / 8 |
| 0.70 | 0.9249 / 36 | 0.934 / 0.907 / 8 |

Other reference points on the same build: the previous default 0.70 / 0.18 gives 0.9655 / 9 on the gate and 0.981 / 0.943 / 0 on LongMemEval; memorygraph's own floor (cosine 0.58 or coverage 0.50) gives 0.9333 / 48 and 0.802 / 0.790 / 68.

Choice. The gate passes at 0.52 / 0.34 (27 of 57 negatives, exactly the threshold) and 0.52 / 0.35 (30 of 57). The LAMP-23 default was 0.52 / 0.35 for one item of margin on the 19 no-answer items; the
0.34 to 0.35 plateau is flat across cosine 0.52 to 0.58, so it is not a knife edge. Against memorygraph on the gate (recall@5 0.9592 vs 0.9487, negatives empty 30 of 57 vs 27 of 57); on LongMemEval it
returns nothing on 6 of 470 questions (1%) with recall@5 0.949, MRR 0.921 and a median of 5 results, against 0.987 / 0.949 with the floor off.

Limits of this tuning. Floor and measurement share the same 138 items and the same 470 questions, so these are in-sample figures and will not hold exactly on new data. With 19 no-answer items one item moves negatives empty by 3 rows of 57. `single-session-preference` remains the weakest LongMemEval type (recall@5 0.767).

### Normalised coverage (LAMP-22)

LAMP-22 tested whether normalising coverage for query length separates the two workloads better than plain coverage, against the LAMP-23 default; method and grid in
[bench/coverage-normalisation.md](bench/coverage-normalisation.md). Two variants were tried behind the bench-only `LAMPL_RECALL_COVERAGE` switch, which changes only the coverage the floor tests:
`salient:<k>` (coverage over the `k` highest-IDF query terms) and `capped:<k>` (all matched weight over the weight of the top `k` terms, capped at 1). `salient` never beat the default on both
workloads. `capped:3` at 0.52 / 0.52 did on every measure: LongMemEval 0.955 / 0.926 / 3 empty against 0.949 / 0.921 / 6, gate 0.9613 / 33 of 57 against 0.9592 / 30. The margins are three LongMemEval
questions and one no-answer item, in-sample; the owner adopted the variant as the default (below).

### Calibration (LAMP-22, owner-approved 2026-11-08)

The default floor is cosine 0.52 or `capped:3` coverage 0.52, in both stores (`Floor::default()`; server and local mode alike). The owner approved it on 2026-11-08 on the LAMP-22 grid. Against the
LAMP-23 default (plain 0.52 / 0.35): LongMemEval recall@5 0.949 to 0.955, MRR 0.921 to 0.926, questions returning nothing 6 to 3 of 470; gate recall@5 0.9592 to 0.9613, negatives empty 30 to 33
of 57 (10 to 11 of 19 no-answer items). **In-sample, with small margins:** the point is the best of 3,157 grid points on the same LongMemEval run and gold set that report it, and the gain is 3
LongMemEval questions and 1 no-answer gold item. It sits on a plateau (coverage 0.52 to 0.54, cosine 0.52 to 0.58) with an abstention cliff just below it at coverage 0.49 to 0.50. Ordering and
the coverage reported on hits are unchanged. `LAMPL_RECALL_COVERAGE=plain LAMPL_RECALL_FLOOR=0.52,0.35` on `lampl serve` restores the LAMP-23 floor for comparison.

### Limits

- The gold set has 19 no-answer queries, so abstain-correct moves in steps of about 5 points; one query flips 9 to 10.
- The corpus is one author's store; recalibrate if the typical memory or query style changes.
- `halfvec` storage versus `float32` was not measured separately, and the SQLite store was not calibrated here (see `local-mode.md`).
