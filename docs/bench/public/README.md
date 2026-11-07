# Public retrieval benchmark

`lampl-bench` measures how well lamplighter retrieves the right past conversation for a question, on the public [LongMemEval](https://github.com/xiaowu0162/LongMemEval) dataset. Anyone can reproduce it; it needs no API key for any model provider. The licence finding for the data is in [datasets.md](datasets.md).

## What is measured

Retrieval only, not answer quality. For each question the harness:

1. Splits each of the question's roughly 48 haystack sessions into rounds (a user turn plus the assistant turns after it) and stores each round as one memory, tagged `session:<id>`, in a project of its own, so questions cannot see each other's haystacks. Dataset fields that mark the answer (`has_answer`) are never read.
2. Waits until the server's background worker has embedded every memory.
3. Calls `POST /v1/recall` with the question text (limit 100, the server's normal hybrid ranking and relevance floor) and ranks sessions by their best-ranked round.
4. Scores against the dataset's evidence sessions (`answer_session_ids`).

| Metric | Meaning |
| --- | --- |
| `any@k` | Share of questions with at least one evidence session in the top k sessions (k = 1, 3, 5, 10) |
| `all@k` | Share with every evidence session in the top k (differs from `any@k` only for multi-session questions) |
| MRR | Mean of 1 / rank of the first evidence session; 0 when none is returned |

There is no LLM judge. LongMemEval's own answer scoring needs a GPT-4o judge, costs money, and is not implemented here. Embedding input is truncated at 512 tokens by the in-process model, so a long round is represented by its start.

## Run it

The benchmark writes tens of thousands of memories and cannot delete them. Point it at a scratch store; it refuses a store that already holds memories unless `--allow-nonempty` is given.

```bash
docker run -d --name lampl-bench-pg -e POSTGRES_PASSWORD=bench -p 127.0.0.1:55432:5432 pgvector/pgvector:pg17
cargo build --release -p lampl -p lampl-bench
./target/release/lampl bench-embed --prepare --texts docs/bench/embed/texts-47.jsonl   # downloads the embedding model once

export LAMPL_KEYS_FILE=$(mktemp -d)/keys
export LAMPL_API_KEY=$(./target/release/lampl serve keys add bench)
./target/release/lampl serve --database-url postgres://postgres:bench@127.0.0.1:55432/postgres --listen 127.0.0.1:7878 &

./target/release/lampl-bench run --subset ci --json-out results.json
```

`--subset ci` is 6 questions (one per question type, chosen by a fixed-seed shuffle so the same run picks the same questions) with each haystack cut to 12 sessions: every evidence session plus a seeded sample of the rest, which makes retrieval easier than the real 48-session haystack; `--subset full` is all 470 questions with every session (`--haystack N` overrides the cut); `--limit N` and `--seed S` override. The dataset (277 MB) is downloaded to the OS cache directory on first use; `lampl-bench fetch` does only that. If the server's embedder is down the run stops, because it would measure full-text only; `--allow-full-text` accepts that and the output then says `recall mode full-text`.

## CI

`.github/workflows/bench.yml` runs `--subset ci` only for pull requests that touch ranking, embedding, storage or the harness paths, and on manual dispatch. It does not run on every push. The full set is never run in CI.

## Comparison arms

`Retriever` in `crates/lampl-bench/src/retriever.rs` is the interface another system implements (`ingest`, `settle`, `rank`). No competitor arm (mem0, Graphiti, Cognee) is implemented or run. They spend model tokens on ingestion (the private LAB-345 comparison spent about $20 on mem0 and $76 on Graphiti for 1,469 memories), so an arm is run only with the owner's explicit approval.

## Results

One run on 2026-11-06, `--subset ci` (6 questions, one per type, 12 sessions each, 354 memories), Qwen3-Embedding-0.6B in-process on an Apple M-series laptop, recall mode hybrid. It took 23 minutes, almost all of it embedding. Six questions is a smoke test, not an estimate: one question moves a figure by 0.167. Treat these numbers as a check that the harness works end to end, not as a quality claim.

```
                             n  any@1    any@3    any@5    any@10   all@1    all@3    all@5    all@10      MRR
overall                      6   0.667   0.667   0.667   0.667   0.333   0.500   0.500   0.500   0.667
```

recall@5 0.667, MRR 0.667. The full 470-question set has not been run; at the speed above it would take days of CPU, so it needs a faster embedder or a GPU. The CI job's runtime on GitHub's runners has not been measured (UNVERIFIED); its `timeout-minutes` is 90.
