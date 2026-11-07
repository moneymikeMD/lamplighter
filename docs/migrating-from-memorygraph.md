# Migrating from memorygraph

```bash
lampl import ~/.memorygraph/export-final-2026-09-28.json --project homelab --dry-run   # convert and check
lampl import ~/.memorygraph/export-final-2026-09-28.json --project homelab
```

Memorygraph memories have no project, so `--project` (default `memorygraph`) is applied to all of them. Imports always re-embed through the server's background worker. Running the import twice adds nothing.

## Type mapping

The table is data: `crates/lampl/src/import/mapping.toml`. Each imported memory also keeps its original type as a `memorygraph:<type>` tag. A memorygraph type missing from the table fails the whole import.

| memorygraph | lamplighter |
| --- | --- |
| problem, error | problem |
| solution, fix | fix |
| code_pattern, workflow, command | procedure |
| general, conversation, technology, project, task, file_context | fact |

`decision` and `gotcha` have no memorygraph source type; use tags such as `decision` to find them and retype by hand if wanted.

## Links

`SOLVES`, `CAUSES` and `CONTRADICTS` carry over with their strength and creation time. `CONTRADICTS` is never converted to `SUPERSEDES`; supersession starts with new memories. `RELATED_TO`, `SIMILAR_TO` and `BLOCKS` have no lean equivalent: they are dropped and the counts are printed on stderr. Links whose endpoint is missing from the export, and self links, are dropped the same way.

## Ids and fields

UUID ids are kept. A non-UUID id becomes a stable UUIDv5 of the original, so reruns land on the same rows. `content` becomes `body` (a non-empty `summary` is appended), `importance` is clamped to 0..1, and `confidence`, `version` and relationship metadata other than strength are not carried.

## Direct database read

`lampl import --from-postgres` reads memory-pg's flat `memories` table (id, type, title, content, summary, tags, importance, timestamps) and `links` table (from_id, to_id, rel_type, strength, created_at), taken from the memorygraph backend's `CREATE TABLE`s, then uses the same conversion. Links with `valid_until` or `invalidated_by` set are soft-invalidated in memorygraph and are skipped. If the tables do not match, the import fails and lists the tables present. Embeddings are never read; everything re-embeds.

## Known limits

The JSON export and the database read are not snapshots of a quiet store; stop writers first. `RELATED_TO`, `SIMILAR_TO` and `BLOCKS` links are dropped (32 of 346 in the 2026-09-28 export) and counted on stderr.
