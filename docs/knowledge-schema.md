# Knowledge and Analysis Schema

> **Status**: planned, 2026-11-17. Nothing here is implemented yet; the stages are in the roadmap (PR 28). It builds on PR 27 (precise source targets, dataset profiles and derived analyses). The research behind it is in [`research/knowledge-bases.md`](research/knowledge-bases.md). Decisions taken and questions still open are in §11.

## 1. Goal

Oikome turns the owner's own material into two derived forms:

- **Knowledge an AI can read.** Text chunks with their location in the original, full-text and vector indexes, media evidence (OCR text, transcripts, captions, image vectors) and stored skills. AI assistants read it over MCP as context, under the owner's grants.
- **Tables that can be analyzed.** Typed tables with field meanings (time, amount, category, key), aggregated and charted like a small BI tool, and queried by assistants with SQL.

Originals stay where they are and are never rewritten. Every derived row points back to its original and to the version, converter and model that made it, so it can be rebuilt after a change.

For ordinary people the apps behave like a knowledge-base application with light BI: they see their files, a **Prepare** action that converts them, and the results (searchable knowledge, tables and charts). Formats are not hidden. Where a choice matters, the owner picks a format with a one-sentence explanation, the way AnythingLLM presents "LanceDB (Built-in)" with "no configuration needed": a person can choose it without knowing what Lance is. The default choice is always filled in.

Knowledge works before any model is chosen. Preparing a collection first gives full-text search over its chunks; vectors are added to the same chunks when a model is chosen, without converting again (Cherry Studio does this).

This changes one principle of [`data-analysis.md`](data-analysis.md): formats may appear in the ordinary flow when they are explained and preselected. Engine names (DuckDB, SQLite) still do not.

## 2. Words

| The owner sees | The system keeps |
|----------------|------------------|
| Collection | Space |
| Original: a file, an app's record, a saved file, a skill | Asset (§4.1) |
| Prepare for AI / Prepare for analysis | Conversion (§4.2) |
| Knowledge: what an AI can read and search | Lance knowledge dataset of the collection (§5) |
| Analysis table | Iceberg table with a dataset profile (§6) |
| Model | A row of the model registry (§4.3) |
| Retrieval test | A search against the knowledge dataset with its scores and sources |
| Skill | A folder with `SKILL.md` registered in the collection (§7) |

## 3. Where things live

Each collection has its own derived data, because grants, sharing, subscriptions and removal already work per Space:

```
.oikome/knowledge/<space>/
  catalog.sqlite        control plane: assets, conversions, jobs, skills, table profiles
  lance/chunks.lance    chunks (§5.1), one Lance dataset per table
  lance/media.lance     media (§5.2, K4)
<Space's analytical storage>/<table>/   analysis tables (Iceberg + Parquet), as today
```

- The control plane is SQLite: small rows, frequent updates, transactions, the same as the in-place index (`.oikome/index/<location>/index.sqlite`, architecture §7.7). The in-place index's `files` rows become the collection's assets; its FTS5 trigram index stays for quick name and title search.
- Knowledge is Lance: versioned, append and update, vector and full-text indexes stored in the dataset, so they travel with a subscription.
- Analysis tables are Iceberg + Parquet: scanned and aggregated by DuckDB, as every analytical Space is today. PR 27's derived analyses are this case.
- A collection's derived data is deleted with the collection and rebuilt from its assets when lost.

### 3.1 Who writes and who reads

