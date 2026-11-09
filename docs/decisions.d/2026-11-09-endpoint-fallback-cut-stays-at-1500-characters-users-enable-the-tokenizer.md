---
date: 2026-11-09
slug: endpoint-fallback-cut-stays-at-1500-characters-users-enable-the-tokenizer
title: "Endpoint fallback cut stays at 1500 characters; users enable the tokenizer"
---

# Endpoint fallback cut stays at 1500 characters; users enable the tokenizer

## Decision

The endpoint embedder's fallback cut without a cached Qwen3 tokenizer stays at 1,500 characters (FALLBACK_MAX_CHARS). lamplighter does not download the tokenizer on its own; the docs tell every
endpoint-mode user to pass --fetch-tokenizer, and doctor and the startup log point to it when the fallback is active.

## Why

Owner decision 2026-11-09. 1,500 characters fits a 1,024-token endpoint slot on the measured corpus (p95 525, max 661 tokens), and an automatic download would break the no-implicit-downloads promise
in docs/modes/README.md. The exact 512-token path is one flag away.

## Cost

Endpoint hosts that skip --fetch-tokenizer cut medium texts of roughly 375 to 511 tokens shorter than the in-process model does, so those vectors are not the input the 0.52/0.52 capped:3 floor was
calibrated on; that path is unmeasured against the floor.
