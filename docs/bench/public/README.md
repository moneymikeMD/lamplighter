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

`--subset ci` is 6 questions (one per question type, chosen by a fixed-seed shuffle so the same run picks the same questions) with each haystack cut to 12 sessions: every evidence session plus a seeded sample of the rest, which makes retrieval easier than the real 48-session haystack; `--subset full` is all 470 questions with every session (`--haystack N` overrides the cut); `--limit N` and `--seed S` override. The dataset (277 MB) is downloaded to the OS cache directory on first use; `lampl-bench fetch` does only that. `--checkpoint FILE` appends each scored question to a JSONL file and, on restart with the same options, skips the questions already in it (the store may then be non-empty); the file's first line fingerprints the run and a mismatch is refused.

On Apple Silicon build the server with `cargo build --release -p lampl --features metal`: on a 2-question full-haystack slice (484 memories) Metal embeds at about 0.21 s per memory including ingest, against 3.75 s on the M2's CPU, with vectors matching CPU at cosine above 0.9999. If the server's embedder is down the run stops, because it would measure full-text only; `--allow-full-text` accepts that and the output then says `recall mode full-text`.

## CI

`.github/workflows/bench.yml` runs `--subset ci` nightly and on manual dispatch (`workflow_dispatch`, with `subset` ci or full), not on pull requests: it takes about 30 minutes on CPU and its six questions cannot detect a quality change. Pull requests are covered by the ordinary `rust` and `store-pg` jobs, which build and test `lampl-bench`. The full set is never run in CI.

## Comparison arms

`Retriever` in `crates/lampl-bench/src/retriever.rs` is the interface another system implements (`ingest`, `settle`, `rank`). No competitor arm (mem0, Graphiti, Cognee) is implemented or run. They spend model tokens on ingestion (the private LAB-345 comparison spent about $20 on mem0 and $76 on Graphiti for 1,469 memories), so an arm is run only with the owner's explicit approval.

## Results

### Smoke test (`--subset ci`)

One run on 2026-11-06, `--subset ci` (6 questions, one per type, 12 sessions each, 354 memories), Qwen3-Embedding-0.6B in-process on an Apple M-series laptop, recall mode hybrid. It took 23 minutes, almost all of it embedding. Six questions is a smoke test, not an estimate: one question moves a figure by 0.167. Treat these numbers as a check that the harness works end to end, not as a quality claim.

```
                             n  any@1    any@3    any@5    any@10   all@1    all@3    all@5    all@10      MRR
overall                      6   0.667   0.667   0.667   0.667   0.333   0.500   0.500   0.500   0.667
```

recall@5 0.667, MRR 0.667. The full set is reported below. The CI job's runtime on GitHub's runners has not been measured (UNVERIFIED); its `timeout-minutes` is 90.

### Full run, all 470 questions

Run on 2026-11-07, scored at the memory level (one memory per round, mapped back to its session).

| Setting | Value |
| --- | --- |
| Dataset | LongMemEval-S cleaned, revision `98d7416c24c778c2fee6e6f3006e7a073259d48f` (`longmemeval_s_cleaned@98d7416c`) |
| Questions | 470 (`--subset full`), seed 20261106 |
| Haystack | all sessions per question, 116,728 memories in total, one project per question |
| Embedder | Qwen3-Embedding-0.6B, in-process, candle on Metal (Apple M2, 16 GB), input truncated at 512 tokens |
| Recall | hybrid (full-text plus cosine, reciprocal-rank fusion), request limit 100 |
| Default floor (LAMP-23) | keep a candidate when cosine >= 0.52 or IDF coverage >= 0.35. **In-sample: tuned on this same run and on the private gold set** |
| Earlier default floors | cosine >= 0.70 with coverage >= 0.24 (until LAMP-21), then coverage >= 0.18 (LAMP-21 to LAMP-23) |
| Query handling (LAMP-23) | query lower-cased, Qwen3 task instruction prefixed to the query embedding, stop words dropped from the coverage terms, survivors ordered by cosine plus coverage; stored memories unchanged |
| Floor off | `LAMPL_RECALL_FLOOR=off` on the server (cosine and coverage thresholds removed); the vectors are the same, queries were re-run against the stored memories, nothing was re-embedded |