- **Lance is written, read and searched through DuckDB's `lance` extension.** The Rust `lance` crate was the first choice, but it needs `lance-index`, whose `dirs` dependency brings in `option-ext` under MPL-2.0, outside the permissive licences this project allows (`cargo deny`). The extension is installed by the owner from the apps (`install_lance`) or the CLI, never downloaded silently. DuckDB also joins Lance results with tables, as it does for Lance Spaces today (architecture §7.6).
- **Two statements per asset, one current version.** The extension commits each statement on its own, and an UPDATE or MERGE after another write to the same table in one transaction is not implemented. A conversion therefore inserts its asset's new rows in one statement, then deletes the rows of other conversions of that asset in the next. The control plane marks the conversion current only after the insert, and every read keeps only rows of current conversions, so a search between the two statements never returns both versions. After a round of conversions the full-text index is rebuilt once. `chunks` and `media` are separate datasets, so a conversion that touches both writes `media` first and `chunks` last, and a reader never sees a chunk pointing at a missing media row. - **Vectors (K2).** A model's vectors are a `FLOAT[n]` column of `chunks`, `vec_<model>`, added with `ALTER TABLE … ADD COLUMN` the first time a collection is searched by meaning with it. A conversion writes its rows' vectors in the same `INSERT` as their text, cast from text, because DuckDB loads only flat Arrow columns. Choosing, changing or dropping the model changes the conversion version, so every original is converted again while the chunks it has keep serving search by words; rows without a vector are left out of search by meaning. Chunk ids do not depend on the model, so chunks the owner turned off stay off.
- **Full text in Chinese.** The `INVERTED` index is built with the `ngram` base tokenizer, which needs no dictionary download and splits any script, as the SQLite FTS5 trigram index does. `jieba` would need model files in `LANCE_LANGUAGE_MODEL_HOME`; it can be added later as an option if word-level ranking proves worth a download.
- **Iceberg stays the format of analysis tables**, written through Oikome's existing path (AppendBuffer → Parquet → Iceberg commit; `replace_data_files` for a rebuild as one overwrite snapshot) and read by DuckDB. Field meanings are also written into each Iceberg field's `doc`. Chunks never go into Iceberg and tables never into Lance: one dataset, one layout (data-analysis.md).

## 4. Control plane (SQLite)

K1 implements `assets`, `conversions` and `chunk_overrides` (a chunk the owner turned off, §10). K2 adds `embedding_cache` and a `model` fact (the model the collection is searched by meaning with). Models are not a table: K2 has one, described in the node (`knowledge/model.rs`) with its pinned revision and file digests, and kept under `.oikome/models/<id>/<revision>/` for every collection to use.

### 4.1 `assets`

One row per original the collection holds, whatever brought it in.

| Column | Type | Meaning |
|--------|------|---------|
| `asset_id` | TEXT PK | Stable identity: BLAKE3 of collection + origin kind + origin key. A file keeps its id across edits; a rename is a new asset (the old one becomes `missing`). |
| `origin_kind` | TEXT | `file` (in place or imported), `saved_file` (Save To), `app_record` (a connector table row group), `skill` |
| `origin_key` | TEXT | Relative path, or `<table>` for connector rows, or the skill folder |
| `name`, `media_type`, `size`, `modified_ms` | | As the in-place index has them |
| `blake3` | TEXT | Content version. A change marks its conversions `stale`. |
| `sha256` | TEXT NULL | Only where a protocol needs it (MCP skills, §7) |
| `state` | TEXT | `present`, `missing`, `unreadable` |
| `first_seen_ms`, `last_seen_ms` | INTEGER | |

### 4.2 `conversions`

One row per asset and target. The previous version keeps serving until a new one is `ready` (Dify and FastGPT both do this; it keeps search working during a rebuild).

| Column | Meaning |
|--------|---------|
| `conversion_id`, `asset_id`, `asset_blake3` | Which original, at which version |
| `target` | `knowledge`, `media`, `table` |
| `converter`, `converter_version`, `params_hash` | What did it and with which settings (chunk mode, page range, column usage) |
| `model_ids` | JSON list of models used (§4.3) |
| `status` | `queued`, `reading`, `splitting`, `embedding`, `ready`, `failed`, `stale` |
| `stage_ms` | JSON of stage → time, for the progress line |
| `channels` | Requested and produced channels with their status (`text`, `ocr`, `asr`, `caption`, `image_vector`): an empty transcript is a result, a failed recognizer is an error |
| `produced` | Counts: chunks, media rows, table rows |
| `error` | Why it failed, in words for the owner |
| `lance_version` | The knowledge dataset version this conversion's rows were committed in |

### 4.3 `models`

| Column | Meaning |
|--------|---------|
| `model_id` | `qwen3-embedding-0.6b@<revision>` |
| `purpose` | `text_embedding`, `image_embedding`, `ocr`, `asr`, `caption`, `rerank` |
| `runtime` | `local` (in the node: ONNX or candle) or `provider` (an API the owner configured with their own key) |
| `dims`, `metric`, `normalized` | For vectors |
| `query_instruction` | Prefix for queries where the model needs one (Qwen3: `Instruct:`; e5: `query:`) |
| `license` | Shown before the owner picks it |
| `column` | The Lance column that holds its vectors, `vec_<slug>` |
| `state` | `active`, `building`, `retired` |

