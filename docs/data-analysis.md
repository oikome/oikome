# Data Analysis and Dataset Interoperability Plan

> **Status**: agreed direction 2026-11-14. PR 21 establishes collection and source targets. PR 31 reads existing Lance datasets and Iceberg tables in a collection's folders; media embeddings are not implemented.

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

Existing Lance datasets and Iceberg tables are not connected through a flow of their own (decided 2026-11-20, PR 31). A dataset is a folder, and it goes where files go: into a collection's folders, on this computer, a NAS, a drive or RustFS. Indexing recognises it and shows it as one item; it is a read-only table of the collection, read in place through DuckDB from its own metadata, schema and latest version, without first converting it, and followed as it changes like a file. Schema inference is unnecessary: the dataset supplies authoritative metadata, and only the roles of its columns are proposed. Because nothing is made from it, a dataset is an analysis table as soon as it is found; the tables of files are still made by the owner (decided 2026-11-21). A dataset that refers to files outside its own folder is not read. Only formats that carry their own metadata in their folder are datasets: Iceberg tables, whichever catalog wrote them (Hadoop's `version-hint.text` and `v<n>.metadata.json`, or a Hive or REST catalog's `<n>-<uuid>.metadata.json`, the highest taken), and Lance datasets (`_versions/*.manifest`). A plain Hive table, data files whose schema and partitions live in a metastore, is not supported (decided 2026-11-21). REST catalogs come later.

### Migration

Migration is not connection. It copies a dataset and its metadata into Oikome, identifies the target storage, states which versions can be retained and makes Oikome the single write master afterwards. Any continuing relationship with the original must be explicit. Archiving original files is a separate action and does not migrate an analysis dataset.

PR 31 first built this as Copy to Oikome on a dataset's table and removed it (decided 2026-11-21). A dataset is a folder of its collection like any other: there is no copy action of its own, and an analysis table, being made from files, is not something to copy (decided 2026-11-21; Copy to Oikome, built on 2026-11-20, was removed). Copying is done on files and folders, with Add Files. Migration as described above stays planned.

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

- **What becomes a table.** Every CSV, TSV, Parquet and JSON Lines file of a folder collection, at any depth (captions files excepted), and each visible, non-empty sheet of its spreadsheets (xlsx, xlsm, xls, xlsb, ods; PR 29), named `<file>_<sheet>`, or after the file when it has one such sheet. A top-level file is a table of its own, named after the file; the files of one format in one subfolder are read as one table, named after the folder, with `UNION ALL BY NAME`. Names are made unique; top-level files keep their own.
- **Where it is kept.** An Iceberg table in the node's catalog under the namespace named after the collection, whatever the collection's own location; binding a folder collection attaches these tables beside its file tables and lets one stand in for the file table of the same name. Each row carries `_source_file`.
- **Chosen by the owner** (decided 2026-11-21, PR 31). Create Analysis Tables… lists what the files can make, none chosen; the owner chooses items (a file, a subfolder's files read as one, a sheet, a workbook's sheets together) and only those are made. Remove Analysis Table… on a file's table drops it with its role corrections, after asking; its files are untouched and read in place again, and it is not made again until chosen. A dataset's table is always analysed and its table cannot be removed. Converting a CSV source to a file makes only that file's table.
- **Following the files.** The size and modification time of each file are kept with the table (`.oikome/analysis/<collection>.json`, with the owner's role corrections). When the folder is read again and they differ, the table is made again whole; when its files are gone, it is dropped. Removing the collection drops its tables and the namespace.
- **Types, not roles.** A column is what DuckDB read it as (a CSV's types from a sample of its rows): a date or a timestamp is a column of time; any column can be a dimension. The Analysis view shows each column's type, read-only, and charts are described in the explorer (PR 33). Roles (time, amount, category, title, key, other) are still proposed and kept for the table collections' Analyze…, and play no part in a folder collection's charts (2026-11-26).
- **Not yet.** Photo metadata and note front matter as tables; tables joining files across folders; aggregations other than sum and count.

## Delivery order

1. **PR 21:** collection and data-source targets, source identity migration, built-in CSV/file flows, semantic declarations and source/destination validation.
2. **PR 22:** safe local UTF-8 text editing, including code, configuration and text tables, with original-versus-derived behavior clear.
3. **PR 26:** Save To and public sharing form one content-delivery item, with saved portfolios and deliberately published public snapshots.
4. **PR 27:** precise source targets first, then internal analysis profiles as core computation: collection-contained files and tables, plugin capabilities, direct-query versus derived-analysis actions, human-friendly inference review and source-structure change handling.
5. **PR 28:** data sources as plugins and analysis tables of a folder collection's files (above). K1 and K2 of knowledge landed with PR 27.
6. **PR 29:** folders from several places in one collection, CSV sources converted to files, Excel sheets as analysis tables.
7. **PR 30–31:** backup and restore, then read-only Lance and Iceberg datasets in collections' folders. Parquet and CSV on RustFS came with PR 29.
8. **PR 32:** the plugin sandbox and RAW previews. Media recognition (knowledge K4) is not planned (2026-11-19).
9. **After:** freer analysis through a third-party library in a plugin, and tables as knowledge (K3). See `roadmap.md` and `research/next-phases.md`.

Advanced dataset and migration capabilities must be marked planned until their real-node acceptance scenarios pass. They must not appear as supported options in the ordinary interface. Reading datasets and Copy to Oikome appear once PR 31's acceptance scenario passes.