The first full run (before LAMP-23) used the then-default floor; re-running its queries with that floor reproduced it to every printed digit. For LAMP-23 the queries were run again at the final build against the same stored memories (nothing re-embedded): once on a server with no override (the default floor) and once with the floor off. The earlier runs are kept below as history.

Default floor (cosine 0.52 or coverage 0.35; in-sample):

```
                             n  any@1    any@3    any@5    any@10   all@1    all@3    all@5    all@10      MRR
overall                    470   0.898   0.945   0.949   0.949   0.313   0.747   0.766   0.777   0.921
knowledge-update            72   0.986   1.000   1.000   1.000   0.000   0.958   0.958   0.958   0.993
multi-session              121   0.926   0.959   0.967   0.967   0.000   0.545   0.587   0.612   0.944
single-session-assistant    56   1.000   1.000   1.000   1.000   1.000   1.000   1.000   1.000   1.000
single-session-preference   30   0.533   0.733   0.767   0.767   0.533   0.733   0.767   0.767   0.640
single-session-user         64   0.953   1.000   1.000   1.000   0.953   1.000   1.000   1.000   0.977
temporal-reasoning         127   0.835   0.898   0.898   0.898   0.110   0.583   0.606   0.622   0.864
```

Floor off (diagnostic, final build):

```
                             n  any@1    any@3    any@5    any@10   all@1    all@3    all@5    all@10      MRR
overall                    470   0.917   0.977   0.987   0.991   0.319   0.830   0.904   0.962   0.949
knowledge-update            72   0.986   1.000   1.000   1.000   0.000   0.986   1.000   1.000   0.993
multi-session              121   0.942   0.975   0.992   1.000   0.000   0.636   0.810   0.942   0.964
single-session-assistant    56   1.000   1.000   1.000   1.000   1.000   1.000   1.000   1.000   1.000
single-session-preference   30   0.633   0.867   0.933   0.933   0.633   0.867   0.933   0.933   0.758
single-session-user         64   0.953   1.000   1.000   1.000   0.953   1.000   1.000   1.000   0.977
temporal-reasoning         127   0.866   0.969   0.976   0.984   0.110   0.756   0.843   0.929   0.917
```

recall@5 is 0.949 with the default floor and 0.987 with the floor off; MRR is 0.921 and 0.949. The default returns nothing on 6 of 470 questions (1%).

#### Why the rows differ: the floor truncates

Results returned per question (rounds, then distinct sessions), final build:

| | Floor | Median rounds | Mean rounds | Questions with 0 rounds | Questions with 3 or fewer rounds | Median sessions |
| --- | --- | --- | --- | --- | --- | --- |
| All 470 | default (0.52 / 0.35) | 5 | 6.6 | 6 | 145 | 2 |
| All 470 | off | 82 | 80.5 | 0 | 0 | 32 |

#### History: runs before LAMP-23

These used the old term logic (stop words counted against coverage), so their coverage thresholds are not comparable to the current ones. On the same stored memories: coverage 0.70 / 0.18 gave recall@5 0.811 with 64 of 470 empty; 0.70 / 0.24 gave 0.591 with 176 empty (37%); floor off gave 0.977 (MRR 0.914).

Floor 0.70 / 0.24 (before LAMP-21):

```
                             n  any@1    any@3    any@5    any@10   all@1    all@3    all@5    all@10      MRR
overall                    470   0.585   0.591   0.591   0.591   0.232   0.383   0.383   0.383   0.588
knowledge-update            72   0.694   0.708   0.708   0.708   0.000   0.514   0.514   0.514   0.701
multi-session              121   0.463   0.471   0.471   0.471   0.000   0.140   0.140   0.140   0.467
single-session-assistant    56   0.821   0.821   0.821   0.821   0.821   0.821   0.821   0.821   0.821
single-session-preference   30   0.133   0.167   0.167   0.167   0.133   0.167   0.167   0.167   0.150
single-session-user         64   0.766   0.766   0.766   0.766   0.766   0.766   0.766   0.766   0.766
temporal-reasoning         127   0.551   0.551   0.551   0.551   0.079   0.205   0.205   0.205   0.551
```

Floor off, before LAMP-23:

