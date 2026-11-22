---
date: 2026-11-21
slug: recall-min-score-filters-on-cosine
title: "recall min-score filters on cosine"
---

# recall min-score filters on cosine

## Decision

lampl recall --min-score and the MCP tool's min_score compare against a hit's cosine, not its score field. A hit with no cosine (a full-text-only recall) is kept. The brief line prints cosine, or - when there is none.

## Why

A hit's score is the reciprocal-rank fusion value, about 0.016 to 0.033, so a threshold on it cannot express LAMP-37's intent of dropping cosine-0.55 matches; cosine is the number the relevance floor and the ticket reason in. Dropping cosine-less hits would empty every recall made while the embedder is down.

## Cost

The flag's name says score while it reads cosine; docs/cli.md and docs/mcp.md say so. A full-text-only recall is not narrowed by --min-score.
