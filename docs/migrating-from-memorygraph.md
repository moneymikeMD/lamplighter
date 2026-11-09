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

Links carry over with their strength and creation time and keep their direction. The table is the `[links]` section of the same `mapping.toml`; a row maps to a lean link type (not necessarily its own name) or to `dropped`.

| memorygraph | lamplighter |
| --- | --- |
| `SOLVES`, `ADDRESSES` | `SOLVES` (solution to problem) |
| `CAUSES` | `CAUSES` |
| `CONTRADICTS` | `CONTRADICTS` |
| `REPLACES` | `SUPERSEDES` |
| `RELATED_TO`, `SIMILAR_TO`, `BLOCKS`, `BUILDS_ON`, `DEPENDS_ON`, `FOLLOWS` | dropped |

memorygraph writes `REPLACES` as `link NEW OLD REPLACES`, and lamplighter reads `SUPERSEDES` as `from` replaces `to`, so endpoints are not swapped. The old memory then stops appearing in recall. `CONTRADICTS` is never converted to `SUPERSEDES`.

Dropped links are counted on stderr (`links dropped: BUILDS_ON: 4`), in `--dry-run` as well. A link type that is not in the table at all is dropped and counted under its own name (`links dropped: unmapped type NEW_TYPE: 2`) rather than failing the import, so a type added by a later memorygraph release shows up in the report; add a row to `mapping.toml` to carry it over. Links whose endpoint is missing from the export, and self links, are dropped the same way.

## Ids and fields

UUID ids are kept. A non-UUID id becomes a stable UUIDv5 of the original, so reruns land on the same rows. `content` becomes `body` (a non-empty `summary` is appended), `importance` is clamped to 0..1, and `confidence`, `version` and relationship metadata other than strength are not carried.

## Direct database read

`lampl import --from-postgres` reads memory-pg's flat `memories` table (id, type, title, content, summary, tags, importance, timestamps) and `links` table (from_id, to_id, rel_type, strength, created_at), taken from the memorygraph backend's `CREATE TABLE`s, then uses the same conversion. Links with `valid_until` or `invalidated_by` set are soft-invalidated in memorygraph and are skipped. If the tables do not match, the import fails and lists the tables present. Embeddings are never read; everything re-embeds.

## Known limits

The JSON export and the database read are not snapshots of a quiet store; stop writers first. Links with no lean equivalent are dropped (32 of 346 in the 2026-09-28 export) and counted on stderr.