```
                             n  any@1    any@3    any@5    any@10   all@1    all@3    all@5    all@10      MRR
overall                    470   0.868   0.957   0.977   0.985   0.289   0.783   0.868   0.930   0.914
knowledge-update            72   0.958   1.000   1.000   1.000   0.000   0.944   0.972   0.972   0.979
multi-session              121   0.909   0.967   0.983   0.992   0.000   0.603   0.785   0.901   0.942
single-session-assistant    56   0.982   0.982   1.000   1.000   0.982   0.982   1.000   1.000   0.987
single-session-preference   30   0.600   0.833   0.900   0.900   0.600   0.833   0.900   0.900   0.717
single-session-user         64   0.812   0.953   0.984   0.984   0.812   0.953   0.984   0.984   0.883
temporal-reasoning         127   0.819   0.945   0.961   0.984   0.087   0.677   0.764   0.882   0.880
```

The old 0.24 floor left 176 of 470 questions (37%) with nothing to rank, which is why `any@k` stopped moving after k = 3 in that run.

#### Caveats

- Retrieval only. There is no LLM answer step and no judge, so these numbers are not comparable to end-to-end LongMemEval QA accuracy.
- Scored at the memory level and mapped to sessions: a session counts as retrieved when any of its rounds ranks. `any@k` is over sessions.
- One run, one seed, one machine. No confidence interval was computed; with 30 to 127 questions per type, one question moves a by-type figure by 0.008 to 0.033.
- The floor-off row is a diagnostic, not a configuration to run: it returns about 80 results per question. The default floor was tuned on this run and on the private gold set together, so the default-floor figures are in-sample; on the gold set it returns nothing on 30 of 57 no-answer rows against the production memorygraph's 27 of 57, with hook recall@5 0.9592 against its 0.9487 (see [ranking.md](../../ranking.md)); one gold query moves that figure by about 5 points.
- Embedding input is truncated at 512 tokens, so a long round is represented by its start.
- Metal and CPU vectors agree to cosine above 0.9999 on a 24-text sample, not on the full corpus.

#### Reproduce

```bash
export LAMPL_KEYS_FILE=$(mktemp -d)/keys
export LAMPL_API_KEY=$(./target/release/lampl serve keys add bench)
./target/release/lampl serve --database-url postgres://postgres:bench@127.0.0.1:55432/postgres --listen 127.0.0.1:7878 &
./target/release/lampl-bench run --subset full --checkpoint full.jsonl --json-out full.json    # about 8 h on Metal

# same stored memories, floor off: a second server with the override, then re-rank without ingesting
LAMPL_RECALL_FLOOR=off ./target/release/lampl serve --database-url postgres://postgres:bench@127.0.0.1:55432/postgres --listen 127.0.0.1:7879 &
./target/release/lampl-bench run --subset full --reuse-run <run id> --server http://127.0.0.1:7879 --checkpoint floor-off.jsonl
```

Build both binaries with `--features lampl/metal` on Apple Silicon. The run id is the middle part of the `lme-<run id>-<question id>` project names. `LAMPL_RECALL_FLOOR` takes `off` or `<cosine>,<coverage>`; unset keeps the shipped default. It exists for benchmarking only.

#### Restarting the stored run for a floor sweep

The full run's store is kept so other floor values can be tried without re-embedding the documents (a 470-question pass takes about three minutes). It lives in a stopped pgvector container whose data volume must not be removed.

```bash
docker start lampl-bench-pg-full                      # the same port mapping and data volume come back
export LAMPL_KEYS_FILE=<keys file from the first run>   # keep it; a key is printed only once, by `serve keys add`
export LAMPL_API_KEY=<that key>
DB=postgres://postgres:bench@127.0.0.1:<mapped port>/postgres

./target/release/lampl serve --database-url $DB --listen 127.0.0.1:7881 &                              # default floor
LAMPL_RECALL_FLOOR=off ./target/release/lampl serve --database-url $DB --listen 127.0.0.1:7882 &       # floor off
LAMPL_RECALL_FLOOR=0.60,0.20 ./target/release/lampl serve --database-url $DB --listen 127.0.0.1:7883 &   # any <cosine>,<coverage>

./target/release/lampl-bench run --subset full --reuse-run <run id> --server http://127.0.0.1:7882 --checkpoint sweep-off.jsonl
```

The server must embed queries with the same model (same digest) as the stored vectors; rows from another digest are skipped by the vector arm. Metal and CPU builds use the same model. Stop the servers when done and `docker stop` the container; `docker rm` or `docker volume rm` deletes the roughly eight hours of embedding.
