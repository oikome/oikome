# Oikome — Architecture v3.0

> **Version**: v3.0
> **Date**: 2026-09-27
> **Codename**: oikome
> **Positioning**: An AI-native, multimodal private data center that usually runs on personal resources and supports trusted data exchange. Built on a decentralized local federated query semantic layer: embedded, sovereign, P2P-native.
> **Core Change**: Oikome no longer owns query *computation*. Computation is delegated to pluggable embedded engines (DuckDB for analytics, SQLite for transactions; browser-hosted engines in WASM). DataFusion is retained **only** as the federation coordinator that merges results across engines and nodes. Oikome's own value concentrates in the semantic layer (naming, policy, views), the federation layer (routing, pushdown, merge), identity/authorization (DID/UCAN) and P2P (discovery, snapshot sync, Arrow Flight).
> **Status**: 3.0-A (engines), 3.0-B (local federation), 3.0-C (semantic layer) and 3.0-D (publishing) implemented on `main`. Supersedes v2.0.
> **Revised 2026-10-30**: positioning changed to an AI-native, multimodal private data center (§1). Added cloud drives (§7.5), Lance as a third Location format (§7.6), connectors (§11), compute sharing (§12) and roadmap phases 3.4–3.6. The engine, federation and semantic layers are unchanged.
> **Revised 2026-11-01**: added the user-facing apps: repository layout (§13.3), apps and the API layer (§13.4) and an apps track beside the core phases (§14.1).
> **Revised 2026-11-02**: storages (§7.5): a NAS at home first (3.4a), cloud drives step by step, each once files are encrypted on the client (3.4b onward). 3.4b: client-side encryption, drives through their sync folders, credentials kept by the node (keychain).

---

## v2 → v3 Change Summary

| Dimension | v2 (DataFusion Core) | v3 (Semantic Layer + Pluggable Engines) |
|-----------|----------------------|------------------------------------------|
| Identity of the project | Multimodal columnar data lake | Decentralized federated query **semantic layer** |
| Computation | DataFusion executes everything | **Engines** compute: DuckDB (OLAP), SQLite (OLTP), host-provided (browser) |
| DataFusion role | Parser + optimizer + executor | **Federation coordinator only**: merges Arrow streams from engines and remote nodes |
| Storage model | Columnar-only (Iceberg + Parquet) | **Two storage classes**: analytical (Iceberg + Parquet) and transactional (SQLite file) |
| Space ↔ engine | Implicit (always DataFusion) | **Explicit per Space**: `KIND = analytical \| transactional` selects engine + storage class |
| Space ↔ path | 1 : 1 | **Space (logical) and Location (physical) are decoupled**; N Spaces may bind one Location, at most one writable |
| Custom operators / indexes | ART, CSR, HNSW, Cypher planned | **Dropped**. Multimodal delegated to engine extensions (DuckDB `vss`/`fts`/DuckPGQ, SQLite FTS5/`sqlite-vec`) |
| P2P sync unit | Iceberg snapshots | Iceberg snapshots (unchanged); transactional Spaces **publish** Parquet snapshots |
| Browser node | DataFusion compiled to WASM (~10MB, risky) | Rust core (catalog, identity, federation planner) in WASM; **engines provided by JS host** (duckdb-wasm, wa-sqlite on OPFS) |
| Engine bridging code | Self-written `IcebergTableProvider` | One generic `EngineSqlExecutor` implementing `datafusion-federation`'s `SQLExecutor` over any `QueryEngine`; engines own their connections |

**What stays the same**: Space sovereignty, single write master per Space, DID/UCAN, deny-by-default, Iceberg catalog for analytical Spaces, Parquet as the interchange/publication format, VFS (opendal), Arrow Flight + libp2p for the network layer.

**Rescinded v1 constraints**: #2 *Columnar-Only Storage* (SQLite is row-oriented) and the Layer 3 *self-built vectorized engine*.

---

## 1. Positioning

Oikome is an **AI-native, multimodal private data center** that usually runs on personal resources and supports trusted data exchange.

AI platforms hold more and more personal data, and storage and compute are concentrating in a few companies. Once data is handed to a platform, its owner no longer controls who uses it or how. Oikome keeps data and compute with individuals, and lets them pool both with each other rather than with a platform.

| | What it means in Oikome | Where |
|---|---|---|
| Private | The data belongs to its owner — a person, a family or a small team — and lives on storage the owner controls: local disk, NAS, cloud drives. Connectors bring it in from the apps the owner uses | §7, §11 |
| Multimodal | Text, images, audio, files and vectors are first-class data alongside tables | §7.6, §8 |
| AI-native | AI is a primary user from the start. AI tools and agents connect through MCP like any other app (dashboards, notes), as principals that see only what they are granted. Data is kept in forms AI uses directly: text, embeddings, search indexes. Views and names tell AI what the data means. AI workloads such as embedding and inference run on personal resources | §2 L5, §6, §7.6, §12 |
| Personal resources | Storage and compute come from individuals, not from a platform's cloud: first the owner's own machines, which compute with embedded engines (DuckDB, SQLite); then other individuals' machines through compute sharing. "Usually": a cloud bucket can still serve as a Location | §4, §7, §12 |
| Trusted data exchange | Data moves only between data centers that trust each other, in both directions, under grants the owner can revoke | §5, §6, §9 |

**Data exchange and compute sharing use the same trust mechanism with different authorizations.** Both rest on DID identities, peer relationships, deny-by-default and UCAN revocation (§9). Data is authorized with `space/query` / `space/read` plus grants and policies (§5.3, §6); compute with `compute/<runtime>` (§12). Neither implies the other.

> **Oikome decides *who may see what*, *where data comes from*, *where it lives* and *where a query or task is sent*. It does not decide *how to compute*.**

Analogy: Oikome is to embedded engines what a service mesh is to services. DuckDB and SQLite are excellent local engines but know nothing about identity, capability delegation, peers, or multi-Space namespaces. Oikome wraps them into a sovereign, federated, P2P-addressable data layer without re-implementing any of their query processing.

### 1.1 Design Principles (v3, Immutable)

1. **Compute is delegated.** Oikome never implements scan, join, aggregate, or index algorithms. If an engine cannot do something, we pick or configure an engine — we do not write an operator.
2. **Single Write Master per Space** (unchanged). Write delegation is explicit via UCAN.
3. **Space is the sovereignty unit; Location is the physical unit.** They are decoupled. Authorization, engine selection and sync policy attach to the Space; format and residency attach to the Location.
4. **Immutable files are the only thing that crosses the network.** P2P synchronization exchanges Iceberg snapshots (Parquet + manifests) and Lance versions (data files + manifest). Transactional state never replicates live; it is *published* as snapshots.
5. **Deny by default** (unchanged). No peer relationship → no catalog visibility → no query possibility. Unauthorized tables "do not exist".
6. **Engines are host-replaceable.** The `QueryEngine` contract must be implementable by a native Rust crate *or* by the embedding host (JS in the browser, FFI elsewhere).
7. **Format interoperability** (unchanged). Parquet / Arrow / Iceberg / SQLite files remain directly usable by external tools with zero export.

---

## 2. Layered Architecture

```
┌──────────────────────────────────────────────────────────────────────┐
│ L5  Interface        SQL API · CLI · HTTP · WASM/JS binding · MCP     │
│     Connectors       import from apps: official APIs · export files   │
├──────────────────────────────────────────────────────────────────────┤
│ L4  Semantic Layer ★                                                  │
│     Unified namespace  space://<did>/<space>.<table>                  │
│     Views · Row/column policies (UCAN → SQL rewrite) · Enumeration    │
│     resistance · Statement classification (DDL / DML / TCL / query)   │
├──────────────────────────────────────────────────────────────────────┤
│ L3  Federation Layer ★  (DataFusion as coordinator)                   │
│     sqlparser → resolve table refs → per-Space subtrees               │
│     → EngineTableProvider (pushdown as engine-dialect SQL)            │
│     → RemoteTableProvider (Arrow Flight, query shipping)              │
│     → DataFusion merges Arrow streams (join / agg across Spaces)      │
├──────────────────────────────────────────────────────────────────────┤
│ L2  Engine Abstraction ★                                              │
│     QueryEngine trait · Capabilities · Dialect                        │
│     ├─ DuckDbEngine   (analytical; Parquet lists, .sqlite, .lance)    │
│     ├─ SqliteEngine   (transactional; DML + transactions)             │
│     └─ HostEngine     (browser: duckdb-wasm / wa-sqlite via JS)       │
├──────────────────────────────────────────────────────────────────────┤
│ L1  Catalog & Storage                                                 │
│     Space registry · Location registry · Iceberg metadata · Lance     │
│     Parquet write path (AppendBuffer → Parquet → Iceberg commit)      │
│     SQLite → Parquet publisher · Compaction                           │
│     VFS (opendal): disk · NAS · S3 · cloud drives · OPFS              │
├──────────────────────────────────────────────────────────────────────┤
│ L0  Identity & Network ★                                              │
│     DID · UCAN issue/verify/revoke · Peer state machine               │
│     libp2p discovery · Snapshot sync · Arrow Flight RPC               │
│     Compute sharing: tasks to peers' registered runtimes (UCAN)       │
└──────────────────────────────────────────────────────────────────────┘
        ★ = Oikome core value.  Others are thin glue.
```

---

## 3. Core Abstractions

### 3.1 Space — Logical Sovereignty Unit

```rust
pub struct Space {
    pub uri: SpaceUri,              // space://did:oikome:7xK9y/blog
    pub kind: SpaceKind,            // Analytical | Transactional
    pub engine: EngineKind,         // DuckDb | Sqlite | Host(String)   (default derived from kind)
    pub location: LocationId,       // binding to a physical Location
    pub access: AccessMode,         // Writable | ReadOnly
    pub owner: Did,                 // write master
    pub policies: Vec<PolicyId>,    // row/column policies (semantic layer)
    pub views: Vec<ViewDef>,        // named SQL views over this Space
    pub publish: Option<PublishPolicy>, // transactional only: how/when to emit Parquet snapshots
}

pub enum SpaceKind { Analytical, Transactional }
```

`CREATE SPACE` syntax (extends v2):

```sql
CREATE SPACE blog   WITH KIND = 'analytical',    STORAGE = 'disk';        -- Iceberg + Parquet, engine DuckDB
CREATE SPACE orders WITH KIND = 'transactional', STORAGE = 'disk';        -- orders.sqlite, engine SQLite
CREATE SPACE orders_ro WITH KIND = 'analytical', LOCATION = 'orders', ACCESS = 'readonly';
--  ↑ second Space over the *same* Location (orders.sqlite), served by DuckDB via sqlite_scanner
```

### 3.2 Location — Physical Residency

```rust
pub struct Location {
    pub id: LocationId,
    pub format: LocationFormat,      // IcebergParquet { namespace } | SqliteFile { path } | Lance { root }
                                     // | Remote / Replica (another node's Space) | StorageCopy (§7.5)
    pub residency: Vec<String>,      // "disk"
    pub writer: Option<String>,      // at most ONE writable Space per Location
    pub storage: Option<StorageBinding>, // the storage (a NAS) the Location is kept on as well (§7.5)
}
```

**Binding rules** (enforced by the catalog at `CREATE SPACE` / `ALTER SPACE`):

| Rule | Description |
|------|-------------|
| Format ↔ engine compatibility | The Space's engine must be able to read the Location's format (see §4.3 matrix). Violations are rejected at DDL time. |
| Single writer | `Location.writer` is `None` or exactly one Space. Any further binding must be `ACCESS = 'readonly'`. |
| Kind ↔ format | `Transactional` requires `SqliteFile`. `Analytical` accepts `IcebergParquet`, `Lance` or (read-only) `SqliteFile`. |
| Independent policies | Two Spaces over one Location carry independent UCAN scopes, policies and views. This is the intended way to expose the same data under different governance. |

### 3.3 Node Types

Unchanged from v1 (Full / Edge / Browser), with one refinement: the node type no longer changes *what engine code ships*; it changes *which `QueryEngine` implementations are registered*.

| Node | Registered engines | Coordinator |
|------|--------------------|-------------|
| Full / Edge | `DuckDbEngine`, `SqliteEngine` (native, bundled) | DataFusion (native) |
| Browser | `HostEngine(duckdb-wasm)`, `HostEngine(wa-sqlite)` | DataFusion (WASM, `sql` + minimal features; only merges Arrow, never scans files) |

### 3.4 Catalog

Per-node registry with three tables: **Spaces**, **Locations**, **Peers**. Peers (3.1c) records `DID → Flight endpoint` as found by discovery (§9), in `.oikome/peers.json`, so every process of the node can resolve a remote Space's owner by DID. Plus per-analytical-Space Iceberg metadata (unchanged). New in v3: `policies`, `views`, `engine` and `publish` are catalog entities, versioned with the Space.

---

## 4. Engine Abstraction Layer (L2)

### 4.1 The `QueryEngine` Contract

