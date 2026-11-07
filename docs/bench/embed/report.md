# lampl-embed CPU latency: Qwen3-Embedding-0.6B on candle

Command: `lampl bench-embed --texts docs/bench/embed/texts-47.jsonl --prepare` (release build, F32 weights, no query prefix, one text per call, 1 untimed warm-up, 3 timed passes).

Text set: `texts-47.jsonl` is a **public stand-in** for the private LAB-391 set: 35 short queries and 12 prompt-length texts (335-385 characters), written from public material. The lab can pass its own set with `--texts`.

Model: `Qwen/Qwen3-Embedding-0.6B` at Hugging Face revision `97b0c614be4d77ee51c0cef4e5f07c00f9eb65b3`, 1024 dimensions.

| Platform | Machine | Threads | Load (ms) | short p50 / p95 (ms) | prompt p50 / p95 (ms) |
| --- | --- | --- | --- | --- | --- |
| Mac arm64 (aarch64) | local Apple M2 | 8 | 3667 | 116.9 / 168.4 | 371.6 / 462.7 |
| Linux x86_64 | GitHub Actions `ubuntu-latest` runner, AMD EPYC 9V74 80-Core Processor | 4 | 1337 | 387.0 / 410.1 | 740.8 / 809.9 |

Short: n=105 samples per platform; prompt: n=36. Linux numbers come from run 42329160714 of the `bench-embed` workflow, not from a lab host. Hetricus and TrueNAS measurements belong to LAB-694.

Both platforms are inside the ~1.5 s hook embedding budget at p95 for these texts.

## Cosine against Ollama Q8_0

Not run. No Ollama was reachable from either measuring environment (none installed on the Mac, none on the CI runner), and no lab host was used. Informational only: import re-embeds everything, so this is not a gate. Run `lampl bench-embed --ollama-url <url>` where an Ollama with `qwen3-embedding:0.6b` is available.
