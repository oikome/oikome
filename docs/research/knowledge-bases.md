# Knowledge-Base Applications and Formats: Research

> **Status**: research notes, checked 2026-11-17, for [`../knowledge-schema.md`](../knowledge-schema.md). Products move quickly; versions are given with each section. Claims marked *unverified* came from secondary sources or could not be confirmed in the primary one.

## 1. Personal and local applications

Versions: AnythingLLM Desktop 1.19.0, Open WebUI (main, 2026-11-16), Cherry Studio 2.2.2, Obsidian Smart Connections 4.7.2.

| | AnythingLLM | Open WebUI | Cherry Studio |
|---|---|---|---|
| Choices at creation | Global: vector store, embedder, chunk size 1000 / overlap 20. Per workspace: search preference, max snippets (4), similarity threshold | Name, description; extraction engine, splitter, chunk 1024 / overlap 100, embedder, hybrid search are admin-wide | Name and embedding model only; the model may be Disabled (BM25 only) |
| Vector store shown | "LanceDB (Built-in)… 100% local… no configuration needed", preselected | Not in the UI (`VECTOR_DB`, default Chroma) | Not shown (SQLite + sqlite-vec) |
| Inputs | PDF (text layer, else Tesseract OCR), Office, EPUB, Whisper for audio and video, images OCR only, sheets as CSV text | Many extraction engines (Docling, MinerU, Mistral OCR…); CSV one document per row | PDF, Office, EPUB, CSV, HTML; several OCR options; no media |
| Per chunk | Text with a title/date/source header, document metadata; **no page or offset** | Loader fields (source, page, row), `start_index`, SHA-256 of the text, `embedding_config` | Exact `[char_start, char_end)`, deterministic id, embeddings keyed by text hash, FTS5 trigram |
| Model change | Warning, then all workspaces reset | Admin Reindex re-extracts everything, overwriting edits | Adding a model fills vectors in place; changing one rebuilds into a new base |
| After ingestion | Pin a document (full text into context), watch for changes | File tree, preview, editable indexed text | Status labels, View Chunks, Recall Test (rank, relevance, ms, history) |
| Agents | Agent skills (RAG, SQL over external databases); MCP client only | Tools `list_knowledge`, `query_knowledge_files`, `grep_knowledge_files`, `view_file`; MCP client | MCP tools `kb_list`, `kb_search`, `kb_read`, `kb_manage` (approval needed); a bundled skill teaches list → search → read |

Smart Connections ships a local model ("Zero-setup… No API key"), keeps its index in the vault's `.smart-env/`, and reimports after a model change.