```rust
#[async_trait]
pub trait QueryEngine: Send + Sync {
    fn kind(&self) -> EngineKind;
    fn dialect(&self) -> Dialect;                 // sqlparser dialect used for SQL generation
    fn capabilities(&self) -> Capabilities;

    /// Make a physical source visible to the engine under `name`.
    async fn attach(&self, name: &str, source: TableSource) -> Result<()>;
    async fn detach(&self, name: &str) -> Result<()>;

    /// Run a query; results stream back as Arrow.
    async fn query(&self, sql: &str, params: &[ScalarValue]) -> Result<SendableRecordBatchStream>;

    /// Run a statement without a result set (DML / DDL / TCL). Transactional engines only.
    async fn execute(&self, sql: &str, params: &[ScalarValue]) -> Result<u64>;
    async fn begin(&self) -> Result<TxHandle>;    // Err(Unsupported) for analytical engines
}

pub enum TableSource {
    ParquetFiles { files: Vec<VfsPath>, schema: SchemaRef },   // resolved from an Iceberg snapshot
    SqliteFile   { path: VfsPath, table: String },
    ArrowBatches { schema: SchemaRef, stream: SendableRecordBatchStream }, // remote results, temp tables
}

// A whole Space exposed as `<space>.<table>`, read and written through the engine.
pub enum SpaceSource {
    LanceNamespace { root: VfsPath },                          // a directory of <table>.lance datasets (§7.6)
}
// QueryEngine::attach_space / detach_space; engines that cannot keep the default (refuse).

bitflags! Capabilities {
    OLAP, OLTP, TRANSACTIONS, READ_PARQUET, READ_SQLITE, READ_LANCE, ARROW_OUT,
    EXT_VECTOR, EXT_FTS, EXT_GRAPH, PUSHDOWN_JOIN, PUSHDOWN_AGG
}
```

### 4.2 Implementations

| Engine | Crate (license) | Notes |
|--------|-----------------|-------|
| `DuckDbEngine` | `duckdb` 1.10506 (DuckDB 1.5.6, MIT, prebuilt `libduckdb`) | Parquet via `read_parquet([...])`; SQLite via the `sqlite` extension; Lance via the `lance` extension (§7.6); Arrow out natively. Extensions are loaded on demand from the node's extension directory and never downloaded by a query (`oikome extensions install` fetches them, signed by DuckDB). Each statement runs on a fresh connection to the shared database, because the `lance` extension caches datasets per connection. |
| `SqliteEngine` | `rusqlite` 0.40 (MIT, `bundled`) | WAL mode; FTS5 built in; `sqlite-vec` 0.1.9 compiled in and registered for every connection, a transaction's private one included. Row results converted to Arrow batches (small result sets by design — OLTP). |
| `HostEngine` | wasm-bindgen bridge | The JS host implements the same contract; Arrow IPC over the boundary. |

Both native engines are `feature`-gated (`engine-duckdb`, `engine-sqlite`) so a node can be built with only what it needs. DuckDB `bundled` compile time (minutes) and binary size (~tens of MB) are accepted for Full/Edge nodes.

### 4.3 Format ↔ Engine Compatibility

| Location format | DuckDB | SQLite | DataFusion (coordinator) |
|-----------------|--------|--------|--------------------------|
| `IcebergParquet` | ✅ read/write (write via Oikome Parquet path, not engine) | ❌ | reads only *Arrow streams*, never files (by policy) |
| `SqliteFile` | ✅ read-only (`sqlite_scanner`) | ✅ read/write + transactions | same |
| `Lance` | ✅ read/write, vector and full-text search (`lance` extension) | ❌ | same |

### 4.4 Statement Routing (Semantic Layer → Engine)

Every statement is classified before planning:

| Class | Target Space kind | Route |
|-------|-------------------|-------|
| DDL on Space (`CREATE SPACE`, `ALTER SPACE`, `GRANT`) | — | Oikome catalog, never an engine |
| DDL on table (`CREATE TABLE`) | analytical | Iceberg schema commit (Oikome) |
| DDL on table | transactional | SQLite engine `execute` + catalog mirror |
| DDL on table (`CREATE/ALTER/DROP TABLE`, `CREATE INDEX ... USING <type>`, `DROP INDEX ... ON`) | analytical on a `Lance` Location | `DuckDbEngine` through the `lance` extension; index statements are rewritten to name the dataset path |
| DML (`INSERT/UPDATE/DELETE`) | transactional | **Directly** to `SqliteEngine`, bypassing DataFusion |
| DML (`INSERT`) | analytical | AppendBuffer → Parquet → Iceberg commit (v2 write path) |
| DML (`INSERT/UPDATE/DELETE`, `MERGE`) | analytical on a `Lance` Location | `DuckDbEngine` through the `lance` extension; each statement commits one Lance version |
| TCL (`BEGIN/COMMIT/ROLLBACK`) | transactional | `SqliteEngine::begin` — session holds the `TxHandle` |
| Query (`SELECT`) touching **one** local Space | any | **Pushed entirely** to that Space's engine; DataFusion is a pass-through |
| Query touching **N** Spaces / remote Spaces queried over Flight | any | Federation Layer (§5) |
| Query calling a search function (`lance_vector_search`, `lance_fts`, `lance_hybrid_search`) | one Lance Space or a copy of one; the node's owner only | **Pushed entirely** to DuckDB, the table argument replaced by its dataset path (§7.6) |
| `CREATE VIRTUAL TABLE ... USING fts5(...)` / `vec0(...)` | transactional; the node's owner only | `SqliteEngine`, the module arguments as written |
| Statement using `MATCH` (FTS5 full text, sqlite-vec nearest neighbours) | one transactional Space; the node's owner only | **Pushed entirely** to `SqliteEngine` |
| `CREATE` / `DROP PROPERTY GRAPH`, query using `GRAPH_TABLE` | one Lance Space; the node's owner only | Catalog (definition) and `DuckDbEngine` through `duckpgq` |

A Space bound `ACCESS = 'readonly'` refuses every DDL and DML statement.

Each transactional Space has one shared SQLite connection for autocommit statements. `BEGIN` gives the session a **private connection** to the Space's file, and until `COMMIT`/`ROLLBACK` every statement of that session on that Space, including federated queries, runs on it, so the session sees its own uncommitted rows and nobody else does. A writer on another connection waits up to 5 s for the write lock (`busy_timeout`); dropping a session with an open transaction rolls it back.

---

## 5. Federation Layer (L3)

### 5.1 Role of DataFusion

DataFusion is the **coordinator**, not the engine:

- It never reads Parquet or SQLite files directly. Every leaf of the plan is a `TableProvider` whose `scan` returns Arrow batches produced by an engine or a remote node.
- Its optimizer is used for one thing that matters: `datafusion-federation` groups the largest possible subtree per source and hands it to the source as *SQL in the source's dialect*. So a two-Space join becomes two pushed-down SQL statements plus one DataFusion `HashJoin` over two Arrow streams.
- Its execution runs only the residual operators (cross-source joins, final aggregates, ordering, limits).

```
SELECT o.customer, sum(o.amount), b.title
FROM   orders.orders o                     -- transactional Space, SQLite
JOIN   blog.posts  b ON b.author = o.customer  -- analytical Space, DuckDB
JOIN   space://did:x/pub.metrics m ON ...  -- remote Space, Arrow Flight
GROUP BY ...

DataFusion plan (after federation optimizer):
  Aggregate
   └─ HashJoin
       ├─ HashJoin
       │   ├─ SqliteScan  "SELECT customer, amount FROM orders WHERE ..."   ← SqliteEngine
       │   └─ DuckDbScan  "SELECT author, title FROM read_parquet([...])"   ← DuckDbEngine
       └─ FlightScan     "SELECT ... FROM metrics WHERE ..."                 ← remote node's engine
```

### 5.2 Providers

| Provider | Source | Backed by |
|----------|--------|-----------|
| `EngineSqlExecutor` + `OikomeRemoteTable` | local Space | `datafusion-federation` `SQLFederationProvider` over our `SQLExecutor`, which runs pushed-down SQL on the Space's `QueryEngine`. For analytical Spaces Oikome first resolves the pinned Iceberg snapshot to a file list and attaches it as a DuckDB view. `compute_context` = engine instance id, so all analytical Spaces (one shared DuckDB) federate as one source. |
| `RemoteTableProvider` | remote Space (query shipping) | Arrow Flight `DoGet` with UCAN in headers; implements `SQLExecutor` from `datafusion-federation` so subtrees push down to the remote node. |
| Replica binding | subscribed Space (data shipping) | The owner's snapshot files, copied to `.oikome/replicas/<space>/` (§5.3), attached to the local DuckDB like any analytical Space. |

### 5.3 Query Shipping vs Data Shipping

Decided per remote Space by the UCAN held:

| UCAN capability | Strategy | Behaviour |
|-----------------|----------|-----------|
| `space/query` | Query shipping | Sub-SQL sent to the owner node; only result rows return. Owner applies its own policies. |
| `space/read` | Data shipping | Snapshot files copied to this node (subscription); executed locally. Offline-capable. |
| both | Chosen per Space | `MODE = 'query'` (default) or `'subscribe'` at `CREATE SPACE`. An automatic choice by the planner is not implemented. |

**Subscription** (3.1b):

```sql
CREATE SPACE copy WITH PEER = 'https://a:7070', REMOTE = 'blog', TOKEN = '<ucan>', MODE = 'subscribe';
REFRESH SPACE copy;   -- one row per table: table, snapshot_id, files_fetched, bytes_fetched, status
```

- `CREATE SPACE` checks that the token grants this node `space/read`, then takes the first copy; if that fails nothing is registered. `REFRESH SPACE` (or `spawn_refresher` on a schedule) copies newer snapshots.
- The owner serves two Flight actions (§9): `oikome.snapshot` lists each table's current snapshot with its schema and files, and `oikome.fetch_file` streams one file. Files are named by their BLAKE3 hash, never by a path on the owner.
- **Only tables the subscriber may read in full are copied.** Files cannot be filtered, so a table under a column grant or a row policy is reported as `restricted` and stays query-only. To share part of a table, publish that part to its own Space and grant it. Transactional Spaces are copied through their publication (§7.3).
- The subscriber downloads only files it does not hold, checks each against the announced size and hash, and swaps `manifest.json` with one rename; a failed refresh leaves the previous copy queryable. Files of the copy before the previous one are deleted, so queries started before a refresh can finish.
- A copy is a remote Space: read-only, never granted or served onwards. `DROP SPACE` deletes its files.
- **Revocation stops refreshes; files already copied stay on the subscriber.** Data shipping cannot take data back.

### 5.4 Snapshot Pinning

A federated query pins one Iceberg snapshot ID per Iceberg table at planning time and threads it through every provider, so all subtrees observe a consistent point in time. Transactional Spaces have no snapshot; they observe the latest committed WAL state at scan start. Lance Spaces behave the same way: the `lance` extension (DuckDB 1.5.6) cannot read a given version, so a scan reads the latest version at its start. Subscriptions are still version-consistent (§9).

---

## 6. Semantic Layer (L4)

This is where Oikome's "semantic" claim is cashed out. Everything here is a **SQL → SQL rewrite** over the `sqlparser` AST (`oikome-semantic::rewrite`), before an engine or the federation planner sees the query. Both rewrites replace a relation reference `<space>.<rel> [AS a]` with a derived table `(<query>) AS a`, so whatever they add ends up *inside* the SQL an engine executes.

### 6.1 Statements

```sql
CREATE [OR REPLACE] VIEW blog.recent AS SELECT id, title FROM blog.posts WHERE NOT draft;
DROP VIEW [IF EXISTS] blog.recent;

GRANT SELECT [(id, title)] ON blog.posts TO alice, "did:key:z6Mk...";
REVOKE SELECT ON blog.posts FROM alice;
GRANT SELECT ON SPACE blog TO "did:key:z6Mk...";
REVOKE SELECT ON SPACE blog FROM "did:key:z6Mk...";

CREATE POLICY own ON orders.orders [FOR SELECT] TO alice, bob USING (customer = current_user);
DROP POLICY [IF EXISTS] own ON orders.orders;
```

Views, grants and policies live in the node registry (`.oikome/views.json`, `grants.json`, `space_grants.json`, `policies.json`) and are removed with their Space (and grants/policies with their view).

A **whole-Space grant** (`ON SPACE`, M1b) covers every table of the Space, including tables created later, with all columns; row policies still apply. It covers a view only when the view's expansion reads nothing outside the Space: views run with the owner's rights and may read other Spaces, while a peer's UCAN is checked against the Spaces it names, so sharing one Space must not open another through a view. Whole-Space grants are kept apart from relation grants, so no relation name can stand for the whole Space. Grants add up: under a whole-Space grant, a column grant on one of its tables no longer narrows it. Every definition is validated by running it with `LIMIT 0` before it is stored, so a view, a grant's column list or a policy predicate that references something missing is rejected up front; a view that would expand into itself is rejected as recursive.

### 6.2 Principals

`SessionConfig::principal` says who a session acts for. `None` is the node owner: full access, views inlined, no policies. `Some(p)` is a peer (a DID once 3.1 lands): **read-only** (any DDL, DML, TCL, GRANT or Space statement is `PermissionDenied`) and **deny-by-default**.