In K2 the node knows one model and runs it with candle in 32-bit floats: on a Mac's GPU through Metal when a first text runs there, on the CPU otherwise (about 2.4 GB of memory while loaded, let go after five idle minutes). On the CPU a 900-character chunk takes seconds, too slow for a collection of hundreds of documents, which is why the GPU is used where there is one. Vectors are kept 16 chunks at a time, so a long original interrupted half way resumes. It is downloaded only when the owner asks, from Hugging Face at a pinned revision, resuming an interrupted download, and every file is checked against its published SHA-256 before use. A document is embedded as its heading path and text; a query after `Instruct: … Query:`; the vector is the hidden state at the closing `<|endoftext|>`, normalized.

### 4.4 `embedding_cache`

`(input_hash, model@revision) → vector`, in the collection's control plane. The input is what was embedded (headings and text), so an unchanged chunk is never embedded twice, even when its document is re-split, re-read or converted again for a new setting (Dify's `embeddings` table, LangChain's record manager).

### 4.5 `jobs`

The queue that runs conversions: `job_id, conversion_id, attempt, not_before_ms, started_ms, worker`. Failed jobs are retried a bounded number of times; the owner can retry all.

In K1 the queue is the `conversions` table itself: a `queued` conversion is a job, with its `attempts` (at most three). One worker runs per collection, in the background after Prepare and in the node's regular refresh, within a time budget. A separate table comes when scheduling needs more.

## 5. Knowledge (Lance)

### 5.1 `chunks`

One table for everything text-like, so one hybrid search covers documents, notes, OCR, transcripts, table rows and skills. Stable row ids are enabled when the dataset is made, so compaction does not move rows.

| Column | Type | Meaning |
|--------|------|---------|
| `chunk_id` | STRING | BLAKE3 of asset version + converter version + locator. Deterministic, so re-reading the same version yields the same ids. |
| `asset_id`, `asset_blake3`, `conversion_id` | STRING | Provenance |
| `channel` | STRING | `text`, `ocr`, `caption`, `transcript`, `table_row`, `summary`, `skill` |
| `parent_id` | STRING NULL | Parent chunk (a section above its paragraphs) |
| `seq` | INT32 | Order within the asset |
| `heading_path` | LIST<STRING> | `Part 1 > Chapter 3`; added to the embedded text, as RAGFlow and FastGPT do |
| `text` | STRING | What is read and embedded |
| `content_hash` | STRING | BLAKE3 of normalized text; keys the embedding cache |
| `lang`, `token_count` | | |
| `loc` | STRUCT | `page`, `char_start`, `char_end`, `bbox` (four floats, top-left origin, 0–1), `t_start_ms`, `t_end_ms`, `sheet`, `row_key`, `snapshot_id`. Enough to open the original at the place: a page and box, a time, a table row at the snapshot it was read from. |
| `tags` | LIST<STRING> | Note tags, folder names, owner tags |
| `enabled` | BOOL | Disabled chunks stay but are not returned |
| `vec_<model>` | FLOAT[N] NULL | One column per text model, added by schema evolution |

Indexes:

In K1 the columns are flat, because DuckDB loads only flat Arrow columns into the extension: `heading` is JSON text, and `page`, `char_start`, `char_end`, `line_start` and `line_end` (1-based, inclusive) are columns of their own instead of `loc`. `parent_id`, `lang`, `token_count`, `tags` and `enabled` are not written yet; a chunk turned off is a row in the control plane, so turning it off needs no Lance write.

- `INVERTED(text)` for full-text. Chinese needs a tokenizer that splits it: `ngram`, or `jieba` if the DuckDB 1.5.6 `lance` build has it (§11). The SQLite FTS5 trigram index stays as the fallback.
- `BTREE(asset_id)`, `BITMAP(channel)`, `LABEL_LIST(tags)` for filters.
- `IVF_HNSW_SQ(vec_<model>)` once a collection has enough chunks for an index to pay off; below that, exact search.

Typed columns rather than a metadata blob: LlamaIndex and LangChain hide provenance in a JSON field, which cannot be indexed or filtered well.

### 5.2 `media`