Sources: AnythingLLM docs and source ([LanceDB](https://github.com/Mintplex-Labs/anythingllm-docs/blob/main/pages/setup/vector-database-configuration/local/lancedb.mdx), [text splitting](https://github.com/Mintplex-Labs/anythingllm-docs/blob/main/pages/setup/embedder-configuration/text-splitting.mdx), [strings](https://github.com/Mintplex-Labs/anything-llm/blob/master/frontend/src/locales/en/common.js), [schema](https://github.com/Mintplex-Labs/anything-llm/blob/master/server/prisma/schema.prisma), [Lance provider](https://github.com/Mintplex-Labs/anything-llm/blob/master/server/utils/vectorDbProviders/lance/index.js)); Open WebUI ([knowledge](https://docs.openwebui.com/features/workspace/knowledge), [RAG](https://docs.openwebui.com/features/chat-conversations/rag/), [loaders](https://github.com/open-webui/open-webui/blob/main/backend/open_webui/retrieval/loaders/main.py)); Cherry Studio ([create](https://cherryai.com.cn/docs/en/knowledge-base/create/), [recall test](https://cherryai.com.cn/docs/en/knowledge-base/recall-test/), [technical design](https://github.com/CherryHQ/cherry-studio/blob/main/docs/dev/references/knowledge/experiment/knowledge-technical-design.md), [schema](https://github.com/CherryHQ/cherry-studio/blob/main/src/main/data/db/schemas/knowledge.ts), [MCP tools](https://github.com/CherryHQ/cherry-studio/blob/main/src/main/ai/mcp/servers/cherryKnowledgeTools.ts)); [Smart Connections](https://github.com/brianpetro/obsidian-smart-connections).

## 2. Team knowledge-base platforms

Versions: Dify 1.19.0, RAGFlow 1.0.0 (backend rewritten in Go), FastGPT 4.17.4, MaxKB 2.10.8-lts.

- **Dify** has two kinds of knowledge base. The classic one asks for a chunk mode (General, Parent-child, Q&A; fixed once created), an index method (High Quality with embeddings, which cannot go back, or Economical with 10 keywords per chunk), and retrieval (vector, full text or hybrid; TopK 3, threshold 0.5). The new **Agent knowledge base** (beta) asks only for models and a retrieval depth (Fast, Deep, Research): a reasoning model splits documents, writes section summaries and a graph, and a Quality page keeps golden questions and bad cases. Its `difyctl knowledge_fs` lets coding agents browse documents like folders.
- **RAGFlow 1.0** configures a dataset as Parser → Chunker → Extractor → Indexer: twelve templates (General, Q&A, Manual, Table, Paper, Book, Laws, Presentation, Picture, One, Audio, Email), several PDF parsers (DeepDOC, Docling, MinerU-style), chunks of 512 tokens by default, a title chunker that keeps the heading path, LLM extraction of keywords, questions, tags, summaries and metadata. Tables become one chunk per row, with each column marked **Indexing, Metadata or Both**, and chat tries LLM-written SQL first. It has a built-in MCP server (`ragflow_retrieval`, `ragflow_list_datasets`).
- **FastGPT** organizes knowledge base → collection → data, and gives each chunk several typed indexes (`default`, `question`, `summary`, `image`, `custom`), so one chunk has many vectors. Status shows stages (Queue, Parsing, Image analysis, Index vectorization, Ready); sync compares a hash of the raw text.
- **MaxKB** keeps paragraphs, linked questions and one `embedding` table with both a pgvector column and a `tsvector`.

| | Dify classic | RAGFlow 1.0 | FastGPT 4.17 | MaxKB |
|---|---|---|---|---|
| Chunk store | `document_segments` (Postgres) | Elasticsearch / Infinity record | `dataset_datas` (MongoDB) | `paragraph` (Postgres) |
| Parent-child | `child_chunks` | `mom_id` | sub-vectors in `indexes[]` | `chunks[]` |
| Position | position | page, position, box | chunkIndex | position |
| Enabled flag | chunk and document | `available_int` | collection `forbid` | `is_active` |
| Hash | node hash; embedding cache by (model, hash) | content and chunk hashes | `hashRawText` | file SHA-256 |
| Tables | rows as `col: value` text | row chunks, column roles, Text-to-SQL | template import | — |
| MCP | apps; agent knowledge via CLI | native retrieval tools | apps | apps |

Sources: Dify [models](https://github.com/langgenius/dify/blob/main/api/models/dataset.py) and [docs](https://github.com/langgenius/dify-docs/tree/main/en); RAGFlow [configuration](https://github.com/infiniflow/ragflow/blob/main/docs/guides/dataset/configuration.md), [retrieval testing](https://github.com/infiniflow/ragflow/blob/main/docs/guides/dataset/retrieval_testing.md), [MCP tools](https://github.com/infiniflow/ragflow/blob/main/docs/develop/mcp/mcp_tools.md), [mapping](https://github.com/infiniflow/ragflow/blob/main/conf/mapping.json); FastGPT [schemas](https://github.com/labring/FastGPT/tree/main/packages/service/core/dataset) and [design](https://github.com/labring/FastGPT/blob/main/document/content/self-host/design/dataset.en.mdx); MaxKB [models](https://github.com/1Panel-dev/MaxKB/blob/v2.10.8-lts/apps/knowledge/models/knowledge.py).

## 3. Formats

**Lance** (Lance 14.0.3, file format 2.2 the default; `lancedb` crate 0.41):

- One versioned table can hold media, structured columns and several vector columns; searchable metadata belongs in scalar columns so filters run before blobs are read ([multimodal](https://docs.lancedb.com/tables/multimodal)).
- Blob v2: inline up to 64 KiB, packed up to 4 MiB, dedicated above; legacy blob columns are rejected at format 2.2 ([Blob v2](https://lancedb.com/blog/lance-blob-v2/)).
- Indexes: BTree, Bitmap, LabelList, FTS (BM25), NGram, ZoneMap, BloomFilter, and vector IVF_PQ / IVF_SQ / IVF_HNSW_* ([index enum](https://docs.rs/crate/lancedb/latest/source/src/index.rs)).
- Full text for Chinese needs the `jieba` or `ngram` base tokenizer; `language` does not segment ([tokenizers](https://lancedb.github.io/lance/guide/tokenizer)).
- Adding a column adds files without rewriting data, so a new embedding model is a new `vec_<model>` column filled later ([schema](https://docs.lancedb.com/tables/schema)).
- Stable row ids must be enabled when the dataset is created; otherwise `_rowid` changes on compaction.
- Through DuckDB's `lance` extension: INSERT / UPDATE / DELETE / MERGE need an attached namespace; an UPDATE or MERGE after another write to the same table in one transaction is not implemented; writes to several tables are not atomic; JSON-typed columns break inserts; unknown index options are ignored silently ([lance-duckdb SQL](https://github.com/lance-format/lance-duckdb/blob/main/docs/sql.md), [DuckDB docs](https://duckdb.org/docs/current/core_extensions/lance)). The docs track the latest DuckDB; behaviour in 1.5.6 needs checking.

**Record layouts in frameworks.** LlamaIndex (`id, doc_id, vector, text, metadata`), LangChain (`vector, id, text, metadata`) and LanceDB's `LanceModel` all keep provenance in an untyped metadata field. Typed columns are better for filtering and indexing.

**Chunk identity and provenance.** Unstructured and Kreuzberg derive chunk ids from text, position and page (Kreuzberg with BLAKE3); Docling keeps page, box and character span per item (its boxes have a bottom-left origin, *unverified*); W3C Media Fragments (`#t=`, `xywh=`) and Web Annotation selectors are the standard anchors. Incremental indexing keys on a content hash per source (LangChain record manager, LlamaIndex docstore).

**Tables.** DuckDB's CSV sniffer infers dialect and types, not roles ([auto detection](https://duckdb.org/docs/current/data/csv/auto_detection)). BI tools add roles: Tableau measures and dimensions, Power BI data categories and default aggregation; dbt MetricFlow entities, dimensions and measures; Apache Ossie (incubating, formerly OSI) a YAML interchange of datasets, fields and metrics ([Ossie](https://ossie.apache.org/)).

## 4. Embedding models (local, Chinese and English)

| Model | Dimensions | Context | License | Rust route |
|---|---|---|---|---|
| Qwen3-Embedding 0.6B / 4B / 8B | 1024 / 2560 / 4096, shorter on request | 32K | Apache-2.0 | candle `qwen3`, fastembed-rs |
| BGE-M3 | 1024, plus sparse and multi-vector | 8192 | MIT | fastembed-rs ONNX |
| multilingual-e5-large | 1024 | 512 | MIT | ONNX; needs `query:` / `passage:` |
| nomic-embed-text-v2-moe | 768, shorter on request | 512 | Apache-2.0 | candle |
| EmbeddingGemma-300m | 768, shorter on request | 2K | Gemma licence | ONNX (Vane uses it) |
| jina-embeddings-v3 / v4 | 1024 / 2048 | — | non-commercial | — |
| Qwen3-VL-Embedding 2B / 8B (text, image, video) | 2048 / 4096 | 32K | Apache-2.0 | candle `qwen3_vl` |
| SigLIP 2 (image–text) | 768–1152 | — | Apache-2.0 | candle `siglip` |

Sources: the models' Hugging Face cards, [fastembed-rs](https://github.com/Anush008/fastembed-rs), [candle models](https://docs.rs/candle-transformers/latest/candle_transformers/models/index.html).

## 5. Skills and MCP

- **Agent Skills** ([specification](https://agentskills.io/specification)): a folder with `SKILL.md` (`name` 1–64 characters `[a-z0-9-]` matching the folder, `description` up to 1024 characters, optional `license`, `compatibility`, `metadata`, `allowed-tools`), plus `scripts/`, `references/`, `assets/`. About 100 tokens of metadata are always loaded, the body on activation, other files on demand.
- **MCP 2026-07-28** ([changelog](https://modelcontextprotocol.io/specification/2026-07-28/changelog)) is stateless (no sessions, no `initialize`); results may carry `resource_link`s and `structuredContent`. Resources are application-driven and servers must sanitize paths.
- **Skills extension** SEP-2640 ([proposal](https://modelcontextprotocol.io/seps/2640-skills-extension)): `skill://<path>/SKILL.md`, `skills/list` with per-file SHA-256 digests, `skills/get`; final since 2026-09, few clients yet.

## 6. What Oikome takes from this

1. **Keep tables as tables.** Every product here flattens spreadsheets into text chunks or writes SQL against a search engine. Oikome has DuckDB and Iceberg: rows stay in a table, text columns also become chunks, other columns become filters (RAGFlow's Indexing / Metadata / Both).
2. **Keep the position of every chunk** (page, box, characters, time, row). AnythingLLM loses PDF pages; RAGFlow uses positions to highlight the source.
3. **Content-addressed chunks and an embedding cache by (content hash, model)** (Cherry Studio, Dify).
4. **Full text first, vectors added later in place** (Cherry Studio).
5. **Never mix models in one vector field; rebuild in the background, then switch** (RAGFlow's "compatible model" shortcut is the counter-example).
6. **A plain status per item, an error list with retry, and the previous version serving during a rebuild** (Cherry Studio, FastGPT, Dify).
7. **Inspectable results**: chunk views, disable and edit, a retrieval test with rank, score and time, saved questions re-run after changes (Cherry Studio, Dify Agent).
8. **Few settings, good defaults, a preview** (Dify's Agent knowledge base exists to hide chunking; RAGFlow's matrix is the counter-example).
9. **Assistant tools shaped like files**: list → search → read and grep with ranges, scoped to what is granted, changes needing the owner (Cherry Studio, Open WebUI, Dify's `knowledge_fs`), and a skill that teaches the order.
10. **Formats explained and preselected** rather than hidden: AnythingLLM's "LanceDB (Built-in)" is the model.