| Feature | Mechanism |
|---------|-----------|
| Unified namespace | `<space>.<table>`, or bare `<table>` qualified with the session's `SET SPACE`. CTE names are left alone. `space://did/...` addressing arrives with 3.1. |
| Views | Inlined at rewrite time, nested up to 16 levels. May span Spaces (then federated). They are the way to build a semantic model over several physical sources. |
| Column grants | Each referenced relation becomes `(SELECT <granted columns> FROM …)`, so `*` expands to granted columns only and any other column simply does not exist. |
| Row policies | Once a relation has any policy, a principal sees rows matching at least one policy naming it (`OR`, as PostgreSQL permissive policies), none if no policy names it. `current_user` is replaced by the principal. The predicate is inside the derived table, so it runs in the engine; `explain()` shows it in the pushed-down SQL. |
| View rights | Access control runs first, view inlining second: a principal needs a grant on the view it names, and the view body reads its base tables with the owner's rights (PostgreSQL's default). Granting a view does not expose the tables behind it. |
| Enumeration resistance | An ungranted relation, a missing relation and a relation in a missing Space all fail with the same `Table not found: <space>.<rel>`; `SET SPACE` fails with `Space not found` for a Space the principal has no grant in. |
| Dialect normalisation | User SQL is parsed with a generic dialect; each pushed-down subtree is re-emitted in the target engine's dialect by `datafusion-federation`'s unparser. |
| Statement classification | §4.4, plus `ViewDdl` and `Acl` classes for the statements above. |

Not yet: writes by principals (UCAN write delegation, 3.1), column-level `REVOKE`, `RESTRICTIVE` policies, policies on commands other than `SELECT`, view column lists and materialized views.

---

## 7. Catalog & Storage (L1)

### 7.1 Analytical Spaces (unchanged core)

Iceberg metadata + Parquet data files via VFS. Write path: `AppendBuffer → Parquet flush → Iceberg fast-append commit` (`oikome-storage`). iceberg-rust 0.10 has no public overwrite action, so `OikomeCatalog::replace_data_files` produces `overwrite` snapshots itself (manifest + manifest list + optimistic metadata commit, retried on conflict); compaction and publishing both use it, so compaction no longer duplicates rows. **Read path**: the snapshot's data-file list is handed to DuckDB, not read by DataFusion `ParquetExec`.

### 7.2 Transactional Spaces (new)

- One SQLite database file per Space, WAL mode, at `Location.path`.
- Writes go straight to SQLite; Oikome does not intercept or log them.
- Table names come from the engine (`sqlite_master`). `CREATE/ALTER/DROP TABLE` pass through to SQLite.

### 7.3 Publishing: SQLite → Parquet snapshots

Transactional Spaces become visible to peers only through **published snapshots**:

```sql
PUBLISH SPACE orders [TABLES (orders, items)] [TO orders_published]
```

- The target defaults to `<space>_published`. It is created on first publish as an ordinary analytical Space marked `published_from = <space>`; only `PUBLISH SPACE` writes it (DML/DDL on it fail with `PublishedSpace`), while views, grants and policies on it work as on any Space. It participates in federation (and later P2P sync) like any other analytical Space.
- Every selected table is read inside **one read transaction on a private connection**: the publication is a consistent committed state of all tables, never includes uncommitted writes, and does not block or disturb open transactions.
- Each table gets **one new Iceberg `overwrite` snapshot** replacing its data (`OikomeCatalog::replace_data_files`), so the published table is an exact copy as of the publish, and every earlier publication remains readable at its snapshot. Commits are atomic per table, not across tables.
- A full publish (no `TABLES`) mirrors the source: tables dropped in SQLite are dropped from the target. A table whose SQLite schema changed is recreated with the new schema (its earlier snapshots are lost with it).
- `spawn_publisher(session, request, every)` republishes on an interval; failures are logged and retried at the next tick. Commit-count triggers and WAL-level replication (Litestream-style) are **out of scope for v3.0**.

### 7.4 VFS

`oikome-vfs` holds the opendal VFS. Since 3.4a it also has `RemoteStore`: the folder of one Space on a storage (§7.5), over WebDAV or as a mounted folder. Every file is written under a temporary name, checked for size and renamed into place, so a reader sees a file whole or not at all. opendal sends a WebDAV file as a single request body, so uploads are streamed with a plain PUT instead.

DuckDB and SQLite need real file paths, and DuckDB reads an Iceberg table as a list of local Parquet files. A Space on a storage is therefore never read in place: each node keeps a local copy of a whole snapshot (§7.5). The per-file `materialize()` cache planned earlier was dropped; a size-bounded cache only helps once an engine reads part of a table. The Iceberg `Storage` adapter in `oikome-vfs` is still unused.

### 7.5 Storages: a NAS First, Cloud Drives Step by Step

A storage is a place other than the node's disk where a Space is kept: a NAS at home (3.4a), reached over WebDAV (Synology, QNAP, Nextcloud and most others) or mounted as a folder (SMB, NFS), and cloud drives through the folder their desktop client syncs (3.4b: iCloud Drive, Dropbox, OneDrive, Google Drive). A drive provider can read whatever is uploaded, so a drive is always encrypted on the client; a NAS may be. Drives over their API follow in 3.4c.

```sh
oikome storage add nas --webdav https://nas.local:5006/oikome --username me --password-stdin
oikome storage add share --folder /Volumes/oikome  # a mounted share; must exist
oikome storage add icloud --drive "~/Library/Mobile Documents/com~apple~CloudDocs/Oikome"  # makes a key
oikome storage export-key icloud                   # on this node; then on another node:
oikome storage add icloud --drive "<its sync folder>" --key-stdin
```

```sql
-- node A writes the Space
CREATE SPACE photos WITH STORAGE = 'nas';                 -- folder = Space name; PATH = 'family/photos' chooses another
INSERT INTO photos.items ...;                             -- each write is pushed to the NAS
-- node B copies it
CREATE SPACE photos WITH STORAGE = 'nas', ACCESS = 'readonly';
REFRESH SPACE photos;                                     -- `oikome serve` also syncs every 60 s (--storage-sync)
```

**How it works.** The writing node keeps the Space on its own disk as before. After each write it pushes the new snapshot: the files the storage lacks first, then a snapshot file naming them. Other nodes copy the newest complete snapshot into a local copy (`.oikome/replicas/<location>/`, Location format `StorageCopy`) and read it like a subscription (§5.3), also while the writer is offline. Iceberg and Lance Spaces work the same way; a Lance copy is searched with the writer's indexes.

```
<storage>/<PATH>/
  oikome-space.json          layout version, writer's DID, Location id, format
  snapshots/<seq>.json       one snapshot: per table its schema, snapshot id and files (BLAKE3, size, Lance path)
  files/<blake3>.parquet     Iceberg data files
  lance/<table>.lance/...    Lance data, deletion, index and version files
```

**What may live on a storage**

| Data | On a storage? | Why |
|------|---------------|-----|
| Analytical Space (Iceberg + Parquet) | Yes, written by one node | Parquet data files are immutable |
| Analytical Space (Lance, §7.6) | Yes, written by one node | Lance files are immutable too; each version adds a manifest |
| Transactional Space (SQLite file) | **No** — rejected at `CREATE SPACE` | SQLite's WAL needs file locks and shared memory on a local filesystem |
| Node registry (`.oikome/`), identity key, storage credentials and keys | No | They belong to one node |

**Rules**

- **One writing node per folder.** The writer's DID is in `oikome-space.json`; a writable binding needs an empty folder or one this node already writes for the same Location. This guards against mistakes, not attacks: anyone the storage lets in can change its files, so access is controlled by the storage's own accounts. WebDAV cannot create a file only if it is absent, so the claim is read back.
- **Readers take the newest complete snapshot.** A snapshot whose files are not all present (pruned while being copied, or never finished) is passed over for the one before it. Downloads are checked against size and BLAKE3; the local copy changes with one rename.
- **The writer keeps two snapshots** and the files they name, so a reader copying the older one can finish; it removes everything else, including leftovers of interrupted uploads. A push uploads whatever the storage lacks, so a removed file is put back.
- **A failed push does not fail the write.** The write is committed on the writer's disk; the next write, `REFRESH SPACE` or the serving node's sync pushes it. Failing the statement would invite a retry that duplicates rows.
- **The node keeps the credentials.** People enter a storage's password, token or key once (in the apps, or on standard input to `oikome storage add`), and the node stores them in the OS keychain (macOS, Windows), or where there is none (Linux servers, containers) in `.oikome/identity/storage_access.json`, readable by the node's user only, like the node key beside it. `OIKOME_STORAGE_<NAME>_PASSWORD`, `_TOKEN` and `_KEY` override both, for deployments. Credentials never enter the storage registry, SQL or logs. Plaintext `http://` needs `--allow-http`.

**Encryption.** An encrypted storage has one random 256-bit key. `oikome-vfs::seal` seals every file in 64 KiB segments with XChaCha20-Poly1305 (STREAM construction, the file's name as associated data), so a file cannot be read, changed, cut short or swapped for another without the key. Names come from a keyed hash of the file's plain path, so the storage sees neither table names nor data hashes; only the number of snapshots, of files and their rough sizes show:

```
<storage>/<PATH>/
  oikome-space.sealed        the record, sealed
  snapshots/<seq>.sealed     snapshots, sealed
  objects/<id>               data, index and version files, sealed; id = keyed BLAKE3 of the plain path
```

Each file's header carries a key id, so the wrong key is reported as such; a folder registered without encryption, or the other way round, is refused with a hint. Another node reads an encrypted storage with the key exported on the node that made it (`oikome storage export-key`; in the apps, "use on another device"). If every copy of the key is lost, the files on the storage cannot be read; the writer still holds the Space on its own disk.

**Drives through their sync folder.** `--drive` marks a folder the drive's desktop client syncs and always encrypts it; a `--folder` inside a known sync location (`~/Library/Mobile Documents`, `~/Library/CloudStorage`, `~/Dropbox`, `~/OneDrive*`, `~/Google Drive`, …) is refused without encryption. Sync clients may upload files out of order or create conflict copies; readers take the newest complete snapshot and ignore files no snapshot names, and a snapshot whose files have not arrived yet waits for the next sync. iCloud may leave files as placeholders, which macOS downloads when they are read.

Not done yet: drives over their API (3.4c: OAuth sign-in, opendal `gdrive`, `dropbox`, `onedrive`), the keychain on Linux desktops, changing a storage's key, a size-bounded cache (every node holds the current snapshot of each Space it reads in full), restoring a writer from its storage, and `PUBLISH` into a Space on a storage.

### 7.6 Lance Locations

Lance is a columnar format with versions, built for vector search, full-text search and large objects. A Lance Location holds one Lance dataset per table. It is for data that is searched or read row by row rather than aggregated: note text and embeddings, documents, images, audio, attachments.

```sql
CREATE SPACE notes WITH FORMAT = 'lance';     -- FORMAT defaults to 'iceberg' for analytical Spaces
CREATE TABLE notes.items (id BIGINT, body VARCHAR, embedding FLOAT[768]);
INSERT INTO notes.items ...;                   -- also UPDATE, DELETE, MERGE; each statement is a version
CREATE INDEX emb ON notes.items USING IVF_PQ (embedding) WITH (num_partitions = 64, metric_type = 'cosine');
CREATE INDEX body ON notes.items USING INVERTED (body);
SELECT id, _distance FROM lance_vector_search(notes.items, 'embedding', [...]::FLOAT[768], k => 10);
SELECT id, _score FROM lance_fts(notes.items, 'body', 'apple', k => 10);
SELECT id, _hybrid_score FROM lance_hybrid_search(notes.items, 'embedding', [...]::FLOAT[768], 'body', 'apple', k => 10);
```

| | Iceberg + Parquet | Lance |
|---|---|---|
| Used for | Tables that are scanned and aggregated | Data that is searched (vector, full-text) or read row by row (large objects) |
| Versions | Iceberg snapshots | Lance versions; each commit writes a new manifest |
| Writes | Append; overwrite via `replace_data_files` (§7.1) | Append, update, delete, merge; each commit is a new version |
| Indexes | None stored; extension indexes are engine-local | Vector and full-text indexes are part of the dataset |
| Read by | Almost every engine and tool | DuckDB `lance` extension, LanceDB, Lance Python / Rust |

Rules:

- **DuckDB does the work.** The Location (`.oikome/lance/<location>/`, one `<table>.lance` per table) is attached to DuckDB as a catalog named after the Space, so `<space>.<table>` is read and written directly. Reads, writes, index builds and searches go through DuckDB's `lance` extension (core extension of DuckDB 1.5.6, built on lance 12). Oikome only rewrites what names a dataset: an index statement's table and a search function's first argument become the dataset path.
- **Search is for the node's owner.** A search function reads the dataset itself, past column grants and row policies, so a peer cannot call it over query shipping (it still queries the table's columns under its grants). A peer that may read a table in full subscribes to it and searches the copy. A search runs in a query over one Space.
- **Versions.** Every statement commits one Lance version. Reads see the latest version (§5.4). Subscription ships exactly one version (§9): Oikome reads that version's manifest with `lance-table` 12 (the extension cannot list a version's files) and ships the manifest, data, deletion and index files it references, so rows deleted before that version never leave the node.
- **Indexes travel with the data.** A subscriber searches its copy with the owner's indexes and never builds one.
- **Single writer** as for any Location (§3.2); a second Space bound `ACCESS = 'readonly'` reads it and refuses writes.
- **Storages:** the same rules as Iceberg Locations (§7.5); a copy is searched with the writer's indexes.
- **Not in the browser.** The `lance` extension is published for linux (amd64, arm64), macOS arm64 and windows amd64, not WebAssembly; browser nodes reach Lance Spaces through a Full Node (§10).

