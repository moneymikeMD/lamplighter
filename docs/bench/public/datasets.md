# Datasets

The harness downloads datasets when it runs. No dataset text is committed to this repository or redistributed by it.

## LongMemEval (used)

| | |
| --- | --- |
| Source | `xiaowu0162/longmemeval-cleaned` on Hugging Face, file `longmemeval_s_cleaned.json` |
| Pinned revision | `98d7416c24c778c2fee6e6f3006e7a073259d48f` (last modified 2025-09-19) |
| SHA-256 | `d6f21ea9d60a0d56f34a05b609c79c88a451d2ae03597821ea3d5a9678c3a442`; the harness refuses a file with any other hash |
| Dataset licence | MIT, read on 2026-11-06 from the dataset card's front matter (`license: mit`) and the `license` field of the Hugging Face API |
| Code licence | MIT (`xiaowu0162/LongMemEval`, GitHub licence API, 2026-11-06) |
| Paper | Wu et al., "LongMemEval: Benchmarking Chat Assistants on Long-Term Interactive Memory" (ICLR 2025) |

The MIT licence permits this use (download, run, publish scores). The cached file lives under the OS cache directory (`lamplighter/bench`), outside the repository.

500 questions, about 48 sessions of chat history each. 30 are abstention questions (`_abs`) with nothing to retrieve; the harness skips them, leaving 470 scorable questions in six types.

## LoCoMo (not used)

LoCoMo is CC BY-NC 4.0 per the ticket that scoped this benchmark: non-commercial, download only, never redistribute. The GitHub licence API reports `NOASSERTION` for `snap-research/locomo`, so that terms claim is UNVERIFIED by this repository. No LoCoMo arm ships. If one is added later it must download at run time and be labelled non-commercial research use.
