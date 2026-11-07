# Ranking

Recall is hybrid: Postgres `tsvector` full-text and pgvector cosine (`halfvec(1024)`, `<=>`) each return their top 50 candidates among non-superseded memories, which are merged by reciprocal-rank fusion
(`score = Σ 1 / (60 + rank)`, in `lampl-core::rrf_fuse`), then cut by a relevance floor. A memory that is the target of a `SUPERSEDES` link is excluded from both arms.

The full-text arm ORs the stemmed query terms (`plainto_tsquery` with `&` rewritten to `|`) and orders by `ts_rank_cd`. Without a query embedding, recall runs full-text only.

## Relevance floor

A fused candidate is kept when `cosine >= Floor.cosine` **or** `coverage >= Floor.coverage`.

- `cosine` is `1 - (embedding <=> query)`; absent when the query carries no embedding.
- `coverage` is the share of the query's total IDF weight carried by the query terms the memory matches. `idf(N, df) = ln((N - df + 0.5) / (df + 0.5) + 1)`, with `N` and `df` counted over the active store
  (scoped to the project when one is given).

## Calibration

The default floor is `cosine 0.70`, `coverage 0.24`, calibrated on Qwen3-Embedding-0.6B vectors (LAMP-19). The earlier `0.5 / 0.5` was an untested placeholder: on these vectors it abstains on none of the no-answer queries, because Qwen3 cosines for unrelated text are already high. The production memorygraph numbers (0.58 / 0.5) were tuned on different vectors and are not copied.

### Data

A private gold set of 138 real hook-style queries against a store of 2,658 memories: 102 recurrence (one or more relevant memories), 17 update (a newer memory replaces an older claim), 19 abstain (no relevant memory). Only the aggregates below are published; items, memory text and per-item results are kept private. Queries are cut at 400 characters, recall is unscoped by project, and no `SUPERSEDES` links exist in the corpus.

### Method

1. Load the store into a scratch `lampl-store-pg` and embed with `lampl-embed` in-process (Qwen3-Embedding-0.6B, no instruction prefix; memories as title, blank line, body, 512 tokens).
2. For every query, run recall with the floor open and a limit of 100, recording each fused candidate's cosine and coverage in rank order. Applying a floor then keeping the first five survivors is exactly what the hook's `limit 5` recall does, so one dump serves the whole sweep.
3. Sweep `cosine` over 0.30..0.80 and `coverage` over 0.20..0.80 in steps of 0.02, plus each arm switched off.
4. Metrics: **recall@5** is the mean over the 119 answerable queries of the share of relevant memories in the top five. **Abstain-correct** is the number of the 19 no-answer queries that return nothing. **Abstain precision** is the share of empty results that are no-answer queries.
5. Choose the pair that maximises abstain-correct while recall@5 stays within 0.01 of the no-floor run. Ties go to the centre of the cosine plateau, then to the highest recall@5.

### Result

No floor: recall@5 0.828, abstain-correct 0 of 19. Chosen floor (0.70, 0.24): recall@5 0.840, abstain-correct 9 of 19, abstain precision 0.75, and 3 of the 119 answerable queries return nothing. Floors cut some noise from the top five, so recall@5 rises slightly instead of falling.

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

### Limits

- The gold set has 19 no-answer queries, so abstain-correct moves in steps of about 5 points; one query flips 9 to 10.
- The corpus is one author's store; recalibrate if the typical memory or query style changes.
- `halfvec` storage versus `float32` was not measured separately, and the SQLite store was not calibrated here (see `local-mode.md`).