| Column | Meaning |
|--------|---------|
| `media_id`, `asset_id`, `asset_blake3` | An image, or a clip of a video or recording |
| `kind` | `image`, `video_clip`, `audio_clip` |
| `t_start_ms`, `t_end_ms` | A clip's span (a clip is identified by asset, clip id, start and end) |
| `width`, `height`, `duration_ms`, `taken_at`, `place` | From the file and EXIF |
| `gps` | STRUCT NULL. Location is personal data: kept for the owner, removed from public shares (architecture §7.9) |
| `exif` | STRING (JSON text; the `lance` extension cannot write JSON-typed columns) |
| `thumbnail` | Blob v2, at most 64 KiB so it stays inline |
| `img_<model>` | FLOAT[N] NULL, one per image model |

OCR text, captions and transcript segments are chunks (§5.1) with a `loc` that points into the media row's time or box, so one text search finds what was said in a video and what is written on a photo.

## 6. Analysis tables (Iceberg)

PR 27 gives each table a dataset profile: fields with an inferred role (`time`, `amount`, `category`, `title`, `key`, `other`), a schema fingerprint, and derived analyses that remember their query and source fingerprint. This draft keeps that and adds:

- **Roles.** Add `measure` (a number that is not money: a temperature, steps), `text` (long text that also becomes knowledge) and `place`. A whole-number column named or behaving as an id is `key`, never a measure.
- **Units.** `unit` and `currency` per field, from the name (`living_room_celsius`) or the owner.
- **Column usage for knowledge**, as RAGFlow's Indexing / Metadata / Both: a `text` field's rows become `table_row` chunks; `category`, `place` and `time` fields become chunk tags or filters.
- **Meaning travels with the table.** The profile is written into each Iceberg field's `doc` so another tool and a subscriber see it.
- **Spreadsheets stay tables.** Dify and FastGPT turn each row into a text chunk, which cannot be summed; RAGFlow then has an LLM write SQL over a search engine with a repair loop. Oikome keeps the rows in a table that DuckDB aggregates, and gives assistants a SQL tool over it.

Inputs that become tables: CSV, TSV, Parquet, JSON Lines, Excel sheets (through a content plugin), and connector tables (published from SQLite, architecture §7.3).

## 7. Skills

A skill is a folder with `SKILL.md` and optional `scripts/`, `references/`, `assets/` (the Agent Skills format: `name` and `description` in front matter, the body loaded when used, other files on demand).

- **Registry.** `skills(skill_uri, asset_id, name, description, front_matter, files)`, where `skill_uri` is `skill://<collection>/<name>/SKILL.md` and `files` lists each file's path, size and SHA-256. MCP's Skills extension (SEP-2640) asks for SHA-256 digests, so skills keep it beside BLAKE3.
- **Search.** The body becomes `skill` chunks, so a search can find the skill that fits a task.
- **Delivery.** Assistants list and read skills through MCP (§9). A skill's scripts are delivered as files and never run by Oikome.
- **Trust.** A skill is instructions for an AI. Only the owner adds one to a collection, it is shared with the same grants as any file, and the apps show its source before an assistant can use it.

In K2 skills come with Prepare: in a collection prepared for AI, every folder holding `SKILL.md` is a skill, read from the in-place index when asked (no registry table yet). Its front matter's `name` (1–64 of `a-z`, `0-9` and single hyphens, its folder's name) and `description` (1–1024 characters) are checked; a skill that fails is shown to the owner with why (a code the apps put in words) and never offered to an assistant. Its files, at most 64, are listed with their SHA-256; an assistant reads `SKILL.md` or a file the skill lists, by exact path, as text up to 1 MB. Oikome's own skill, `oikome-knowledge`, is served by the node and listed first. Listing and reading skills is in Activity.

## 8. Conversions

| Original | Converter | Produces | Where it runs first |
|----------|-----------|----------|---------------------|
| Markdown, text, code, notes | Split by headings, then paragraphs, keeping the heading path | `text` chunks | Node (Rust) |
| PDF with a text layer | Text per page with positions | `text` chunks with page and box | Node |
| Scanned PDF, photos of documents | OCR | `ocr` chunks with page and box | Later (K4) |
| Photos | EXIF, thumbnail (exists), optional caption, image vector | `media` row, `caption` chunk | Node for EXIF; captions and vectors later (K4) |
| Audio, video | Speech to text, keyframes | `media` clips, `transcript` chunks with times | Later (K4) |
| CSV, TSV, Parquet, JSON Lines, Excel | Sniff types, infer roles, write Iceberg | Analysis table; `table_row` chunks for text fields | Node (DuckDB) |
| Connector records (flomo memos) | Long text fields become chunks | `text` chunks | Node |
| Skill folders | Parse front matter | Skill registry, `skill` chunks | Node |
| Office documents | Content plugin (content-viewing.md) | `text` chunks | Plugin |

