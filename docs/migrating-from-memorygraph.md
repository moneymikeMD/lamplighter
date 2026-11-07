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

`lampl import --from-postgres` reads memory-pg's `nodes(id, properties jsonb)` and `relationships(from_id, to_id, rel_type, properties jsonb)` tables and then uses the same conversion. **UNVERIFIED:** that table layout is from memory of the memorygraph Postgres backend and was not checked against the live store (read access was not available when this was written). If the schema is not recognised the import fails and lists the tables present; use the JSON export in that case, and confirm the layout before relying on this path in the cutover.