Not done yet: reading a pinned version (waits for the extension), `PUBLISH` (§7.3) into a Lance Space, peers searching over query shipping, and a search spanning several Spaces.

---

## 8. Multimodal via Engine Extensions

| Modality | DuckDB (analytical) | SQLite (transactional) | Oikome responsibility |
|----------|---------------------|------------------------|------------------------|
| VECTOR | Lance Location (§7.6): `lance_vector_search`, index stored in the dataset | `sqlite-vec` | Capability check; route to the one engine that runs it |
| FTS | Lance Location: `lance_fts`, `lance_hybrid_search` (vector + text) | FTS5 | same |
| GRAPH | DuckPGQ (SQL/PGQ `GRAPH_TABLE ... MATCH`) on Lance Spaces | — | Keep graph definitions; capability check; Cypher is **dropped** |
| BLOB | Lance Location for files kept in the data center (images, audio, attachments); CID columns + gateway dereference for external content | CID columns + gateway dereference | CID: unchanged from v1 |

Indexes inside a Lance dataset are part of its versions and **travel with it**, so a subscribing node searches without rebuilding. Indexes built by other extensions are engine-local and **not synchronized**; a subscribing node rebuilds them.

In a transactional Space:

```sql
CREATE VIRTUAL TABLE shop.docs USING fts5(title, body, content = 'notes', content_rowid = 'id', tokenize = 'porter');
SELECT rowid, title FROM shop.docs WHERE docs MATCH 'apple' ORDER BY bm25(docs);
CREATE VIRTUAL TABLE shop.vecs USING vec0(embedding float[768]);
SELECT rowid, distance FROM shop.vecs WHERE embedding MATCH '[...]' AND k = 10 ORDER BY distance;
```

FTS5 and vec0 tables are listed as relations of the Space; the tables SQLite keeps behind them are not. `PUBLISH` (§7.3) skips them: they index other tables, which are published. `MATCH`, like the Lance search functions, is for the node's owner within one Space. sqlparser's generic dialect has no `MATCH` operator, so a statement using it is parsed with the SQLite dialect.

Graphs, on a Lance Space (DuckPGQ cannot define a graph over views, and Iceberg tables reach DuckDB as views):

```sql
CREATE PROPERTY GRAPH social.friends
    VERTEX TABLES (people LABEL person)
    EDGE TABLES (knows SOURCE KEY (src) REFERENCES people (id)
                       DESTINATION KEY (dst) REFERENCES people (id) LABEL knows);
FROM GRAPH_TABLE (social.friends
    MATCH p = ANY SHORTEST (a:person WHERE a.name = 'alice')-[k:knows]->+(b:person WHERE b.name = 'dave')
    COLUMNS (path_length(p) AS hops));
DROP PROPERTY GRAPH social.friends;
```

A graph's tables belong to its Space. sqlparser cannot read these statements; Oikome tokenizes them, qualifies the table names and names the graph `<space>__<graph>` in DuckDB. DuckPGQ keeps graphs in the DuckDB database, which lives in memory, so the catalog keeps the definitions (`graphs.json`) and a DuckDB that lacks one creates it before a graph query. Graph statements are for the node's owner and read one Space. Graph algorithm functions (`pagerank`, ...) are not available yet.

A Space does not declare the extensions it needs. They follow from its format and from the statement: a Lance Location needs `lance` (`Capabilities::READ_LANCE`), a search function needs it on the Space's engine; `MATCH` needs FTS5 and sqlite-vec (`EXT_FTS`, `EXT_VECTOR`), which every SQLite engine has; a graph needs `duckpgq` (`EXT_GRAPH`). `QueryEngine::enable` loads what a capability takes. The check happens before the statement runs; a node without the extension answers with how to install it.

---

## 9. Identity & Network (L0)

Unchanged in intent from v1 §8: DID (Ed25519), UCAN (issue/verify/revoke/CRL), peer state machine, libp2p discovery, Arrow Flight RPC. Two clarifications for v3:

- Arrow Flight `DoGet` carries **SQL in the receiver's semantic namespace**; the receiver runs it through its own semantic layer and engines. A node never executes SQL it did not rewrite itself.
- Snapshot sync exchanges only `IcebergParquet` and `Lance` Locations. SQLite files never leave a node. For a Lance table, `oikome.snapshot` lists one version's files with their path in the dataset; the subscriber checks each path (only `data/`, `_versions/`, `_deletions/`, `_indices/<uuid>/`, plain names), downloads the version's manifest last so a reader sees the old version or the whole new one, and keeps the files of the current and previous version.

The peer protocol over Arrow Flight (`oikome-network::protocol`). Every call carries a UCAN and a request signed for the receiving node over the call's method, payload and UCAN; each signed request is accepted once.

| Call | Ability | Purpose |
|------|---------|---------|
| `DoAction("oikome.list_tables")` | `space/query` | Relations of a Space the caller may read |
| `GetSchema` `[space, table]` | `space/query` | Schema with the granted columns only |
| `DoGet` (SQL ticket) | `space/query` | Run a query through the owner's semantic layer (query shipping) |
| `DoAction("oikome.snapshot")` | `space/read` | Current snapshot of the tables the caller may copy in full: schema, files by hash and size (subscription) |
| `DoAction("oikome.fetch_file")` | `space/read` | One file of a snapshot, by hash, in 1 MiB chunks |

`space/read` includes `space/query`.

**Transport (3.1c).** Nodes speak TLS by default. Each node presents a self-signed certificate for its own Ed25519 key, generated at start (`NodeIdentity::tls_identity`, rcgen); nothing is stored. A client knows the DID it expects, the root issuer of the token it presents, and accepts the server only if the certificate carries that DID's key. rustls (ring backend) checks that the handshake is signed with it. There is no CA and no host-name check. Plaintext `http://` is refused unless the node runs with `--allow-http` (`NodeConfig::allow_http`); `serve --plaintext` needs it too. Requests are signed either way.

**Discovery (3.1c, `oikome-discovery`).** libp2p is used only to find nodes; data stays on Arrow Flight.

