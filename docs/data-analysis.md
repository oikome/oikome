# Data Analysis and Dataset Interoperability Plan

> **Status**: agreed direction 2026-11-14. PR 21 establishes collection and source targets. It does not implement media embeddings, external Iceberg/Lance connections or migrations.

## Principles

- Original content is the source of truth. Analysis produces derived, rebuildable data and never silently rewrites or removes the original.
- Adding content, querying it directly, generating analysis data and archiving a copy are different actions.
- Ordinary people work with content and goals, not database engines. A format may appear where it is a real choice, preselected and explained in one sentence (Knowledge in Lance for search and AI assistants, Analysis tables in Iceberg for totals and charts; see `knowledge-schema.md`). Engines, storage layouts and other technical details stay under Info.
- One analysis dataset has one primary physical layout selected by the system. Do not duplicate the same table across Iceberg and Lance to provide different capabilities.
- Every derived row, chunk, vector or preview retains its source path, content version, parser/model version and enough location information to open the original and rebuild after a change.
- External datasets are read-only by default. Oikome and an external system must never become silent write masters of the same dataset.

## Collections and source targets

A collection is a container, not a source's implicit destination. It can hold original files, ordinary tables and views, connector-synchronized tables, and derived analysis tables or indexes at the same time. A source belongs to one of those data objects, not implicitly to the whole collection.

Every source registration identifies a target object precisely:

- collection and relative file path or table name;
- plugin id and capability id, with the connector's declared input and output shape;
- origin file, application or external dataset, independent of the storage the collection lives on;
- direct-query versus continuing-sync behavior, primary key and conflict policy;
- content version and links to derived artifacts.

A file connector may target `reading-list/books.csv` or `reading-list/books`; it must not create a `drive-reading` collection merely to demonstrate Drive access. An application connector may produce one or more named tables in an existing collection. New collections are created only by an explicit owner choice, never as a side effect of connecting a plugin.

The ordinary interface presents this as "reading-list / books comes from Drive". Storage and provenance are separate facts: Drive may be where the collection is kept, the CSV plugin may be how the file is read, and an analysis action may be why a derived table exists.

## Two kinds of source, plus migration

### Ordinary content

Files, folders and application data enter a collection chosen by the owner. The system identifies useful semantics and views without asking for SQL types. A question is asked only when ambiguity affects correctness, using human terms such as which column is the date or how invalid amounts should be treated.

### Existing technical datasets

A later advanced Add Data Source flow connects existing Iceberg or Lance datasets, including datasets reached over S3. It reads their metadata, schema, partitions and versions in place, without first converting them. The default is a read-only external dataset; schema inference is unnecessary when the dataset already supplies authoritative metadata.

### Migration

Migration is not connection. It copies a dataset and its metadata into Oikome, identifies the target storage, states which versions can be retained and makes Oikome the single write master afterwards. Any continuing relationship with the original must be explicit. Archiving original files is a separate action and does not migrate an analysis dataset.

## Internal dataset profile

The system, not the ordinary owner, maintains a versioned dataset profile:

- source kind: `content`, `external_dataset`, `migration` or `derived_index`;
- target collection and relative file or table, plus the plugin and capability that produced it;
- source identity, path and content version, distinct from the target's storage identity;
- parser and inference-rule versions;
- physical fields and types, with raw content retained when inference is uncertain;
- field roles such as title, time, place, amount and primary key;
- content semantics and recommended views;
- inference confidence and records that could not be understood;
- the selected internal layout and links to derived indexes.

Safe additive changes can be accepted automatically. Type widening is allowed when compatible. Changed meanings, repeated keys and incompatible types require a human decision rather than silent migration.

## Actions and internal layouts

- **Add originals:** reference files where they are or deliberately copy them into a collection.
- **Query directly:** let DuckDB read supported CSV, Parquet, JSON and compatible directory datasets without first copying them. In-place file-preview limits are not a claim that direct dataset analysis has the same limit.
- **Generate analysis data:** create an internal analytical table only when repeated analysis, sharing or derived indexes justify it. Existing Iceberg analytical paths remain the default for ordinary structured analysis; existing SQLite paths remain responsible for connector writes, keys and cursor transactions.
- **Generate a semantic index:** only after the owner asks for text or semantic search. Lance owns suitable vector/full-text/large-object capabilities. Do not create a vector index by default and do not present an embedding as human-readable content.
- **Archive originals:** copy original files with hashes and a manifest to object storage when the owner explicitly asks. This is unrelated to analysis or vector generation.

## Content rules

- CSV can be queried directly or converted deliberately after target, key, schema and conflict checks. Continuous connector synchronization still uses SQLite; long-term derived analysis uses the internal analytical layout.
- JSON and JSON Lines become tables only when records have a compatible shape. Nested or varying records retain their raw JSON and may receive only the appropriate indexes.
- A directory of compatible tabular files may be treated as one logical dataset with per-file provenance and partition information. A mixed directory remains a file collection; eligible files are analyzed individually and failures are reported per file.
- Images remain original files. Metadata can be analyzed separately. Thumbnails are caches. Embeddings require an explicit semantic-index action and a model/plugin choice; they are never backups or replacements for original images.

## Analysis tables of a folder collection (PR 28)

Delivered as the first concrete form of "Generate analysis data" for files:

- **What becomes a table.** Every CSV, TSV, Parquet and JSON Lines file of a folder collection, at any depth (captions files excepted). A top-level file is a table of its own, named after the file; the files of one format in one subfolder are read as one table, named after the folder, with `UNION ALL BY NAME`. Names are made unique; top-level files keep their own.
- **Where it is kept.** An Iceberg table in the node's catalog under the namespace named after the collection, whatever the collection's own location; binding a folder collection attaches these tables beside its file tables and lets one stand in for the file table of the same name. Each row carries `_source_file`.
- **Following the files.** The size and modification time of each file are kept with the table (`.oikome/analysis/<collection>.json`, with the owner's role corrections). When the folder is read again and they differ, the table is made again whole; when its files are gone, it is dropped. Removing the collection drops its tables and the namespace.
- **Roles.** Proposed from names and types (time, amount, category, title, key, other), corrected per column by the owner; the summary sums the Amount column by month and by category, or counts rows.
- **Not yet.** Excel sheets, photo metadata and note front matter as tables; tables joining files across folders; aggregations other than sum and count.

## Delivery order

1. **PR 21:** collection and data-source targets, source identity migration, built-in CSV/file flows, semantic declarations and source/destination validation.
2. **PR 22:** safe local UTF-8 text editing, including code, configuration and text tables, with original-versus-derived behavior clear.
3. **PR 26:** Save To and public sharing form one content-delivery item, with saved portfolios and deliberately published public snapshots.
4. **PR 27:** precise source targets first, then internal analysis profiles as core computation: collection-contained files and tables, plugin capabilities, direct-query versus derived-analysis actions, human-friendly inference review and source-structure change handling.
5. **PR 28:** knowledge for AI in steps K1–K4 (chunks and full text, a local embedding model and skills, tables as knowledge, then media), plus real RAW plugin support, as planned in `knowledge-schema.md`. Models are chosen explicitly and never downloaded silently.
6. **PR 29:** read-only Iceberg/Lance/S3 dataset interoperability, followed by deliberately evaluated migrations and backup/recovery paths with integrity and restore checks.

Advanced dataset and migration capabilities must be marked planned until their real-node acceptance scenarios pass. They must not appear as supported options in the ordinary interface.
