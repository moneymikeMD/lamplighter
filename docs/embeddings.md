# Embeddings

lamplighter turns each memory and each query into a 1024-dimension vector and searches by cosine similarity next to full-text search. No embedding means no vector arm: recall still works, by full
text, and says so.

## In-process (default)

`lampl-embed` runs Qwen3-Embedding-0.6B (`Qwen/Qwen3-Embedding-0.6B`, Hugging Face revision `97b0c614be4d77ee51c0cef4e5f07c00f9eb65b3`) on CPU through candle, with F32 weights and no query instruction
prefix. There is no Ollama, no API key and no GPU requirement.

- **Weights:** about 1.1 GB, never bundled and never downloaded implicitly. `lampl init --local --prepare` or `lampl doctor --fetch-model` fetches them from Hugging Face into the platform cache
  directory under `lamplighter/models`.
- **Input:** title, a blank line, then body, truncated at 512 tokens; a long memory is represented by its start.
- **Who embeds:** in embedded mode the `lampl` client process; in server mode `lampl serve` (queries as they arrive, memories in a background sweep every `--embed-interval-secs`, default 5).
- **Memory:** about 2.5 GiB resident for a server with the model loaded, measured in a container.

Measured latency, one text per call, release build ([bench/embed/report.md](bench/embed/report.md)):

| Platform | short query p50 / p95 | prompt-length text p50 / p95 | model load |
| --- | --- | --- | --- |
| Apple M2, 8 threads | 117 / 168 ms | 372 / 463 ms | 3.7 s |
| GitHub Actions `ubuntu-latest`, 4 threads | 387 / 410 ms | 741 / 810 ms | 1.3 s |

Both are inside the hook's 1.5 second embedding budget at p95 for these texts. The text set is a public stand-in of 35 short queries and 12 prompt-length texts. Reproduce on your machine:

```sh
lampl bench-embed --texts docs/bench/embed/texts-47.jsonl --prepare
```

## HTTP endpoint (optional)

For a GPU host or a shared embedder, `lampl serve` can call an endpoint instead of loading the model:

```sh
lampl serve --embed-endpoint http://embedder.internal:11434 --embed-endpoint-kind ollama --embed-model qwen3-embedding:0.6b
```

| Flag | Meaning |
| --- | --- |
| `--embed-endpoint URL` | Base URL; replaces the in-process model |
| `--embed-endpoint-kind openai\|ollama` | Protocol: `POST {URL}/v1/embeddings` (default) or `POST {URL}/api/embed` |
| `--embed-model NAME` | Model name sent to the endpoint (default `Qwen/Qwen3-Embedding-0.6B`) |
| `LAMPL_EMBED_API_KEY` | If set, sent as `Authorization: Bearer ...` |
| `--fetch-tokenizer` | Download Qwen3-Embedding-0.6B's `tokenizer.json` (about 11 MB) into the model cache if it is not there, then serve |

### Input truncation

The endpoint gets the same input as the in-process model: title, a blank line, then body, cut to the first 512 tokens, and each query cut the same way. lamplighter cuts the text before
the request, so an endpoint with a context (per slot, for llama-server `-c / -np`) of at least 512 tokens never rejects a long memory, and the endpoint never applies its own truncation.

- **Exact:** when Qwen3-Embedding-0.6B's `tokenizer.json` is in the model cache, the text is cut after the 511th token, which is what the in-process model keeps before its end-of-text
  token. It gets there from `--fetch-tokenizer` (tokenizer only), `lampl doctor --fetch-model` or `lampl init --local --prepare` (whole model), run on the server host. Nothing is
  downloaded otherwise. The startup log line `endpoint embedder input truncation` says `truncation=tokens:512`.
- **Approximate:** without the tokenizer, the text is cut at 1,500 characters and the startup log warns (`truncation=chars:1500`). Measured with the Qwen3 tokenizer on 38 long texts
  from this workspace (Markdown docs, handoffs, Rust source): 1,500 characters is 405 tokens at the median, 525 at p95 and 661 at most, so a long memory is represented by somewhat
  less than the in-process 512 tokens, and an endpoint context of 1,024 tokens is safe for that text. Dense code or non-Latin text can run past 512 tokens; use the tokenizer.
- The tokenizer is Qwen3's whatever `--embed-model` names. For another 1024-dimension model the cut is close to 512 of its tokens, not exact.

Constraints, from the code:

- The endpoint must return **1024-dimension** vectors; anything else is a dimension-mismatch error. Use a 1024-dimension model, ideally the same Qwen3-Embedding-0.6B.
- The model digest recorded for endpoint vectors is the constant `endpoint`. Switching to a different endpoint model does not mark old vectors stale, so re-embed deliberately (export with
  `--no-vectors`, import into a fresh store) if you change models.
- The relevance floor (cosine 0.52, `capped:3` coverage 0.52) was calibrated on Qwen3-Embedding-0.6B vectors. The earlier LAMP-19 sweep on Ollama Q8_0 vectors of the same model agreed with the
  in-process vectors, but the current default was tuned on the in-process vectors only (in-sample); other models need their own calibration ([ranking.md](ranking.md)). Up to 0.3.0 the endpoint embedder sent
  the whole text, so a memory over 512 tokens that an endpoint stored then has a whole-text vector, not what the floor was calibrated on. Endpoint vectors all carry the digest
  `endpoint`, so the worker does not redo them by itself; clearing the vector (`UPDATE memories SET embedding = NULL WHERE embedding_digest = 'endpoint' AND length(title) + length(body)
  > 1000;`) makes the worker embed those rows again with truncation. The Ollama run of `bench-embed`
  for cosine agreement was not performed (UNVERIFIED).

## Degraded mode

If the embedder is unavailable, the server starts anyway, `/health` reports `degraded` with the reason, and recall answers full-text only with `mode: full-text` in the response and a `warning` in the
CLI output. `--min-coverage` (default 0.95) is the share of memories that must have vectors before `/health` stops reporting degraded. The client can also abandon a slow query embedding with
`--embed-timeout-ms` or `LAMPL_EMBED_TIMEOUT_MS`.

## Changing the model

Vectors are tagged with a model id and digest. A lamplighter import refuses an export whose model id or digest differs from the server's unless you pass `--reembed`, which drops the vectors and lets
the server embed again. The background worker re-embeds rows whose digest differs from the server's.
