# Ranking

Recall is hybrid: Postgres `tsvector` full-text and pgvector cosine (`halfvec(1024)`, `<=>`) each return their top 50 candidates among non-superseded memories, which are merged by reciprocal-rank fusion
(`score = Σ 1 / (60 + rank)`, in `lampl-core::rrf_fuse`), then cut by a relevance floor. A memory that is the target of a `SUPERSEDES` link is excluded from both arms.

The full-text arm ORs the stemmed query terms (`plainto_tsquery` with `&` rewritten to `|`) and orders by `ts_rank_cd`. Without a query embedding, recall runs full-text only.

## Relevance floor

A fused candidate is kept when `cosine >= Floor.cosine` **or** `coverage >= Floor.coverage`.

- `cosine` is `1 - (embedding <=> query)`; absent when the query carries no embedding.
- `coverage` is the share of the query's total IDF weight carried by the query terms the memory matches. `idf(N, df) = ln((N - df + 0.5) / (df + 0.5) + 1)`, with `N` and `df` counted over the active store
  (scoped to the project when one is given).

## Calibration status

**The default floor (`cosine 0.5`, `coverage 0.5`) is a placeholder** until the calibration follow-up ticket lands; it needs the private lab gold set, so it is not done here and no tuned values are claimed.
The production memorygraph numbers (0.58 / 0.5) were tuned on different vectors and are deliberately not copied.

Method for that ticket:

1. Embed the gold set's memories and queries with `lampl-embed` (Qwen3-Embedding-0.6B, candle, no instruction prefix).
2. Label each (query, memory) pair relevant or not, including queries with no relevant memory.
3. Sweep `cosine` over 0.30..0.80 and `coverage` over 0.20..0.80 in steps of 0.02; for each pair of values measure recall of relevant items and the rate at which no-answer queries return anything.
4. Choose the pair with the highest recall subject to a zero-result rate of at least 95% on no-answer queries; record the values, the data and the sweep table here.

The tests use hand-built orthogonal unit vectors, which prove the floor and fusion mechanics, not the threshold values. The `halfvec` recall-loss assumption is likewise unmeasured.