Defaults follow the original: documents and media go to knowledge, tables to analysis tables. Chunking is automatic with a preview of the first chunks. The options are few: the chunk mode (automatic, by heading, by paragraph, one per row) and the model. Large numbers of parsing knobs are what knowledge-base users find hardest (RAGFlow has twelve templates times several parsers; Dify built its Agent knowledge base largely to hide them).

### 8.1 Runtimes

- **Node.** Splitting, PDF text, EXIF, table sniffing and profiles, and the default text-embedding model (Qwen3-Embedding-0.6B) through candle, so a collection can be prepared without anything else installed. The model is downloaded only when the owner chooses it, with its size shown first (no hidden downloads, roadmap PR 28).
- **Media recognition (K4).** OCR, speech to text, captions and image vectors need larger models. Their runtime is decided when K4 starts: candle has Rust implementations of Whisper, TrOCR, PaddleOCR-VL and SigLIP, and a content plugin with a sandbox (PR 28's RAW plugin host) is the other route. Vane is not integrated (decided 2026-11-17). Whatever runs them, the node decides channels, clips and provenance and commits the results itself, so it stays the single writer of the dataset (architecture principle 2).
- **Provider.** An embedding or vision API the owner configures with their own key, kept like storage credentials. Never the default and not part of K1 or K2: it sends content off the computer, and the apps say so before the first use.

### 8.2 In the apps

- **Prepare.** A collection's page offers Prepare. Its dialog shows what will be prepared (all originals, a selection, or one table), the result with its format preselected, and the model (none, or a local one). Formats are explained in one sentence each:
  - Knowledge (Lance): text, vectors and indexes kept together for search and for AI assistants.
  - Analysis table (Iceberg): columns stored for totals, grouping and charts; readable by most analysis tools.
  - A preview shows the first chunks, or the inferred fields for a table, before anything is written. A table's inferred fields are reviewed and corrected there (PR 27).
- **Progress.** Each original has one plain status: Waiting, Reading, Splitting, Embedding, Ready or Failed, with the reason and Retry. A collection says how many originals are ready, failed, or prepared with older settings.
- **Knowledge view.** Per original: its chunks, each with its place in the original (page, time, row), Disable and Edit. A **Retrieval test** box searches the collection and shows each hit's rank, score, source and the time taken; a question can be saved and run again after a change, as Dify's golden questions and Cherry Studio's recall test do.
- **Analysis view.** The table, its fields and their meanings, and charts by time and category (the existing series view and PR 27's derived analyses).
- **Info.** Format, model and dimensions, chunk count, dataset version and size, for those who want them.

## 9. AI access over MCP

Today assistants have `list_collections`, `search` (full text) and `read_row`. They become:

| Tool | Does |
|------|------|
| `list_collections` | Also says what each holds in knowledge and tables, and its active model |
| `search` | Hybrid search (full text + vector) with filters (collection, channel, tags, time). Returns table rows and passages: each passage with its collection, document, heading, page, lines, an excerpt, a score and how it was found (words, meaning or both), not whole documents. A collection with a model is searched both ways and fused by rank; one without, by words. Collections are merged by each one's rank, since their scores need not be comparable. |
| `read` | A chunk with up to five neighbours, or a range of a document's text by characters or lines (at most 20,000 characters at a time), so an assistant can quote and cite the document and lines |
| `grep` | Exact words or a pattern within one document or collection, with line numbers, for what meaning-based search misses (after K1) |
| `query_table` | Read-only SQL over analysis tables the assistant may read, with row and time limits |
| `list_skills`, `get_skill` | Oikome's own skill, then the skills of the assistant's collections, each with its files and SHA-256 digests; a skill's `SKILL.md` or a file it lists, as text. The MCP Skills extension can be offered as well where a client supports it |

The tools follow the order list → search → read, as Cherry Studio's and Open WebUI's do. Citations are the document and lines in the tool results; `resource_link` is left until clients use it. Oikome ships one skill of its own that teaches assistants that order and to cite the original for every answer. No tool changes knowledge; preparing and editing stay the owner's.

Grants stay the rule:

- A whole-collection grant lets an assistant search that collection's knowledge.
- Where a collection has column grants or row policies, each hit is read again as the assistant before it is returned, as search already does for in-place files (architecture §7.7). A hit it may not read is dropped, never summarized.
- Lance search functions read the dataset directly, so only the node calls them, never an assistant's SQL.
- Every read is in Activity.

## 10. Versions, rebuilds and sharing

- **An original changes.** Its BLAKE3 differs, its conversions become `stale`, and only that asset is converted again. Chunks whose `content_hash` did not change reuse cached vectors.
- **A model changes.** A new `vec_<model>` column is added and filled in the background; search switches when it is full; the old column is dropped later. Vectors of two models never share a column (RAGFlow accepts a "compatible" model without re-embedding; Oikome does not).
- **A converter or setting changes.** Existing results keep serving. The apps say how many originals still use the old settings and offer to convert them again.
- **Owner edits.** A chunk the owner edits or disables is recorded as an override keyed by `chunk_id`, so a rebuild of an unchanged version keeps it, and a changed version asks.
- **Sharing.** A subscriber receives the knowledge dataset at one Lance version with its indexes (architecture §7.6). Searching it by meaning needs the same model to embed the query: the subscriber has it, or the query goes to the owner (query shipping).

## 11. Decisions and open questions

Decided on 2026-11-17:

| Question | Decision |
|----------|----------|
| Default text model | Qwen3-Embedding-0.6B: Apache-2.0, Chinese and English, 1024 dimensions, 32K context, a query instruction for searches. Run locally through candle. BGE-M3 (MIT) is the alternative if a benchmark on real collections favours it. The jina models are excluded by their non-commercial licenses. |
| Where content is processed | On this computer by default. A cloud embedding or vision API is used only after the owner adds their own key and is told that content leaves the computer; not in K1 or K2. |
| Vane | Not integrated. Media recognition (K4) gets its own runtime decision. |
| Who writes Lance | DuckDB's `lance` extension writes, reads and searches (§3.1). The Rust `lance` crate is out while its dependencies include MPL-2.0 code (changed during K1). |
| Hybrid search | Reciprocal rank fusion in the node (each list adds 1 / (60 + rank)), not `lance_hybrid_search`: words keep their every-word filter, both lists drop replaced and turned-off chunks before fusing, and each hit says how it was found (decided during K2). |
| Where the model is chosen | Per collection, in Prepare or later in its knowledge view; the model's download is shown where it is needed and in Settings › AI Models, which also removes it (decided during K2). |
| Chinese full text in Lance | `ngram` base tokenizer (§3.1). |
| Analysis tables | Iceberg through the existing write path, with field meanings in the profile and in each field's `doc` (§3.1, §6). |
| One dataset per collection or per node | Per collection, so grants, sharing, subscriptions and removal stay as they are. Search across collections runs per collection and merges the results; collections with different models are searched with each one's own model. |
| How much format choice to show | Per Prepare, with the default preselected and each format explained in one sentence (§8.2). Of the applications surveyed, AnythingLLM shows its store preselected and explained; Cherry Studio and Open WebUI do not show it. |

Still open:

1. **Image model**, decided with K4: Qwen3-VL-Embedding-2B (text, images and video in one space) or SigLIP 2 (smaller, image–text only).
2. **Media runtime**, decided with K4 (§8.1).
3. **Skills from other people.** Whether a collection shared by a contact may offer its skills to the owner's assistants, and with what warning.

## 12. Delivery

| Step | Delivers | Owner can |
|------|----------|-----------|
| K1 (done) | `assets`, `conversions` as jobs, `chunk_overrides`; Markdown, text, code and PDF text to `chunks`; full-text search over chunks; retrieval test; MCP `search` and `read` over chunks | Prepare a collection of documents and let an assistant read it with sources |
| K2 (done) | Qwen3-Embedding-0.6B in the node, `vec_<model>`, hybrid search, model download and rebuild; skills in prepared collections and Oikome's own, MCP `list_skills` and `get_skill` | Search by meaning; give assistants skills |
| K3 | Table profiles (PR 27) extended with units and column usage; `table_row` chunks; `query_table` | Analyze a spreadsheet as a table and ask about it |
| K4 | OCR, speech to text, captions, image vectors; `media` (runtime decided when it starts) | Find what is written on photos and said in recordings |

The acceptance workspace's `oikome-docs` collection (this project's own documents) is the first real knowledge base for K1 and K2.
