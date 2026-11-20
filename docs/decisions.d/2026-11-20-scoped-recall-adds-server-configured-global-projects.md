---
date: 2026-11-20
slug: scoped-recall-adds-server-configured-global-projects
title: "Scoped recall adds server-configured global projects"
---

# Scoped recall adds server-configured global projects

## Decision

recall --project P searches P plus the server's global projects (--global-project / LAMPL_GLOBAL_PROJECTS, default general); --strict searches P alone. export, audit and list keep matching P exactly. The plugin hook recalls with the session's repo name. Memories keep a single project column: no schema change, no multi-project rows. The /v1 API version is not bumped.

## Why

The goal is to exclude memories of unrelated projects while still surfacing the ones that hold everywhere. Making the caller list general itself (a plain multi-project filter) means any caller that forgets it silently loses those memories; a global flag or a projects array needs a migration and a new export format for a need one reserved project meets. The server owning the list keeps every client in agreement.

## Cost

A scoped recall's candidates now include general, a semantics change for existing project callers made without an API version bump. The relevance floor's IDF corpus becomes the scoped set, so the floor measured unscoped must be re-checked. A memory shared by two specific repos still has to live in general or be duplicated.
