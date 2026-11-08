# Ranking

Recall is hybrid: Postgres `tsvector` full-text and pgvector cosine (`halfvec(1024)`, `<=>`) each return their top 50 candidates among non-superseded memories, which are merged by reciprocal-rank fusion
(`score = Σ 1 / (60 + rank)`, in `lampl-core::rrf_fuse`), then cut by a relevance floor. The survivors are ordered by cosine plus IDF coverage (`lampl-core::relevance`); fusion only chooses the candidates. A memory that is the target of a `SUPERSEDES` link is excluded from both arms.

The full-text arm ORs the stemmed query terms (`plainto_tsquery` with `&` rewritten to `|`) and orders by `ts_rank_cd`. Without a query embedding, recall runs full-text only.

The query is lower-cased, so recall does not depend on query case: full-text matching already folds case, and the embedding leg embeds `Instruct: <task>\nQuery:` followed by the lower-cased query, the form Qwen3-Embedding is trained
for. Stored memories are embedded as written, without an instruction, so changing either does not need a re-embed. On the private 138-item gold set the top 5 changed with query case on 56 items before and on none after;
hook recall@5 at the default floor moved from 0.894 to 0.906, and with the floor off from 0.908 to 0.951. LongMemEval at the default floor: recall@5 0.806 (was 0.811), MRR 0.800 (was 0.796), 66 of 470 queries empty (was 64).

## Relevance floor

A fused candidate is kept when `cosine >= Floor.cosine` **or** `coverage >= Floor.coverage`.

- `cosine` is `1 - (embedding <=> query)`; absent when the query carries no embedding.
- `coverage` is the share of the query's total IDF weight carried by the query terms the memory matches. `idf(N, df) = ln((N - df + 0.5) / (df + 0.5) + 1)`, with `N` and `df` counted over the active store
  (scoped to the project when one is given).

## Calibration

The default floor is `cosine 0.70`, `coverage 0.18`, calibrated on Qwen3-Embedding-0.6B vectors (LAMP-19, then re-tuned against LongMemEval in LAMP-21; see [Re-calibration](#re-calibration-against-longmemeval-lamp-21)). LAMP-19 chose coverage 0.24. The earlier `0.5 / 0.5` was an untested placeholder: on these vectors it abstains on none of the no-answer queries, because Qwen3 cosines for unrelated text are already high. The production memorygraph numbers (0.58 / 0.5) were tuned on different vectors and are not copied.

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

Method. The floor is applied to the ranked candidate list after fusion, so one pass with the floor open (limit 100) gives every setting exactly; checked by running servers with `LAMPL_RECALL_FLOOR` at three settings, which reproduced the simulation to every digit. LongMemEval used the stored run with `lampl-bench run --reuse-run`; the gold set used the LAMP-19 candidate dump (same Qwen3 vectors). At the new default both were re-run: LongMemEval 0.811 recall@5, 64 empty, from a live server; gold 0.845, 5 of 19, precision 0.83.

Trade-off. The new default gives up 4 of the 9 abstentions LAMP-19 reached (9 of 19 down to 5 of 19) to cut the LongMemEval empty rate from 37% to 14% and lift its recall@5 from 0.591 to 0.811. Coverage 0.20 keeps 6 of 19 but leaves 96 empty (20%); no coverage below 0.20 abstains more than 3 of 19. The gold set has 19 no-answer queries, so one query moves abstain-correct by about 5 points. A length-normalised coverage might separate the two workloads better; it was not tested.

`LAMPL_RECALL_FLOOR` (`off` or `<cosine>,<coverage>`) on `lampl serve` overrides the default for benchmarking; unset keeps the default above.

### Limits

- The gold set has 19 no-answer queries, so abstain-correct moves in steps of about 5 points; one query flips 9 to 10.
- The corpus is one author's store; recalibrate if the typical memory or query style changes.
- `halfvec` storage versus `float32` was not measured separately, and the SQLite store was not calibrated here (see `local-mode.md`).
