# Next Phases: Research

> **Status**: research notes, checked 2026-11-18, for the plan in [`../roadmap.md`](../roadmap.md) (PR 29–32 and later). Libraries and models move quickly; versions are given with each claim. Claims marked *unverified* came from secondary sources or could not be confirmed in the primary one. Nothing here is implemented.

The agreed order is D (finish file sources) → E (backup and restore, then read-only datasets) → C (the plugin sandbox and RAW). A (freer analysis through a third-party library in a plugin) and B (tables as knowledge, K3) come after. Media recognition was taken out of the plan on 2026-11-19; §3.1–3.3 are kept for when it is taken up again.

## 1. D: file sources across places, CSV sources as files, Excel

### 1.1 What the code does today

- A folder collection is an `InPlace` location: `place` (a storage, or this computer), `path`, and `sources: Option<Vec<InPlaceSource>>` for collections made of several roots. `InPlaceSource { root, files }` is documented as "an absolute local folder. Multi-source references are local only" (`crates/oikome-catalog/src/space.rs`). `walk()` in `crates/oikome-node/src/inplace.rs` refuses several sources on a storage ("multiple file references are available only on this computer") and maps every file to a path relative to the roots' common ancestor.
- A remote root (NAS over WebDAV, a drive through its API, a RustFS bucket) is walked through `RemoteStore` and its files are read through a size-bounded cache (`cache/places`), which tables, previews, knowledge and analysis already use for a single remote root.
- Connector sources (CSV files, flomo) write rows into a transactional SQLite collection; `sources.json` keeps their targets. Since PR 27 the Sources page no longer offers CSV, but registered CSV sources keep syncing.
- Analysis tables are made from CSV, TSV, Parquet and JSON Lines files (PR 28).

### 1.2 Design notes

- **Roots on different places.** `InPlaceSource` gains `place: Option<String>`; `root` is then absolute on this computer or within that storage. Local roots keep today's paths (relative to their common ancestor), so nothing already indexed moves. Each remote root appears as a top-level folder named after its last segment (`archive/`), numbered on a clash. Everything that resolves "the place" of a collection (reads, the cache, thumbnails, data files for analysis and knowledge) resolves it per root instead. When a remote root cannot be reached, its files stay listed as unavailable and the local ones keep working, as a single remote root does today.
- **CSV sources to files.** A registered CSV source gets "Convert to File" on its page: its file joins a file collection the owner chooses (by reference, like File Import), its analysis table is made, and the source stops. The old transactional collection is kept until the owner removes it, because its shares and grants do not move; the dialog says so.
- **Excel.** See §4.2; the reader feeds the same analysis path, one table per sheet.

## 2. E: backup and restore, then read-only datasets

### 2.1 What a node keeps

Under the data folder: `.oikome/` (JSON registries such as `spaces.json`, `locations.json`, `storages.json`, `sources.json`, grants, contacts, plugins; `access_log.sqlite`; `analysis/`; `knowledge/<collection>/catalog.sqlite` and Lance datasets; `index/` and `cache/`, which are rebuildable; `extensions/` and downloaded models, which are re-downloadable; `identity/`), and one folder per Iceberg namespace or SQLite collection. Credentials are in the system keychain, not in the folder. The owner's original files are referenced, never copied.

### 2.2 Options for backup

