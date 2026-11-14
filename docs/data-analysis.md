# Data Analysis and Dataset Interoperability Plan

> **Status**: agreed direction 2026-11-14. PR 21 establishes collection and source targets. It does not implement media embeddings, external Iceberg/Lance connections or migrations.

## Principles

- Original content is the source of truth. Analysis produces derived, rebuildable data and never silently rewrites or removes the original.
- Adding content, querying it directly, generating analysis data and archiving a copy are different actions.
- Ordinary people work with content and goals, not database engines or file formats. Iceberg, Lance, Parquet, SQLite, vectors and storage layouts stay out of the ordinary flow; technical details remain available under Info.
- One analysis dataset has one primary physical layout selected by the system. Do not duplicate the same table across Iceberg and Lance to provide different capabilities.
- Every derived row, chunk, vector or preview retains its source path, content version, parser/model version and enough location information to open the original and rebuild after a change.
- External datasets are read-only by default. Oikome and an external system must never become silent write masters of the same dataset.

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
- source identity, path and content version;
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

## Delivery order

1. **PR 21:** collection and data-source targets, source identity migration, built-in CSV/file flows, semantic declarations and source/destination validation.
2. **PR 22:** safe Markdown and plain-text editing with original-versus-derived behavior clear.
3. **Analysis profiles:** the internal dataset profile, direct-query versus derived-analysis actions, human-friendly inference review and source-structure change handling.
4. **Dataset interoperability:** read-only Iceberg/Lance/S3 connections, followed by deliberately evaluated migrations.
5. **Media analysis:** image metadata, optional model-based semantic indexes and optional archive workflows. Model and RAW capabilities belong in plugins with independent grants and no hidden downloads.

Advanced dataset and migration capabilities must be marked planned until their real-node acceptance scenarios pass. They must not appear as supported options in the ordinary interface.