- The libp2p identity is the DID key, so a `PeerId` and a DID name the same public key. The swarm runs tcp, noise, yamux, mDNS, identify and `/oikome/endpoint/1`.
- After connecting, a node asks the other for its Flight scheme and port. The host is the address the connection came from. The result, `DID → scheme://ip:port`, goes into the Peers table (§3.4).
- mDNS finds nodes on the local network; `serve --dial <multiaddr>` connects to a known one.
- `CREATE SPACE ... WITH TOKEN = '...'` without `PEER` resolves the owner (the token's root issuer) in the Peers table, again on every connection, so an owner that moved is followed.
- Crossing NATs (relay, DHT) is not covered.
- mDNS needs multicast, and on macOS the application firewall must let `oikome` receive it.

**Running nodes (3.1d).** `docker/oikome.Dockerfile` builds the node image: the `oikome` binary and the prebuilt libduckdb, with the node's data in `/data`. `docker/compose.yml` runs three nodes on one bridge network:
- `node-a` owns the data;
- `node-b` reaches it after dialing `node-a`'s libp2p address;
- `node-c` is given no address and has to find `node-a` by mDNS.

`tests/e2e/docker_nodes.sh` drives them, checking each step against its expected result: discovery, a UCAN-gated federated query over TLS with no `PEER`, a subscription, then revocation stopping both the query and refreshes while the copy stays readable. `.github/workflows/docker-e2e.yml` runs it on GitHub Actions on every push to `feat/**` or `ci/**`.

**Access log (M1b, `oikome-node::access_log`).** A serving node records every request a peer or a browser sends it, so the owner can see who read what and when.
- One row per call of the six entry points (`get_schema`, `do_get`, the three actions, `serve_envelope`): time, caller DID, method, Spaces, the id of the UCAN that let it in, outcome (`allowed`, `denied`, `failed`) and the reason. The reason is never sent to the caller.
- `fetch_file` is recorded only when refused; the subscription is recorded by its `oikome.snapshot` call.
- Rows live in `.oikome/access_log.sqlite`. Requests never wait for the log: a writer thread takes entries from a bounded channel (10,000), and entries are dropped (and counted) when it falls behind.
- Failed authentications carry no caller and can come from anyone who reaches the port, so they are merged into one row per method and minute, under a cap of their own (10,000 rows). The log keeps 90 days, at most 100,000 rows.
- The owner reads it through `list_access` (§13.4).

**Registry writes across processes (M1b).** `serve` and `oikome sql` are separate processes over the same `.oikome/`. A writer holds an exclusive lock on `.oikome/registry.lock`, reads the registry files again, changes them and saves them; `refresh` holds the lock shared while it reads. Without this, a write from one process could save a copy loaded before the other process's write and undo it, for example bringing back a token revoked with `oikome sql`.

---

## 10. Browser Node

```
 JS host (packages/node)              WASM (crates/oikome-wasm)                Full Node
 ┌──────────────────────────┐  Arrow  ┌───────────────────────────────┐
 │ duckdb-wasm (worker,     │◄───IPC──│ HostEngine                    │
 │   database in OPFS)      │         │ BrowserNode: identity · Space │
 │ IndexedDB (seed,registry)│◄─bytes──│   registry · semantic layer   │
 │ transport.js             │◄─envel.─│ RemoteEngine                  │   ┌──────────────────┐
 │   RTCPeerConnection ─────┼─────────┼── WebRTC data channel ────────┼──►│ oikome-webrtc    │
 └──────────────────────────┘         │ federation coordinator        │   │ (str0m, UDP) →   │
                                      │   (single task)               │   │ OikomeFlight-    │
                                      └───────────────────────────────┘   │ Service          │
                                                                          └──────────────────┘
```

**3.2a (done): local queries.**
- **Rust core.** `oikome-wasm` compiles the identity, statement parsing and semantic layer to WebAssembly.
- **Engine.** The JavaScript host provides duckdb-wasm, running in a worker with its database file in OPFS.
- **Spaces.** Each browser Space is a schema of that database (`EngineKind::Host`). Transactional Spaces, views and grants are not available in the browser yet: there is one owner and no peers to serve.
- **How `HostEngine` works** (`host.rs`). It implements `QueryEngine` while holding only an id. `QueryEngine` and DataFusion need `Send` and JavaScript objects are not, so:
  - the host object lives in a thread-local table;
  - each call starts with `spawn_local`;
  - its result returns over a oneshot channel;
  - results cross as Arrow IPC streams.
- **Persistence.** The host keeps the node's seed and its Space registry as bytes in IndexedDB. A same-origin script can read the seed; a non-extractable WebCrypto Ed25519 key is the planned replacement.
- **JavaScript API.** `OikomeNode.open(engine, store, transport)` (the transport arrived in 3.2b), `node.did()`, `node.sql(text)` resolving to `{ ipc }` or `{ rowsAffected }`.
- **Build** (`npm run build` at the repository root). `packages/node/scripts/build.sh` runs cargo for `wasm32-unknown-unknown`, then `wasm-bindgen` (pinned to the crate's version); `e2e` then bundles the test page with esbuild.
  - DataFusion 54 turns on Arrow's zstd unconditionally, so a clang with a wasm32 backend is needed: Homebrew LLVM on macOS, the distribution clang on Linux.
  - `datafusion-federation` 0.5.5 is patched (`third_party/`) so DataFusion's C-backed `compression` and `parquet` features stay off.
- **Tests.** `e2e/` (Playwright, headless Chromium) runs local queries and checks that data, identity and registry survive a reload. `browser-e2e` runs them in CI.

**3.2b (done): federated queries via a Full Node over a WebRTC data channel.**
- **Full Node side** (`oikome-webrtc`; `oikome serve --webrtc-listen <udp> [--webrtc-advertise <ip>]`).
  - The server is str0m 0.24 with its pure-Rust crypto (DTLS by dimpl, SCTP by sctp-proto), as an ICE-lite endpoint.
  - The browser dials "WebRTC-direct": no signaling server. The server names each session by the ICE username fragment in the browser's first STUN request; the fragment doubles as the ICE password.
  - The DTLS certificate (ECDSA P-256) is created once and kept in `.oikome/identity/webrtc_dtls.json` (owner-only), so its fingerprint does not change between runs.
  - The server does not check the browser's DTLS certificate. Each request is authenticated by the browser's DID signature instead, as a native peer's is.
- **Access record.** `SHOW WEBRTC` prints `webrtc:<base64url JSON>.<signature>`.
  - The JSON is `{did, addr, fingerprint, expires_at}`, signed with the node's DID key under its own domain string, valid for 30 days.
  - `serve` writes it at startup and removes it at shutdown; it can be copied or shown as a QR code.
- **Mounting.** `CREATE SPACE pub WITH PEER = 'webrtc:<record>', REMOTE = 'blog', TOKEN = '<ucan>'`. The browser checks that:
  - the record's signature matches the DID it names, and it has not expired;
  - the token's root issuer is that DID;
  - the token was issued (or delegated) to the browser's DID and grants `space/query` on `blog`.
  
  The registry in IndexedDB keeps the record and the token, as it keeps the seed. Remote Spaces are read-only and queried in place; `MODE = 'subscribe'` is not available in the browser.
- **Connection** (`packages/node/src/transport.js`). The browser builds the node's SDP answer from the record: ICE-lite, passive DTLS, the record's fingerprint, one negotiated data channel. A certificate with another fingerprint fails the DTLS handshake. One channel per node is opened on first use and reused.
- **Requests.** Each call is an envelope `{method, payload, ucan, auth}`, the same fields as an Arrow Flight call.
  - The Full Node serves it with the same authentication (signature, replay window), authorization (UCAN, revocation) and semantic layer (`OikomeFlightService::serve_envelope`).
  - The reply is a status and a body: an Arrow IPC stream for `do_get` and `get_schema`, JSON for `list_tables`.
  - Messages are cut into frames of at most 16 KiB (`[u32 request id][flags][payload]`), so concurrent calls share the channel.
  - Limits: 64 sessions, 60 s idle timeout, 256 MiB per message.
- **Federation in the browser.** A query that names a remote Space runs on the coordinator built with `Coordinator::single_task`:
  - DataFusion runs partitions in spawned Tokio tasks, and the browser has no Tokio runtime. The coordinator plans with one target partition, and `MergePartitionsExec` replaces `CoalescePartitionsExec`: it polls every partition from one task (a `UNION` still has one partition per input). A native test runs such plans without a runtime.
  - DataFusion stamps every plan with `chrono::Utc::now()`, so chrono's `wasmbind` feature is on to read the JavaScript clock.
  - Queries over local Spaces only still go straight to duckdb-wasm.
- **Size.** 32.7 MB with the `wasm` profile `packages/node/scripts/build.sh` uses (`opt-level = "s"`, LTO, stripped); 132 MB with the plain release profile. Before the coordinator it was about 9.5 MB. Size is a measurement, not a requirement.
- **Tests.**
  - Native: `oikome-webrtc` dials its server with a native str0m client (a wrong fingerprint never connects); `oikome-server` serves envelopes; `oikome-wasm/tests/remote_spaces.rs` runs the browser node on DuckDB against a Full Node's service.
  - Browser: `e2e/remote.spec.mjs` starts `oikome serve --webrtc-listen` and, over a real WebRTC channel, joins a duckdb-wasm table with the node's table. It also checks that a mount survives a reload, that revocation cuts access, and that a tampered record and a wrong fingerprint are refused.
- **Not covered yet.** The browser must reach the published UDP address directly (same network, or a public address): no STUN or TURN. Browsers do not serve each other.
- **Lance.** Lance Spaces are also reached through a Full Node, since DuckDB's `lance` extension is not published for WebAssembly (§7.6).

---

## 11. Connectors

A private data center is useful only once the data is in it, and most of an owner's data sits inside apps. A connector brings one app's data into a Space the owner holds.

### 11.1 Sources

| Kind | Example | Sync |
|------|---------|------|
| App API | flomo through its MCP server (`https://flomoapp.com/mcp`, personal token; requires flomo MAX) | Incremental, on a schedule |
| Export file | flomo's HTML export (all plans), Apple Health `export.xml`, a bank's CSV statement | On each import; a re-import reconciles |

Connectors use only an app's official API or files the person exports. Reverse-engineered endpoints break without notice and may breach the app's terms.

### 11.2 Contract

```rust
#[async_trait]
pub trait Connector: Send + Sync {
    fn name(&self) -> &str;              // "flomo"
    fn tables(&self) -> Vec<TableDef>;   // output tables: name, Arrow schema, source key columns
    /// Records changed since `cursor`, plus the cursor to store for the next pull.
    async fn pull(&self, cursor: Option<Cursor>) -> Result<Pull>;
}

pub struct Pull {
    pub upserts: Vec<(String, SendableRecordBatchStream)>, // table → new or changed rows
    pub deletes: Vec<(String, Vec<SourceKey>)>,            // when the source reports deletions
    pub cursor: Cursor,                                    // opaque to Oikome
    pub complete: bool,                                    // full listing: rows not in it were deleted
}
```

### 11.3 Where the Data Lands

- Each connector instance writes to its own **transactional** Space, e.g. `flomo.memos`. Sources edit and delete records, so loading is an upsert by source key, which needs SQLite; analytical Spaces are append-only.
- A pull is applied in one SQLite transaction together with the new cursor. A failed pull leaves data and cursor unchanged, and the next run retries.
- Sharing uses the existing path: `PUBLISH SPACE flomo` (§7.3), then grants and policies (§6).
- Reshaping is done with views (§6.1), not inside the connector. Connectors only move data (principle 1).

### 11.4 Running

- A connector runs only on the node that owns its target Space (principle 2), on a schedule like the publisher (§7.3) or on demand from the CLI.
- Credentials are named in the connector config and read from the OS keychain. They never enter Space data, `.oikome/` files, snapshots, or anything sent to a peer.

Open:
- In-process Rust connectors only, or also external programs that write Arrow IPC to stdout. External programs let a connector use the language of each app's SDK, at the cost of a process boundary and a sandboxing question.
- How a connector is configured: a statement (`CREATE CONNECTOR …`) or a config file.

---

## 12. Compute Sharing

Compute comes from personal resources: the owner's own machines first, other individuals' machines when those are not enough. Compute sharing is how a node uses another individual's machine, or lends its own. It uses the same trust mechanism as data exchange, with a separate authorization.

### 12.1 Compute Sharing vs Data Exchange

| | Data exchange | Compute sharing |
|---|---|---|
| Trust | DID identities, peer relationships, deny-by-default, UCAN revocation (§9) | Same |
| Authorization | `space/query`, `space/read`, grants and policies, issued by the data owner (§5.3, §6) | `compute/<runtime>` with limits, issued by the owner of the executing node (§12.2) |
| What moves | Query results or snapshots of the owner's Spaces | A task and its input to the executing node; the output back |
| Who computes | The data owner's node, over its own data | The executing node, over the input it was sent |
| What the other side sees | Only what grants and policies allow | The executing node sees the task input, and none of the requester's Spaces |

The two authorizations never imply each other: a peer I exchange data with cannot run tasks on my machine, and a peer that runs my tasks gains no access to my Spaces. Running SQL on a peer's Space (query shipping, §5.3) is data exchange, even though the peer's node does the computing. A browser node sending heavy queries to a Full Node of the same owner (§10) is the owner using their own resources and needs no compute grant.

### 12.2 Tasks

A task asks another node to run work on one of its runtimes, over input the requester sends.

- **Tasks run only on runtimes the executing node has registered.** SQL runs on its engines, with the input attached as a temporary table (`TableSource::ArrowBatches`, §4.1). Anything else, e.g. embedding or model inference, runs on a runtime the node's host registers, the way the browser host provides `HostEngine` (§4.2). Oikome routes tasks and does not implement them (principle 1). Shipping arbitrary code or binaries is out of scope.
- **Every task carries a UCAN** with a `compute/<runtime>` ability and limits: wall time, memory, input and output bytes, expiry. The executing node checks it before queuing, as it checks `space/query`; revocation drops queued tasks.
- **The executing node's owner sets per-peer quotas** and may refuse or queue any task.
- **The requester chooses the input** for each task: a query result, never a whole Space. The executing node sees it while the task runs and deletion afterwards cannot be verified, so what to send is the requester's decision.
- **Inputs and outputs are Arrow** over Arrow Flight, as for remote queries (§9).

Open:
- Which workloads come first: SQL, embeddings, model inference or training. This decides the runtime contract and scheduling.
- Accounting between peers: plain per-peer quotas, or reciprocal credit.
- Whether a WASM sandbox should later allow user-supplied task code.

---

## 13. Crate Layout & Migration from v2

```
crates/
├── oikome-core            errors, SpaceUri, SpaceKind/EngineKind/AccessMode, CreateSpaceRequest   [done]
├── oikome-vfs             opendal VFS; RemoteStore: a Space's folder on a storage (WebDAV, folder) [3.4a]
├── oikome-catalog         Iceberg Catalog + Space/Location registries (.oikome/*.json) + resolve_table [done]
├── oikome-storage         Parquet write path (keyed by TableIdent), compaction                     [done]
├── oikome-engine          QueryEngine trait, TableSource, Capabilities, BlockingWorker             [done]
├── oikome-engine-duckdb   DuckDbEngine (bundled DuckDB + parquet)                                  [done]
├── oikome-engine-sqlite   SqliteEngine + SQLite rows → Arrow                                       [done]
├── oikome-semantic        CREATE/DROP/SET SPACE parser, classification, view + access-control rewrites [done]
├── oikome-federation      EngineSqlExecutor, OikomeRemoteTable, Coordinator                         [done]
├── oikome-node            OikomeSession, EngineRegistry, routing, DDL/DML                           [done]
├── oikome-identity        DID keys, UCAN issue/verify/revoke, peer FSM                             [3.1]
├── oikome-network         Peer protocol, PeerClient, FlightEngine, DID-pinned TLS; tasks in 3.6    [3.1]
├── oikome-discovery       libp2p discovery (mDNS, endpoint exchange) → Peers table                 [3.1]
├── oikome-server          Arrow Flight service (queries, snapshots, files), oikome binary         [3.1]
├── oikome-protocol        Peer protocol types, request digest, WebRTC record and frames            [3.2]
├── oikome-webrtc          WebRTC endpoint for browsers (str0m): sessions, DTLS certificate         [3.2]
├── oikome-wasm            Browser node: BrowserNode, HostEngine (duckdb-wasm), RemoteEngine, JS API [3.2]
├── oikome-connect         Connector trait, cursor store, built-in connectors (file import, flomo)  [3.5]
├── oikome-api             API for clients, plugins and MCP; caller is Owner or Principal (§13.4)   [M1b]
├── oikome-mcp             MCP adapter: principal operations only; bridges to the running node      [M3]
└── oikome-cli                                                                                      [later]
```

Tags `M1b` and `M3` refer to the apps track (§14.1).

`oikome-query` was deleted; its `CREATE SPACE` parser moved to `oikome-semantic`.

### 13.1 Dependency Changes

The Arrow-line crates must move together: DataFusion 55 and `datafusion-federation` ≥ 0.5.6 require Arrow 59, which DuckDB 1.10506 and Iceberg 0.10 do not support yet. `lance-table` follows the lance release inside DuckDB's `lance` extension, so it moves with `duckdb`.

| Action | Crate | Version | License |
|--------|-------|---------|---------|
| Upgrade | `datafusion` | 51 → **54** | Apache-2.0 |
| Upgrade | `arrow*`, `parquet` | 57 → **58** | Apache-2.0 |
| Upgrade | `iceberg` | 0.9 → **0.10.1** | Apache-2.0 |
| Upgrade | `sqlparser` | 0.59 → **0.62** (same as DataFusion 54) | Apache-2.0 |
| Add | `datafusion-federation` | **=0.5.5** (`sql` feature) | Apache-2.0 |
| Add | `duckdb` | **=1.10506.1** (DuckDB 1.5.6, prebuilt `libduckdb`) | MIT |
| Add | `lance-table`, `lance-io` | **=12.0.0** (Lance manifests only; `protoc` built from source; no cloud stores) | Apache-2.0 |
| Add | `rusqlite` | **0.40** (`bundled`, `column_decltype`) | MIT |
| Add | `sqlite-vec` | **=0.1.9** (compiled with the bundled SQLite) | MIT / Apache-2.0 |
| Feature | `opendal` | 0.52 `services-webdav` (no new crates) | Apache-2.0 |
| Add | `reqwest` | **0.12**, direct (streamed WebDAV uploads; the same version and TLS stack opendal uses) | MIT / Apache-2.0 |
| Add (tests) | `dav-server` | **0.11**, dev-dependency (a WebDAV server standing in for a NAS) | Apache-2.0 |
| Add | `chacha20poly1305` | **0.10** (RustCrypto; already in the tree through libp2p) | Apache-2.0 / MIT |
| Add | `keyring-core`, `apple-native-keyring-store` (macOS), `windows-native-keyring-store` (Windows) | **=1.0.1**, **=1.0.2**, **=1.1.0** | MIT / Apache-2.0 |
| Remove | `datafusion-datasource/-common/-execution/-physical-plan` direct deps, `dashmap`, `memmap2` | — | — |

`datafusion-table-providers` was evaluated and **not** used: it brings its own connection pools (a second path to each engine besides `QueryEngine`) and pins an older `rusqlite`. Licenses are checked by `cargo deny check licenses` (`deny.toml`).

### 13.2 Code to Delete (v2 residue)

- `oikome-core/src/stage.rs`, `operator.rs` — self-built execution plan types; nothing in v3 consumes them.
- `oikome-catalog/src/datafusion_bridge.rs` (`IcebergTableProvider`, `OikomeCatalogProvider`), `datafusion_sink.rs`, `parquet_util.rs` (morsel reader), `scan_provider.rs` — replaced by engine providers.
- `oikome-query/src/session.rs` — replaced by `oikome-federation` + `oikome-semantic`.

Roughly 1,100 of the current ~4,300 lines are engine-bridge code that goes away; VFS, storage write path, Iceberg catalog core and `SpaceUri` (~2,600 lines) carry over.

### 13.3 Repository Layout for User-Facing Apps

The apps that people use (web now; desktop and phone later) live in this repository beside the Rust core. Each directory below is created when the work that needs it starts (tag = apps-track milestone, §14). Until then it exists only in this document.

```
oikome/
├── Cargo.toml        Rust workspace: crates/*, tests, tests/tools/data-gen; apps/desktop/src-tauri later
├── package.json      npm workspaces: packages/*, e2e; apps/* and plugins/* join when created        [M1a ✅]
├── crates/           Rust core; adds oikome-api [M1b] and oikome-mcp [M3]
├── apps/             one thin shell per client: entry, platform glue, packaging; no screens
│   ├── web/          the web app, served by `oikome serve --web --web-root apps/web/dist`          [M1c ✅]
│   ├── desktop/      Tauri; runs a Full Node in process                                             [later]
│   │   └── src-tauri/
│   └── mobile/       Tauri 2 mobile; reaches the owner's node over WebRTC, as a phone browser does  [later]
├── packages/         TypeScript shared by every client
│   ├── screens/      every screen and feature, desktop and phone layouts, UI strings                [M1c ✅]
│   ├── ui/           design system: tokens and components                                           [M1c ✅]
│   ├── client/       NodeClient interface, data hooks, HTTP and browser-node implementations        [M1b]
│   ├── node/         browser-node runtime: host, store, transport, wasm build                       [M1a ✅]
│   ├── api/          TypeScript types generated from oikome-api                                     [M1b]
│   └── plugin-sdk/   message bridge and theme tokens for plugin authors                             [M2]
├── plugins/          official reference plugins: recent-notes, storage, smart-home, on-this-day     [M2]
├── design/           PRODUCT.md, DESIGN.md, design.json                                             [later]
└── e2e/              Playwright: browser-node tests and test page (harness/); later per client      [M1a ✅]
```

- **Three layers.** `crates/` owns data and permissions. `packages/` owns screens and client logic. `apps/` owns how a client starts, reaches a node and is packaged.
- **Screens live in `packages/screens`, not in an app**, so desktop and mobile reuse them instead of extracting them from the web app later.
- **M1a moved `web/`** without changing behavior:

| Before M1a | Now |
|------------|-----|
| `web/src/host.js`, `store.js`, `transport.js` | `packages/node/src/` |
| `web/scripts/build.sh` | `packages/node/scripts/build.sh` |
| `web/index.html`, `web/src/main.js` (test page that exposes `window.oikome`) | `e2e/harness/` |
| `web/tests/`, `web/playwright.config.mjs` | `e2e/` |
| `web/package.json`, `web/package-lock.json` | `packages/node/package.json`; root `package.json` and `package-lock.json` (workspaces) |
| `web/` entries in `.gitignore`, paths in `browser-e2e.yml` | the new paths |

- **Packages.** `@oikome/node` exports `host`, `store`, `transport` and `wasm` (the wasm-bindgen output in `pkg/`). `@oikome/e2e` holds the tests; its test page imports `@oikome/node`.
- **Commands** at the repository root: `npm ci`; `npm run build` (the wasm, then the test page); `npm test` (Playwright). The build and `remote.spec.mjs` follow `CARGO_TARGET_DIR`.
- **Test server** (`e2e/serve.mjs`). It maps URL prefixes to directories: `/pkg/` to `packages/node/pkg`, `/node_modules/` to `node_modules` (duckdb-wasm's bundles) and `/` to `e2e/harness`. A path that leaves its directory is refused.

### 13.4 Apps and the API Layer

```
 apps/web (browser)          apps/desktop (Tauri, later)        AI tools (MCP clients)
       │ HTTP                        │ in-process call                  │ MCP (stdio or HTTP)
 oikome-server --web                 │                              oikome-mcp            [M3]
       └─────────────────────────────┼──────────────────────────────────┘
                               oikome-api                                                 [M1b]
                request and response types · handlers · caller identity
                                     │
                oikome-node · oikome-semantic · …   (grants and policies enforced here)
```

- **`oikome-api`** is what Oikome offers to everything above the node: its own clients, plugins and MCP. The peer-to-peer protocol stays in `oikome-protocol` / `oikome-network`.
  - It has no transport.
  - Every call carries the caller: **Owner**, or **Principal** (a DID with a UCAN, e.g. an AI tool or a plugin). Owner-only operations (create Spaces, connect sources, share, issue and revoke grants) refuse a Principal. A Principal's reads go through the semantic layer like a peer's (§6), so it sees only what it is granted.
- **HTTP** is a thin layer in `oikome-server` (`web.rs`), turned on by `oikome serve --web` (`--web-listen`, default `127.0.0.1:7340`). It serves the API now and the embedded `apps/web` build from M1c.
  - It listens on loopback only.
  - **Login.** A one-time token (32 random bytes, 10 minutes) is written into a login link in `.oikome/web/login`, readable by the owner only; it is never printed or logged. `oikome web open` opens that link in the browser. Using it sets the cookie `oikome_session_<port>` (HttpOnly, SameSite=Strict; the port keeps two nodes on one machine apart) and replaces the token. Sessions live in memory and end after 12 idle hours or a restart. The login file is removed when the node stops.
  - **Every request**: `Host` must name this server (DNS rebinding).
  - **API calls**: `Origin` must be this server, `Sec-Fetch-Site` (when sent) `same-origin`, `Content-Type` `application/json`, and the session valid. Bodies are capped at 1 MiB; token comparison is constant-time.
  - Operations are `POST /api/<operation>` with a JSON request. Replies are JSON, or an Arrow IPC stream (`application/vnd.apache.arrow.stream`) for rows. Errors are `{code, message}` with code `invalid` (400), `unauthenticated` (401), `permission_denied` (403), `not_found` (404), `conflict` (409) or `internal` (500, details only in the node's log).
- **Desktop** (later) links `oikome-api` into its Tauri process and runs the Full Node there. Tauri commands call the handlers directly, so no port is opened.
- **`packages/api`** is generated from `oikome-api`'s types with ts-rs (MIT). The `bindings` test fails when `packages/api/src/index.ts` is out of date; `UPDATE_BINDINGS=1 cargo test -p oikome-api --test bindings` rewrites it.
- **Operations (M1b)**, the same names for HTTP, later Tauri commands and MCP tools. All are owner-only except `node_info`:

| Operation | What it does |
|-----------|--------------|
| `node_info` | DID, version and the Flight URL peers reach |
| `list_spaces`, `create_space`, `drop_space` | Local Spaces and how many contacts each is shared with. `drop_space` first revokes the Space's active shares, since a token names a Space by name and would work again on a new Space of that name |
| `list_tables`, `read_rows` | Tables and views; a page of rows (at most 1,000) as Arrow |
| `run_sql` | The owner's statements in one session (at most 256 KiB); the last result as Arrow (cut at 10,000 rows or about 32 MiB, flagged by `oikome-truncated: true`) or `{rowsAffected}` |
| `list_contacts`, `add_contact`, `rename_contact`, `remove_contact` | Names for the DIDs the owner shares with (`contacts.json`). Removing a contact revokes their shares and whole-Space grants |
| `share_space` | A whole-Space grant plus a UCAN (`space/query`, or `space/read` to allow copying an analytical Space) for 1 to 366 days; the token is returned once. Only to contacts |
| `list_shares`, `revoke_share` | Shares: issued UCANs whose capabilities all name one Space (tokens issued with SQL for several Spaces or `*` are not listed). Revoking the contact's last active share of a Space removes their grant on it |
| `list_access` | The access log (§9), newest first, by DID or Space, with contact names |

- The API never builds SQL from request fields: names are checked against the catalog and operations call the node directly. Only `run_sql` runs SQL, the owner's own.
- **`oikome-mcp`** (M3) exposes Principal operations only: list the Spaces it may read, then search and query within its grants. It never exposes Owner commands.
  - Grants for AI tools are managed on the app's AI Assistants screen.
  - `oikome mcp`, when started by an AI client, is a thin bridge: it reaches the running node with that assistant's UCAN. It does not open the data directory, where it would contend for DuckDB and SQLite file locks.
  - The MCP SDK and its license are checked when M3 starts.
- **Plugins** (M2) run sandboxed and talk to the app through `packages/plugin-sdk`. Each is a Principal with its own grants, and has no network access.

How each client reaches data (screens depend only on `NodeClient`):

| Client | Runs in | Reaches data through | Layout |
|--------|---------|----------------------|--------|
| Web, desktop browser | Page served by the Full Node | HTTP on loopback, owner session | Desktop |
| Desktop app (later) | Tauri window | In-process calls to `oikome-api` | Desktop |
| Phone app (later) | Tauri 2 mobile, paired with the owner's node | WebRTC with a UCAN issued at pairing | Phone |

**Phones.** Pairing a phone comes with the phone app. A phone browser would need the browser node's page over HTTPS, which the owner's loopback server cannot give it, and a page from a public origin or a self-signed certificate on the local network was turned down. The screens have a phone layout already, which the phone app reuses. Which Owner operations a paired phone may perform is decided with it.

**The web app (M1c).**
- **Stack:** React 19 and Vite 7, built into `apps/web/dist`. Vite 8 was not used: it requires lightningcss, which is MPL-2.0.
  - `packages/ui` holds the design tokens and components.
  - `packages/screens` holds the screens and all UI strings (English, ready for i18n), a small History-API router, and data loading through `NodeClient`.
  - `apps/web` renders them with `HttpClient`.
- **Serving:** `oikome serve --web --web-root apps/web/dist` (`OIKOME_WEB_ROOT`).
  - GET serves the app's files under `Content-Security-Policy: default-src 'self'` (no inline scripts or styles, `frame-ancestors 'none'`) with `X-Frame-Options: DENY`.
  - Paths are checked segment by segment (no escapes, dots or hidden names) and must resolve inside the root. A path without an extension gets `index.html`.
  - Hashed assets are cached for a year; everything else is `no-store`.
  - Embedding the build in the binary waits for packaging.
- **Words for people:** Collection (a Space), Contact, Share, Stop sharing, Activity (the access log), Your ID (the DID), invite code (the token). The SQL workbench lives under Settings → Advanced.
- **Recovery key:** the identity seed, shown to the owner as 56 Crockford Base32 characters in 14 groups. It is made of a version byte, the 32-byte seed and two BLAKE3 check bytes (`oikome-identity::recovery`).
  - Reading it forgives case, spaces and look-alike characters, and rejects typos.
  - `recovery_key` returns it (owner only, never logged); `confirm_recovery_key` records that it was stored. The node's name and that record live in `.oikome/node.json`.
  - `oikome identity restore` reads a key from standard input and writes it as the node key. It only does this on a node that holds nothing yet: no Spaces, contacts or issued tokens. A running node keeps its identity until it is restarted, so the web app only explains this step; the desktop app can do it in process later.

**Scope of the apps.**
- They serve the owner's personal data center: onboarding, Spaces, sources, sharing, AI assistants, plugins and settings. Node operations (engines, logs, peers' internals) stay in the CLI. Members and family roles come later.
- The UI is English; other languages arrive through i18n later.
- Visual design: Apple-style minimal on a low-chroma cream palette. One crimson primary action per view; blue only for links and focus. Its tokens and rules move into `design/` with DESIGN.md.
- **Storage.** The web app's onboarding shows where the data is kept; choosing a NAS (§7.5, phase 3.4a) moves into the apps with a storage screen. Cloud drives come through their sync folders (3.4b, always encrypted) and later over their API (3.4c); the apps offer each as it arrives. Credentials and keys are entered in the apps and kept by the node (§7.5). Browsing files already on a drive (as AList does) and serving Oikome over WebDAV are planned but not scheduled.

---

## 14. Roadmap (v3)

| Phase | Goal | Exit criterion |
|-------|------|----------------|
| **3.0-A Engines** ✅ | `oikome-engine` trait; DuckDB + SQLite engines; Location/Space split in catalog; `CREATE SPACE ... KIND`; single-Space queries pushed entirely to the engine | TPC-H Q1–Q5 on an analytical Space via DuckDB; an OLTP smoke test (BEGIN/INSERT/COMMIT/SELECT) on a transactional Space |
| **3.0-B Federation** ✅ | DataFusion coordinator over `datafusion-federation` + `EngineSqlExecutor`; cross-Space join (Parquet ⋈ SQLite) locally; snapshot pinning | Two-Space join returns correct results; EXPLAIN shows two pushed-down SQL leaves |
| **3.0-C Semantic** ✅ | Views, column grants and row policies as SQL rewrites; principals read-only and deny-by-default; enumeration resistance | Policy test suite: a restricted DID sees rewritten `*` and injected predicates inside the engine SQL |
| **3.0-D Publish** ✅ | `PUBLISH SPACE` → `orders_published` analytical read-only Space; scheduled publisher | Publish → query via DuckDB matches SQLite source |
| **3.1 Network** ✅ | DID/UCAN, Arrow Flight remote Spaces (3.1a ✅), snapshot subscription (3.1b ✅), libp2p discovery and TLS (3.1c ✅), two Docker nodes plus an mDNS-only node, run in CI (3.1d ✅) | Two Docker nodes: UCAN-gated federated query; revocation cuts access |
| **3.2 Browser** ✅ | WASM core + `HostEngine` on duckdb-wasm (3.2a ✅); federated queries via a Full Node over WebRTC (3.2b ✅); browser SQLite later | Browser node runs a local query and a federated query via a Full Node |
| **3.3 Multimodal** ✅ | Lance Locations through DuckDB's `lance` extension: vector, full-text and hybrid search, indexes, version sync (3.3a ✅); FTS5/`sqlite-vec` on transactional Spaces (3.3b ✅); DuckPGQ on Lance Spaces (3.3c ✅); capability checks | Vector + FTS query on a Lance Space and on a transactional Space; a subscribed Lance Space is searchable without rebuilding its index |
| **3.4a NAS** ✅ | Storages registered per node (`oikome storage add`: WebDAV or a mounted folder); the writer pushes each snapshot, other nodes copy the newest complete one; one writing node per folder; credentials from the environment; transactional Spaces refused | An analytical Space (Iceberg and Lance) on a WebDAV NAS is written by one node and read by another, and the Lance copy is searched with the writer's index; a second node cannot bind it writable; a transactional Space on a NAS is rejected at DDL; a reader never reads a snapshot with missing files |
| **3.4b Cloud drives** ✅ | Client-side encryption of every file and name on a storage; drives through their desktop client's sync folder (`--drive`, always encrypted); credentials and keys kept by the node (OS keychain, else a private file), entered once in the apps or on the command line | A Space on a drive's sync folder is written by one node and read by another with the same key; without the key the drive's files cannot be read (no content, table names or paths), and a wrong key is reported; credentials are in the keychain or the node's private file only, never the registry, SQL or logs, and the environment overrides them |
| **3.4c Drives over their API** | OAuth sign-in and opendal drive services (Google Drive, Dropbox, OneDrive) for nodes without a desktop client, encrypted as in 3.4b | Set when 3.4c is planned |
| **3.5 Connectors** | `Connector` contract; upsert and cursor in one SQLite transaction; scheduled and on-demand pulls; credentials from the OS keychain; file import and flomo connectors | flomo memos land in `flomo.memos`; a second pull touches only changed memos; an edited memo is updated, not duplicated; a failed pull leaves data and cursor unchanged |
| **3.6 Compute sharing** | `compute/<runtime>` UCAN abilities with limits; task routing over Arrow Flight; host-registered runtimes; per-peer quotas | Node B runs a task for node A within its quota; a task over its limits is refused; revoking the UCAN drops A's queued tasks; A's compute grant gives no access to B's Spaces, and a data grant does not let A run tasks |

No phase skipping (AGENT.md §4.1).

### 14.1 Apps Track

The user-facing apps (§13.3, §13.4) run beside the core phases. They do not block the core phases, and the core phases do not wait for them. A milestone starts only when its gate phases are done; within the track, too, there is no skipping.

| Milestone | Goal | Gate | Exit criterion |
|-----------|------|------|----------------|
| **M1a Layout** ✅ | `web/` split into `packages/node` and `e2e/`; root `package.json` with npm workspaces | 3.2 ✅ | `browser-e2e` passes from the new paths with no change to the tests' assertions |
| **M1b API** ✅ | `oikome-api` with the Owner operations of §13.4 (Spaces, rows, SQL, contacts, sharing and revoking, access log); whole-Space grants; the access log; `oikome serve --web` and `oikome web open`; `packages/api`; `packages/client` with its HTTP implementation. The recovery key and phone pairing are designed separately before M1c | 3.0–3.2 ✅, M1a | e2e starts `oikome serve --web` on an empty temporary data directory and, through `packages/client`, creates a Space, writes and reads rows, shares the Space with a contact and revokes it, after which the contact's query fails. A request without the session cookie or with a foreign `Origin` is refused |
| **M1c Usable** ✅ | `packages/ui`, `packages/screens` and `apps/web`: the M1 screens (onboarding and the recovery key, Collections, sharing and contacts, Activity, the SQL workbench, settings), desktop and phone layouts; `--web-root`; `oikome identity restore`. Phone pairing moves to the phone app | M1b | In e2e on CI (`web-app.spec.mjs`), in Chromium against a local node started from an empty data directory: onboarding and the recovery key; a collection filled in the SQL workbench and browsed; the collection shared with a contact (a second node) and stopped, after which the contact's query fails; both in Activity. Each M1 screen renders at 1280 and 390 px without sideways scrolling, and no text fails WCAG AA |
| **M2 Connect** | Sources screen and sync status, Home dashboard, built-in cards, dashboard plugins v1 (`plugin-sdk`, data contract, plugin grants), reference plugins | 3.5, M1c | A connector's data appears on Home; a plugin reads only what its grant allows and cannot reach the network |
| **M3 Find** | Full-text and semantic search, photo and note views, AI Assistants screen, `oikome-mcp` | 3.3 ✅, M2 | An MCP client lists and searches only the Spaces granted to it; revoking the grant in the app cuts it off; Owner commands are not offered over MCP |
| **M4 Extend** | Borrowed compute, third-party connector plugins, plugin directory; cloud-drive storage as 3.4b and 3.4c bring it (§7.5, §13.4) | 3.4b, 3.6, M3 | Set when M4 starts |

**Tests use a local node.** e2e starts `oikome serve` on a temporary data directory and writes its test data through the CLI or the API. There is no fake-data layer and no mock `NodeClient`. The API comes before the screens, and each screen arrives with the milestone whose data it shows: Sources and Home in M2, search and AI Assistants in M3.

---

## 15. Risks

| Risk | Mitigation |
|------|------------|
| `datafusion-federation` lags DataFusion releases (0.5.5 is the last for DF 54 / Arrow 58) | Pin `datafusion` / `datafusion-federation` / `duckdb` / `iceberg` together in `[workspace.dependencies]`; upgrade as a unit. Our adapter (`EngineSqlExecutor` + `OikomeRemoteTable`) is ~250 lines and can be forked if blocked. |
| DataFusion 54 unparser quirks in pushed-down SQL | Known: (1) a filter pushed below a table alias is qualified by the base name (`FROM t AS o WHERE t.x`) — fixed by the `OikomeRemoteTable` AST analyzer; (2) joining two same-named tables from different Spaces without aliases (`a.t JOIN b.t`) produces ambiguous `"t"` qualifiers — users must alias them. Both covered by tests. |
| Filters DataFusion pushes into `VirtualExecutionPlan` at execution time | `datafusion-federation` 0.5.5 reports them as pushed without adding them to the SQL; `EngineSqlExecutor::execute` evaluates them on each returned batch so results stay correct. |
| DuckDB `bundled` build time / binary size | Feature-gated; CI caches `libduckdb-sys`. Not shipped to browser at all. |
| Pushdown misses (residual operators execute in DataFusion over large Arrow streams) | `EXPLAIN` surfaces pushed vs residual; planner emits a warning when a leaf returns > N batches without pushdown. |
| Cross-engine type mapping (SQLite dynamic typing → Arrow) | Catalog stores declared Arrow schema per transactional table; `SqliteEngine` casts on the way out. |
| Two Spaces over one Location with divergent policies leak via timing/side-channels | Policies are applied per Space before pushdown; engines see only the rewritten SQL. Documented; audited in Phase 3.1 security review. |
| `datafusion-federation` unparser cannot express an engine-specific construct (e.g. `MATCH` in DuckPGQ) | Semantic layer marks such subtrees as *opaque* and pushes the original text verbatim to the single owning engine; opaque subtrees cannot span Spaces. |
| WebDAV servers differ (MOVE overwrite, PROPFIND depth, chunked uploads) | Every upload goes to a temporary name, is checked for size and renamed; tested against `dav-server` and rclone; NAS models are tested on real devices as they come |
| A node needs room for the current snapshot of every Space on a storage it reads | Stated; a size-bounded cache helps only once engines read part of a table |
| Two nodes claim one folder at the same moment (WebDAV has no create-if-absent) | The claim is read back; the DID record guards against mistakes, the storage's accounts control access |
| The keychain crates are young (1.x, 2025–2026) | Pinned exactly; tests use an in-memory or file store and never the machine's keychain; the keychain path was checked by hand on macOS |
| macOS may ask again for keychain access after the binary is rebuilt (its signature changes) | Stated; signed release builds keep one signature |
| The only key of an encrypted storage is lost | The writer still holds the Space on its disk; `export-key` lets the owner keep a copy; stated where the key is made |
| Drive APIs change, rate-limit or expire tokens (3.4c) | Services are maintained in opendal and feature-gated; the sync-folder path needs no API; a failed upload leaves the previous snapshot readable |
| Sync clients reorder uploads, create conflict copies or leave placeholders (3.4b) | Snapshot files are written after the files they name; readers take the newest complete snapshot; files no snapshot names are ignored |
| A connector breaks when an app changes its API or export format | Connectors are isolated from each other; a failed pull changes nothing because data and cursor commit together; only official APIs and exports are used |
| Data sent for remote compute cannot be proven deleted | Only to explicitly trusted peers; the input is a chosen query result, not a Space; documented as trust-based |
| Peers overuse a node's compute | Deny-by-default, per-peer quotas, UCAN limits and expiry |
| DataFusion 54 spawns Tokio tasks in operators the browser coordinator has not met yet | `MergePartitionsExec` covers partition merges; the native single-task test runs joins, unions, aggregates and limits without a runtime; a new plan shape gets a test before it is relied on, and the upstream fix lands with DataFusion 55 / 56 |
| A browser can reach a Full Node only at a directly routable UDP address | Same network or a public address for now; ICE is already in the connection, so STUN / TURN can be added without changing records or requests |
| DuckDB's `lance` extension cannot read a given version or list a version's files, and is not published for WebAssembly | Lance reads see the latest version (§5.4); `lance-table`, at the extension's lance version, lists a version's files for subscription; browser nodes go through a Full Node |
| The extension and `lance-table` disagree about a dataset after one of them is upgraded | Both pinned to the same lance release (12, DuckDB 1.5.6) and upgraded together; the subscription test copies a version listed by `lance-table` and searches it with the extension |
| The extension caches each dataset per DuckDB connection, so a long-lived connection reads a stale version | Every statement runs on a fresh connection; a test reads through a second DuckDB after writes by the first |
| `lance-io` uses `object_store` 0.14, DataFusion 54 uses 0.13 | Two copies, native nodes only; they share nothing at run time |
| DuckPGQ is a research project, and cannot define graphs over views | Graphs are on Lance Spaces only; definitions are kept by Oikome and replayed; the extension is pinned with DuckDB |
| sqlite-vec is pre-1.0, and 0.1.9 searches by brute force | Pinned exactly; fine at personal scale; its DiskANN and IVF indexes arrive with later releases, which change only the `vec0(...)` options |
| Fewer external tools read Lance than Parquet / Iceberg | Lance only for data that is searched; tables that are aggregated stay Iceberg + Parquet |

---

## 16. Decision Log

| Date | Decision | Rationale |
|------|----------|-----------|
| 2026-09-27 | Stop building computation; delegate to DuckDB / SQLite | Self-built OLAP is out of scope for a small team; differentiation is sovereignty + federation, not operators |
| 2026-09-27 | Keep DataFusion as coordinator only | Needed for cross-engine / cross-node merge; `datafusion-federation` already solves pushdown-as-SQL |
| 2026-09-27 | Engine bound per Space, not per table | Keeps authorization, sync and engine boundaries aligned |
| 2026-09-27 | Split Space (logical) from Location (physical); N:1 with a single writer | Allows the same data to be exposed under different engines/policies without duplicating files |
| 2026-09-27 | Transactional Spaces publish Parquet snapshots; no WAL replication in v3.0 | Keeps the P2P layer to one artefact type (immutable files) |
| 2026-09-27 | Multimodal via engine extensions; drop ART/CSR/HNSW/Cypher | Extensions exist and are maintained; custom indexes were the largest remaining scope |
| 2026-09-27 | Bridge engines with our own `SQLExecutor` instead of `datafusion-table-providers` | One path to each engine (`QueryEngine`), no second connection pool, no pinned old `rusqlite`; the same executor will wrap remote and browser-host engines |
| 2026-09-29 | Run the federation optimizer rule after `push_down_filter` | With the upstream default position, filters above a join stayed in DataFusion instead of reaching the engine SQL |
| 2026-09-30 | Overwrite snapshots produced by `OikomeCatalog` itself (`replace_data_files`) | iceberg-rust 0.10 only exposes fast-append; publishing needs replace semantics and compaction was duplicating rows |
| 2026-09-30 | One private SQLite connection per open transaction | A shared connection let one session's `ROLLBACK` discard other sessions' autocommit writes |
| 2026-09-30 | Grants and policies use PostgreSQL syntax (`GRANT SELECT (cols)`, `CREATE POLICY … USING`); views run with owner rights | Familiar syntax parsed by sqlparser as-is; owner-rights views are what makes a view a publishable semantic interface |
| 2026-10-30 | Reposition as an AI-native, multimodal private data center that usually runs on personal resources and supports trusted data exchange: connectors bring data in, owned storage (including cloud drives) keeps it, local engines compute, apps and AI tools read it | Keep data and compute with individuals while letting them pool both with each other rather than with a platform; the v3 layers are its core, unchanged |
| 2026-10-30 | Cloud drives through opendal services or the drive's sync folder; SQLite files never on a drive | opendal is already the VFS; SQLite's WAL needs a local filesystem |
| 2026-10-30 | Connectors write into transactional Spaces and share through `PUBLISH` | Sources edit and delete records, which needs upserts; publishing keeps a single sharing path |
| 2026-10-30 | Compute sharing is separate from data exchange: same trust mechanism (DID, peers, deny-by-default, UCAN revocation), separate authorization (`compute/<runtime>` vs `space/query` / `space/read`); tasks run only on runtimes the executing node registers | A data grant and a compute grant answer different questions: which data a peer may see, and whose machine a task may use. Running on registered runtimes keeps principle 1 and avoids shipping code |
| 2026-10-30 | Subscription copies only tables the subscriber may read in full; others are reported `restricted` and stay query-only | Copied files cannot be filtered by column grants or row policies; publishing a subset to its own Space is the way to share part of a table |
| 2026-10-30 | Revoking a `space/read` token stops refreshes; copied files stay with the subscriber | Data shipping cannot take data back; stated rather than implied |
| 2026-10-30 | A subscribed copy is a remote Space (read-only, never served onwards) bound to a replica Location and read by the local DuckDB; files are named by BLAKE3 hash and the manifest is swapped by rename | Reuses every remote-Space guard; hashes verify the download and keep owner paths private; a failed refresh never leaves a half copy |
| 2026-10-30 | TLS between nodes with self-signed certificates pinned to the owner's DID; plaintext only with `--allow-http` | The DID is already the identity every request is signed with; pinning it needs no CA or host names, and an address that points at another node fails the handshake |
| 2026-10-30 | libp2p only for discovery (mDNS + endpoint exchange), results kept in the catalog's Peers table; `PEER` optional | Keeps data on Flight; the libp2p key is the DID key, so nothing new has to be trusted; a table in the catalog serves both `oikome serve` and `oikome sql` |
| 2026-10-31 | Browser node: duckdb-wasm only in 3.2 (SQLite later); `HostEngine` keeps an id, JS objects stay in a thread-local table, results cross as Arrow IPC | `QueryEngine` and DataFusion need `Send`; the id-plus-channel bridge keeps the trait unchanged instead of forking DataFusion |
| 2026-10-31 | DataFusion without `compression` / `parquet`, `datafusion-federation` 0.5.5 patched to stop forcing them; Homebrew / distribution clang for the zstd that remains | The coordinator never reads files; 0.5.7 fixes this upstream but needs DataFusion 55 / Arrow 59, which DuckDB and Iceberg do not support yet |
| 2026-10-31 | Browser ↔ Full Node over a WebRTC data channel, the DTLS fingerprint vouched for by a DID-signed record | Browsers cannot pin a certificate to a DID and Chrome rejects Ed25519 certificates; WebRTC lets the browser pin a fingerprint, works in every major browser, needs no 14-day rotation and brings ICE for later NAT traversal |
| 2026-11-01 | WebRTC endpoint on str0m with its pure-Rust crypto; WebRTC-direct (sessions named by the browser's ICE fragment, SDP answer built by the browser); the server does not verify the browser's DTLS certificate | No C crypto in the server; no signaling service; the browser is authenticated by its signed requests, exactly as a native peer, and the spike showed browsers reject a wrong pinned fingerprint at the handshake |
| 2026-11-01 | Browser requests are envelopes with the Flight call's fields, served by `OikomeFlightService::serve_envelope` | One authentication, authorization and semantic layer for both transports; replies carry the whole result as one Arrow IPC message |
| 2026-11-01 | The browser coordinator runs in one task: one target partition, `MergePartitionsExec` instead of `CoalescePartitionsExec`; chrono `wasmbind` | DataFusion 54 spawns Tokio tasks to merge partitions and reads the clock through `std`, both of which panic in the browser; merging by polling keeps remote partitions concurrent |
| 2026-11-01 | DuckDB 1.5.5 → 1.5.6 | DuckPGQ has no 1.5.5 build, and 1.5.6's `lance` extension is built on lance 12 with DataFusion 54, the versions Oikome uses |
| 2026-11-01 | Lance Spaces read the latest version; subscriptions ship one version, listed with `lance-table` | The extension cannot read a given version or list its files; shipping exactly one version keeps deleted rows on the owner and lets indexes travel |
| 2026-11-01 | Search functions only for the owner (and on subscribed copies), within one Space; peers subscribe to search | A search reads the dataset past column grants and row policies; subscription copies only tables the peer may read in full |
| 2026-11-01 | No `MODES` declaration on Spaces: required extensions follow from the format and the statement, checked against `Capabilities` | A declaration could drift from what the Space actually holds |
| 2026-11-01 | Graphs (DuckPGQ) on Lance Spaces only; definitions in the catalog, created again in a DuckDB that lacks them; owner only, one Space | DuckPGQ refuses graphs over views, and Iceberg tables reach DuckDB as views; copying them into tables for every snapshot was turned down |
| 2026-11-01 | FTS5 and sqlite-vec in transactional Spaces: `CREATE VIRTUAL TABLE` read by Oikome (module arguments as written, fts5 and vec0 only), `MATCH` parsed with the SQLite dialect, for the owner within one Space; `PUBLISH` skips virtual tables | sqlparser cannot read the module arguments, and its generic dialect has no `MATCH`; a dialect of our own would change how other statements parse |
| 2026-11-01 | DuckDB extensions are fetched only by `oikome extensions install` (and in the Docker image build), never by a query | A query never downloads code; the node's extension directory is explicit |
| 2026-10-30 | The 3.1 exit run is three Docker nodes on GitHub Actions (`docker-e2e`), one of them found only by mDNS | Development machines may have no container runtime; CI runs it on every push, and the Docker bridge network gives mDNS the multicast a firewalled laptop may block |
| 2026-10-30 | VFS `materialize()` moves from 3.1 to 3.4 | Subscription copies live on local disk; the VFS is first needed for cloud drives |
| 2026-10-30 | Lance as a third Location format for vector, full-text and large-object data; it replaces DuckDB `vss` / `fts` for analytical Spaces | Lance indexes live in the dataset and sync with its versions, while extension indexes must be rebuilt on every subscriber; Lance versions are immutable files plus a manifest, which fits principle 4; Iceberg + Parquet stays for tables that are scanned and aggregated |
| 2026-11-01 | User-facing apps serve the owner's personal data center only: no node-operations or developer screens; members later; English UI, i18n later | The apps are for ordinary people using their data; operating the node stays in the CLI; the audience is global |
| 2026-11-01 | Repository layout: `apps/` (thin shells), `packages/` (shared TypeScript), `plugins/`, `design/` and `e2e/` at the root beside `crates/`; root `package.json` (npm workspaces) beside `Cargo.toml`; a directory is created only when its work starts | Web, desktop and phone are peers, so `apps/` sits at the root rather than inside `web/`; shared screens avoid extracting them from one app later; directories for unbuilt features would only describe intent, which this document already does |
| 2026-11-01 | `oikome-api`: one transport-free API with the caller as Owner or Principal; HTTP in `oikome-server --web`, desktop in process, MCP in `oikome-mcp` exposing Principal operations only; `packages/api` generated from it | One set of handlers for every client; the name covers plugins and MCP, not just Oikome's own apps, while the peer protocol keeps its own crates; an AI tool can never reach Owner commands |
| 2026-11-01 | Desktop and phone apps on Tauri (later) | Its backend is Rust and links our crates directly, so the desktop app is a Full Node with a UI and no command line; Tauri 2 also targets iOS and Android; Apache-2.0 / MIT |
| 2026-11-01 | The apps offer local folders and NAS as storage; cloud drives as Locations stay undecided, with §7.5 and phase 3.4 unchanged until then | The product rule is that data stays at home, which conflicts with keeping it on a drive provider's servers; browsing existing drive files and serving WebDAV are planned separately |
| 2026-11-02 | Storages: a NAS first (3.4a, WebDAV or a mounted folder); cloud drives step by step, each only once its files are encrypted on the client (3.4b onward). Settles the entry above | Data on a NAS stays at home; a drive provider can read what is uploaded, so encryption comes first |
| 2026-11-02 | A Space on a storage is copied, not read in place: the writer keeps it on its disk and pushes each snapshot (files, then a snapshot file); readers copy the newest complete snapshot and read it like a subscription. `materialize()` dropped | DuckDB reads Iceberg as lists of local Parquet files and the `lance` extension attaches only local directories; Iceberg commits over WebDAV have no compare-and-swap; the subscription code already copies verified snapshots, so one mechanism serves Iceberg and Lance, and encryption later wraps each file |
| 2026-11-02 | One writing node per folder, recorded in `oikome-space.json`; passwords and tokens only from the environment; a failed push does not fail the write | WebDAV has no conditional create, so the record is a guard, not access control; servers and containers have no keychain (it comes with 3.5); failing a committed write invites a retry that duplicates rows |
| 2026-11-02 | 3.4b: drives through their sync folders first, over their API in 3.4c; every file on a drive sealed whole (XChaCha20-Poly1305 STREAM, names from a keyed hash), one random key per storage | Sync folders cover iCloud Drive, Dropbox, OneDrive and Google Drive with no OAuth app; whole-file sealing in `RemoteStore` covers data, indexes, snapshots and names alike, which Parquet modular encryption would not |
| 2026-11-02 | Credentials are entered once (apps, or standard input to the CLI) and kept by the node: OS keychain on macOS / Windows, else a private file beside the node key; the environment overrides; this replaces "environment only" for 3.4a | People should not have to set environment variables; the apps call the same node functions (`add_storage`, `set_storage_secret`, `export_storage_key`); servers and containers keep the environment |
| 2026-11-01 | App tests run against a local node (temporary data directory, test data written through the CLI or the API); no fake data and no mock `NodeClient`. So the API (M1b) comes before the screens (M1c), and each screen arrives with the milestone whose data it shows | One test chain instead of a fake one followed by a real one; screens exercise the real API from the start, and there are no fixtures to drift from it |
| 2026-11-01 | Whole-Space grants (`GRANT SELECT ON SPACE`) in their own registry; they cover a view only when its expansion stays in the Space | Sharing is per Space in the apps and should include tables created later; a relation name such as `"*"` could not stand for the Space safely; owner-rights views would otherwise open other Spaces |
| 2026-11-01 | The access log is SQLite under `.oikome/`, written by a thread behind a bounded channel; failed authentications merged per method and minute | Requests never wait for logging; the owner can page and filter it; unauthenticated callers cannot push real entries out |
| 2026-11-01 | The web API's login link is written to an owner-only file and opened with `oikome web open`, never printed | A token in terminal output stays in scrollback and logs; whoever can read the data directory is the owner anyway |
| 2026-11-01 | Registry writes take a file lock and re-read the files first; `refresh` reads under a shared lock | With the web API, `serve` writes the registry too; a `REVOKE UCAN` from `oikome sql` must not be undone by its next write |
| 2026-11-02 | Phone pairing comes with the phone app, not the web app | A phone browser needs the browser node's page over HTTPS; the loopback server cannot serve it, and a public origin or a self-signed local certificate was turned down. The screens already have a phone layout for the app |
| 2026-11-02 | Recovery key: Crockford Base32 groups with two check bytes; restored only into an empty node, with `oikome identity restore` | No dependency (unlike a word list); a typo is caught instead of restoring another identity; a running node cannot swap identities, so restore happens while it is stopped |
| 2026-11-02 | Web app on React 19 and Vite 7, served from a directory (`--web-root`) under a strict CSP | React's ecosystem and Tauri templates for the later apps; Vite 8 requires lightningcss (MPL-2.0); embedding the build waits for packaging |
| 2026-11-01 | Visual design: Apple-style minimal, low-chroma cream palette, one crimson primary action per view, blue only for links and focus | Calm and familiar for ordinary users; color marks the one thing to do instead of decorating; every text pair is checked against WCAG contrast |

---

*This document is the canonical architecture baseline for Oikome v3.0. All engineering implementation proceeds from this specification. v1 and v2 remain for historical context; where they conflict with v3, v3 wins.*