| Option | License, version | Notes |
|---|---|---|
| rustic_core 0.14.0 + rustic_backend 0.8.0 (2026-11-17) | Apache-2.0 OR MIT ([crates.io](https://crates.io/api/v1/crates/rustic_core)) | A library that reads and writes the restic repository format: encryption, deduplication, checks. Backends through opendal 0.59.5, the version Oikome already pins (fs, s3, webdav…) ([Cargo.toml](https://raw.githubusercontent.com/rustic-rs/rustic_core/main/crates/backend/Cargo.toml)). The README calls the API early and changing ([rustic_core](https://github.com/rustic-rs/rustic_core)). |
| restic 0.19.1 as a program (2026-07-05) | BSD-2-Clause ([restic](https://github.com/restic/restic)) | A Go binary per platform; JSON output and documented exit codes ([scripting](https://restic.readthedocs.io/en/stable/075_scripting.html)). The `restic` crate on crates.io is an unrelated MPL-2.0 stub. |
| Own format: tar 0.4.46, blake3 1.8.7, chacha20poly1305 0.11.0 (audited), age 0.12.1 (beta) | MIT/Apache | Deduplication, retention, checks and the encryption framing would be ours to build and get right. |

**Consistent copies.** SQLite: the online backup API or `VACUUM INTO`, never a raw copy of a WAL database ([backup](https://sqlite.org/backup.html), [VACUUM](https://sqlite.org/lang_vacuum.html); rusqlite `backup` feature). Iceberg: data and metadata files are immutable and a commit swaps the metadata pointer, so copy the catalog first, then the files it names ([spec](https://iceberg.apache.org/spec/)). Lance: each version has an immutable manifest; `cleanup_old_versions` must not run during a copy ([format](https://lance.org/format/table/)). Compaction, snapshot expiry and settings writes pause while a backup is staged.

**Pitfalls.**
- Iceberg before v4 stores absolute paths ([spec](https://iceberg.apache.org/spec/)). Restoring to another folder or computer needs the paths rewritten; Java has `rewrite_table_path`, nothing equivalent was found in Rust (*unverified*). Oikome uses snapshot ids only to pin a query, not as history the owner sees, so a restore elsewhere can make each table again from its current snapshot.
- Lance datasets use relative paths and move as they are.
- Keychain credentials are not in the folder: places and connectors are signed in again after a restore. DuckDB persistent secrets are stored unencrypted and are not used ([secrets](https://duckdb.org/docs/1.5/configuration/secrets_manager.html)).
- The identity seed (the recovery key) stays out of the backup. A restic repository can hold several keys ([encryption](https://restic.readthedocs.io/en/stable/070_encryption.html)): one derived from the recovery key with HKDF under its own label, optionally one from an owner passphrase. A password kept only in the keychain would be lost with the computer.
- Original files are not in the backup. Their paths and hashes are recorded so a restore can say which are missing.
- A restore is checked before it is used: the repository check with a read subset (rustic `CheckOptions`), then a restore into a scratch folder with `PRAGMA integrity_check`, each Iceberg table's current snapshot loaded, each Lance dataset opened, BLAKE3 hashes and row counts compared with the manifest written at backup time.

**Recommendation.** rustic_core behind a small internal trait (its API may change), pinned exactly; a staging step that makes the consistent copy and a manifest; destinations through the same opendal places (a local folder or disk, a NAS, RustFS). The repository stays readable by the standard restic program, the way out if rustic fails. A single-file export can come later.

### 2.3 Read-only datasets

- DuckDB's `iceberg` extension is a core extension, signed and loaded on demand; `iceberg_scan` reads a table read-only from an explicit `metadata.json` or version, `snapshot_from_id` reads an older one, REST catalogs need S3/GCS-backed storage ([overview](https://duckdb.org/docs/1.5/core_extensions/iceberg/overview.html), [options](https://duckdb.org/docs/1.5/core_extensions/iceberg/iceberg_options.html), [catalogs](https://duckdb.org/docs/1.5/core_extensions/iceberg/catalogs)).
- The `lance` extension (already used) reads datasets by path and on S3-compatible storage with a `TYPE LANCE` secret; osx_amd64 is not among its platforms ([lance](https://duckdb.org/docs/1.5/core_extensions/lance.html), [cloud](https://github.com/lance-format/lance-duckdb/blob/main/docs/cloud.md)).
- `httpfs` reads Parquet and CSV on S3-compatible storage with an endpoint and path-style URLs; RustFS is not on its tested list (*unverified*, to be tested) ([s3api](https://duckdb.org/docs/1.5/core_extensions/httpfs/s3api.html)).
- iceberg-rust 0.11.0 reads a table without a catalog (`StaticTable::from_metadata_file`) and registers an existing metadata file in a SQL catalog (`register_table`) ([StaticTable](https://docs.rs/iceberg/latest/iceberg/table/struct.StaticTable.html)).
- DuckDB 2.0 LTS was released on 2026-10-21 and 1.5 is supported until March 2027; lance-duckdb builds main against 2.0 ([announcement](https://duckdb.org/2026/10/21/announcing-duckdb-200), [lance-duckdb](https://github.com/lance-format/lance-duckdb)). Oikome is on 1.5.6; moving to 2.0 is its own step.

**First scope.** Lance datasets by path, then on RustFS, latest version; Iceberg tables from an explicit `metadata.json`, local or on RustFS, pinned to the snapshot seen when connecting and refreshed when the owner asks; Parquet and CSV on RustFS. Only SELECT runs on them; secrets are temporary, from the keychain per session; an offline dataset keeps its last-known schema and is shown unavailable. REST catalogs and migration come second: migrating copies the current snapshot into a new Oikome table (history dropped) for Iceberg, and the dataset folder for Lance (versions kept), recording the source URI, snapshot or version and hashes.

## 3. C: RAW and the plugin sandbox; media recognition (not planned)

candle 0.11.1 (2026-10-31), which the node already runs, has `whisper`, `siglip` (SigLIP 2 included), `paddleocr_vl`, `chinese_clip`, `blip` and `qwen3_vl` ([models](https://docs.rs/candle-transformers/latest/candle_transformers/models/index.html)).

### 3.1 Speech to text

| Model | License | Size | Notes |
|---|---|---|---|
| Whisper large-v3-turbo | MIT | 809M parameters (4 decoder layers) | In candle's Whisper; Chinese and English ([card](https://huggingface.co/openai/whisper-large-v3-turbo)). candle quantizes only tiny ([example](https://raw.githubusercontent.com/huggingface/candle/main/candle-examples/examples/whisper/main.rs)). |
| Whisper small | MIT | 244M | The fast choice for computers without a GPU. |
| Qwen3-ASR-0.6B / 1.7B (2026-01-29) | Apache-2.0 | ~0.9B / 1.7B | Better Chinese by the vendor's numbers (AISHELL-2 WER 3.15 vs Whisper large-v3 5.06) ([repo](https://github.com/QwenLM/Qwen3-ASR)); not in candle yet. |
| whisper.cpp via whisper-rs 0.16 | MIT / Unlicense | — | Fast on Metal and Core ML, but a second runtime and a C++ build ([whisper.cpp](https://github.com/ggml-org/whisper.cpp)). |

Excluded: SenseVoice (custom license), Parakeet v3 (no Chinese), older Moonshine non-English models (non-commercial). Speed: whisper.cpp's benchmarks suggest tens of times real time on Apple Silicon for turbo (*estimate*); candle's Whisper on Metal has no published benchmark and is measured before choosing.

### 3.2 OCR

| Option | License | Notes |
|---|---|---|
| Apple Vision (`VNRecognizeTextRequest`) | system | macOS only, no download, Simplified and Traditional Chinese with English ([docs](https://developer.apple.com/documentation/vision/recognizing-text-in-images)); Rust through `objc2-vision` 0.3.2. |
| PP-OCRv5 mobile through `ort` (`oar-ocr` 0.10.3) | Apache-2.0 / MIT-Apache | ~21 MB of models, Chinese and English ([PP-OCRv5](https://www.paddleocr.ai/latest/en/version3.x/algorithm/PP-OCRv5/PP-OCRv5.html), [oar-ocr](https://github.com/GreatV/oar-ocr)); adds the ONNX Runtime native library. |
| PaddleOCR-VL in candle | Apache-2.0 | ~0.9B, tables and formulas too; slow on CPU (*unverified*), needs layout detection for pages ([example](https://github.com/huggingface/candle/tree/main/candle-examples/examples/paddleocr-vl)). |

Excluded: Tesseract (C++ system dependency, weaker Chinese, *unverified*), ocrs (Latin only), candle's TrOCR (one English line at a time).

### 3.3 Image search

SigLIP 2 base (Apache-2.0, ~0.4B, multilingual tokenizer, mostly English training pairs) is wired in candle's example ([paper](https://arxiv.org/html/2502.14786v1)). Qwen3-VL-Embedding-2B (Apache-2.0, text, image and video, 30+ languages) is better for Chinese but not in candle ([card](https://huggingface.co/Qwen/Qwen3-VL-Embedding-2B)). Captions are optional: BLIP-large is BSD-3 and in candle; the moondream model candle's example loads is research-only.

### 3.4 RAW previews and the plugin sandbox

Every Rust RAW decoder found is LGPL or GPL: rawler 0.8.0 (LGPL-2.1, has `preview_image`), rawloader 0.37.2 (LGPL-2.1), imagepipe (LGPL-3.0), raw_preview_rs (GPL-3.0); LibRaw is LGPL-2.1 or CDDL-1.0 ([rawler](https://docs.rs/rawler/latest/rawler/decoders/trait.Decoder.html), [LibRaw](https://www.libraw.org/)). Most RAW formats embed a full-size JPEG preview (TIFF-based CR2, NEF, ARW, DNG, ORF, PEF behind IFD offsets; CR3 in ISO-BMFF; RAF at a header offset), so a small permissive extractor covers previews without a decoder.

For the sandbox, wasmtime 50.0.2 (Apache-2.0 with LLVM exception) gives memory limits (`StoreLimits`), CPU limits (epoch interruption, cheaper than fuel) and the component model; WASI grants directories only, so a one-file grant is a host import such as `read(offset, len)` on one handle ([StoreLimitsBuilder](https://docs.wasmtime.dev/api/wasmtime/struct.StoreLimitsBuilder.html), [WasiCtxBuilder](https://docs.wasmtime.dev/api/wasmtime_wasi/struct.WasiCtxBuilder.html)). Extism is WASI preview 1 only; OS process sandboxes differ on each system and macOS's is deprecated.

## 4. A: freer analysis in a plugin, and Excel

### 4.1 Libraries a plugin could ship

| Library | License, version | Fit |
|---|---|---|
| Perspective (`@perspective-dev/*`, formerly `@finos`) | Apache-2.0, 5.9.3 (2026-11-18), very active ([repo](https://github.com/perspective-dev/perspective)) | Grid, pivot (group by, split by), aggregates, filters, sort and 15+ WebGL chart types; Arrow natively; views saved and restored as JSON ([save/restore](https://perspective.finos.org/guide/how_to/javascript/save_restore.html)). A "virtual server" turns the view into queries and fetches only the visible slice, with a Rust implementation in `perspective-client` ([virtual servers](https://github.com/perspective-dev/perspective/blob/master/docs/md/explanation/virtual_servers.md)). About 5 MB of WASM. |
| Graphic Walker (Kanaries) | Apache-2.0 (logos separate), 0.6.0 (2026-11-12) ([repo](https://github.com/Kanaries/graphic-walker)) | Closest to Tableau-style drag and drop, with date drill-down, pivot and charts saved as JSON. Takes row objects, not Arrow; a `computation` callback lets the host aggregate. 1.25 MB gzip plus React 19. Its "Ask" feature calls Kanaries' server and stays off. |
| Mosaic / vgplot (UW) | BSD-3, 0.35.1 (2026-11-16) ([repo](https://github.com/uwdata/mosaic)) | Spec-driven charts over DuckDB with an Arrow connector; no drag and drop or pivot UI. |
| ECharts 6.1, Vega-Lite 6.5, Observable Plot 0.6.19, AntV S2/G2 | Apache-2.0 / BSD-3 / ISC / MIT | Renderers or pivot tables only, no exploration UI. |
| Evidence, Superset, Metabase | MIT / Apache-2.0 / AGPL | Not embeddable as a component, or need their own server. |

**Choice (2026-11-18).** The owner first asked for a Tableau-like experience, then settled the question differently: assistants will make charts over MCP, so how much a chart can express and how fast it is made matter most, and manual drag and drop is not required. Vega-Lite, which Graphic Walker itself renders with, becomes the chart language; neither Graphic Walker nor Perspective is used. Making Perspective feel like Tableau would have meant building the shelves, chart recommendation and date hierarchy ourselves over its fixed chart types; Graphic Walker (about 2.4 MB of TypeScript, two main maintainers, English, Simplified Chinese and Japanese only) could have been wrapped but not cheaply kept as a fork.

Plugin input that becomes SQL needs guarding in both leading candidates: Perspective's generic SQL model places user expressions in SQL as text, and Graphic Walker's query payload can carry `sql` expressions (*see each project's source*). The host therefore builds the SQL itself from validated fields, with quoted identifiers and bound parameters, refuses free expressions, and runs it read-only over the granted views only, with external access off and memory and time limits.

### 4.2 Where it computes

DuckDB-WASM in the iframe is 8.1 MB gzip, its stable releases stopped at 1.32.0, and its extensions download at load, which a plugin without network cannot do ([npm](https://www.npmjs.com/package/@duckdb/duckdb-wasm)). The node's DuckDB computes instead: the plugin sends a structured request (a Perspective view or a Graphic Walker query), the host answers with aggregated Arrow. Small tables may be sent whole and computed by the library itself.

### 4.3 Excel

calamine 0.38.0 (MIT, pure Rust, 2026-11-02, active) reads xls, xlsx, xlsm, xlsb and ods, sheet by sheet, with typed cells including dates ([docs](https://docs.rs/calamine/latest/calamine/)); there is no Arrow output, so the conversion is ours. DuckDB's `excel` extension reads xlsx only and would be a runtime download for Oikome's prebuilt libduckdb ([excel](https://duckdb.org/docs/2.0/core_extensions/excel.html)). calamine is the choice: one analysis table per sheet.

## 5. BI on the node's DuckDB (2026-11-19)

The owner's frame: Oikome's analysis is a BI, its computation is SQL in Oikome's own DuckDB (that is the premise), and the BI layer comes from a mature third-party product rather than being built here. Charts made by assistants over MCP lower the threshold, so expressive power and speed of making charts come first.

### 5.1 Why every BI must come through Oikome

DuckDB allows one writing process; another process can only open a file read-only, and would skip Oikome's grants and row policies anyway ([concurrency](https://duckdb.org/docs/1.5/connect/concurrency)). Every BI product that "supports DuckDB" opens its own. So a BI reaches Oikome's data only through an interface Oikome serves, behind one **SQL gateway**: who is asking (a BI, an assistant, a plugin, the owner), only queries let through, Oikome's semantic layer rewriting for grants and row policies, only granted tables listed, time and memory limits, every statement in the access log. Each protocol is a door in front of the same gateway.

### 5.2 Doors

| Door | Clients | Notes |
|---|---|---|
| Inside Oikome's own pages | Oikome's analysis view, plugins | Cheapest; no protocol |
| PostgreSQL wire (`pgwire` 0.42.3, MIT/Apache-2.0) | Superset, Metabase, Grafana, Redash, most SQL tools | Works on DuckDB 1.5. Targeting only a bundled Superset (with a DuckDB SQLAlchemy dialect over the Postgres transport) is about 3–4 weeks; full Postgres dialect and catalog compatibility for every BI is open-ended (BI upgrades break it) |
| Quack (DuckDB's client-server protocol) | DuckDB clients (CLI, Python, R, JDBC, Superset's duckdb-engine) | Native dialect, fast (10M rows in 0.89 s against 2.9 s over Flight). **Needs DuckDB 2.0 on both sides**: 1.5 clients push no whole queries down and fail on joins of two remote tables. Its authorization callback (a Rust function registered through duckdb-rs) sees every statement and may return rewritten SQL. Young: open bugs on parameterized queries (#323, which Superset's table browsing uses), dropped connections (#354, #356), `quack_stop` leaving sessions open (#361); the client's catalog listing must be filtered by rewriting Quack's internal queries ([2.0 security](https://duckdb.org/docs/2.0/quack/security), [duckdb-quack](https://github.com/duckdb/duckdb-quack)) |
| Arrow Flight SQL | DBeaver, ADBC clients | Superset, Metabase and Grafana support it poorly |

### 5.3 BI products compared

Superset (Apache-2.0, may be bundled, widest charts and dashboards, official MCP that builds charts and dashboards, heavy Python service) and Metabase (AGPL, installed by the owner, friendliest) are the full BI products worth connecting; Grafana is AGPL and time-series first; Rill, Lightdash and Evidence compute in their own DuckDB or need dbt/paid features; DataEase is GPL with extra terms; Datart is dormant. Each would come through a door above.

### 5.4 Perspective computing in Oikome's DuckDB: spike

Perspective (Apache-2.0, `@perspective-dev/*` 5.9.3) has a "virtual server": its SQL builder turns the grid, pivot, filters and aggregates into SQL, and a handler runs it. **Its bundled DuckDB handler runs on DuckDB-WASM in the browser, not on Oikome**, so a handler of our own is needed. Spike on branch `spike/perspective-duckdb` (a card plugin on the acceptance workspace's `household`):

- **Computation:** Perspective's builder writes the SQL; the page's executor never runs it. Perspective creates a temporary table per view; the executor keeps that definition in page memory only and sends each read as one `WITH <view> AS (<query>) SELECT …` to the node, which runs it as the plugin's principal through the semantic layer. Of 337 statements the node received, all were SELECT/WITH except four deliberate probes. Nothing is created in DuckDB, so views are isolated per page by construction. Totals matched the Analysis view.
- **Authorization:** another collection's table is "not found"; DROP, CREATE, ATTACH, SET, `read_csv` and two statements at once are refused; a row policy for the plugin (`category <> 'Health'`) removed that row and changed the total accordingly; every statement is in Activity.
- **Browser constraints:** Perspective needs WebAssembly compilation (`'wasm-unsafe-eval'`) and creates `blob:` scripts; its charts render in a **module** worker. In today's plugin sandbox (an opaque origin) classic blob workers start and module workers do not, so the grid and pivot worked and charts did not. Served from an origin of its own (still not the app's: no app cookie, `connect-src 'none'` still blocks the app's API), bar, line and treemap charts rendered with data computed by the node.
- **Rough edges:** the bundle is 7.3 MB (the plugin limit is 8 MB); one interaction issues dozens of small statements (describe, count, refresh); the spike's executor recognises Perspective's `CREATE … AS (…)` by its text, which a release could change, so the real executor belongs in the node (Rust) or upstream as a builder option that returns the query alone; expressions and expand/collapse were turned off.

### 5.5 Where Perspective lives

As **Oikome's own component in the Analysis view** (recommended), it is a pinned, reviewed dependency, needs no iframe, and the opaque-origin, worker and size problems disappear; the app's CSP must then allow WebAssembly compilation and `blob:` workers and scripts, which is reviewed on its own. Assistants make views declaratively (Perspective's view configuration as JSON over MCP) and Oikome's Perspective renders them, so no outside code runs. Third-party plugin pages stay in iframes, each on an origin of its own.
