# Oikome — Architecture

> **Version**: v3.0
> **Date**: 2026-09-27
> **Codename**: oikome
> Oikome's Chinese name is 尘屋 (Chén Wū). For now it is only a note: code, paths and the interface use Oikome.
> **Positioning**: An AI-native, multimodal private data center that usually runs on personal resources and supports trusted data exchange. Built on a decentralized local federated query semantic layer: embedded, sovereign, P2P-native.
> **Core Change**: Oikome no longer owns query *computation*. Computation is delegated to pluggable embedded engines (DuckDB for analytics, SQLite for transactions; browser-hosted engines in WASM). DataFusion is retained **only** as the federation coordinator that merges results across engines and nodes. Oikome's own value concentrates in the semantic layer (naming, policy, views), the federation layer (routing, pushdown, merge), identity/authorization (DID/UCAN) and P2P (discovery, snapshot sync, Arrow Flight).
> **Status**: 3.0-A (engines), 3.0-B (local federation), 3.0-C (semantic layer) and 3.0-D (publishing) implemented on `main`. Supersedes v2.0.
> **Revised 2026-10-30**: positioning changed to an AI-native, multimodal private data center (§1). Added cloud drives (§7.5), Lance as a third Location format (§7.6), connectors (§11), compute sharing (§12) and roadmap phases 3.4–3.6. The engine, federation and semantic layers are unchanged.
> **Revised 2026-11-01**: added the user-facing apps: repository layout (§13.3), apps and the API layer (§13.4) and an apps track beside the core phases (§14.1).
> **Revised 2026-11-02**: storages (§7.5): a NAS at home first (3.4a), cloud drives step by step, each once files are encrypted on the client (3.4b onward). 3.4b: client-side encryption, drives through their sync folders, credentials kept by the node (keychain). 3.5: connectors (§11): CSV files and flomo (HTML export; MCP, not yet run against flomo).
> **Revised 2026-11-04**: security (§17): what is protected and who can see what, defaults kept sound rather than maximal, a per-node security mode, known gaps and what is left for later; phase 3.7. Public shares to anyone within the owner's limits, Web2 and Web3 (wallets as identities, chains as sources, no token), and paid access through payment plugins the owner picks (§1, §11.1, §13.4); phase 3.8 and milestone M5.
> **Revised 2026-11-04**: files are viewed where they are (§7.7). A copy is made only when the owner saves one somewhere (§7.8), and a saved Space can be shared on its own, as a portfolio or publicly (§7.9). Storages are sources as well as targets. Drives over their API (3.4c), Baidu Netdisk (3.4d) and object storage (3.4e) are added (§7.5), with roadmap phases 3.9 and 3.10 (§14).

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

**Rescinded from v1**: columnar-only storage (SQLite is row-oriented) and a self-built vectorized engine.

---

## 1. Positioning

Oikome is an **AI-native, multimodal private data center** that usually runs on personal resources and supports trusted data exchange.

AI platforms hold more and more personal data, and storage and compute are concentrating in a few companies. Once data is handed to a platform, its owner no longer controls who uses it or how. Oikome keeps data and compute with individuals, and lets them pool both with each other rather than with a platform.

| | What it means in Oikome | Where |
|---|---|---|
| Private | The data belongs to its owner — a person, a family or a small team — and lives on storage the owner controls: local disk, NAS, cloud drives, object storage. Files are viewed where they are and copied only when the owner saves them somewhere. Connectors bring data in from the apps the owner uses | §7, §11 |
| Multimodal | Text, images, audio, files and vectors are first-class data alongside tables | §7.6, §8 |
| AI-native | AI is a primary user from the start. AI tools and agents connect through MCP like any other app (dashboards, notes), as principals that see only what they are granted. Data is kept in forms AI uses directly: text, embeddings, search indexes. Views and names tell AI what the data means. AI workloads such as embedding and inference run on personal resources | §2 L5, §6, §7.6, §12 |
| Personal resources | Storage and compute come from individuals, not from a platform's cloud: first the owner's own machines, which compute with embedded engines (DuckDB, SQLite); then other individuals' machines through compute sharing. "Usually": a cloud bucket can still serve as a Location | §4, §7, §12 |
| Trusted data exchange | Data moves between data centers that trust each other, in both directions, under grants the owner can revoke. The owner may also open chosen data to anyone, strangers included, within limits the owner sets (public shares) | §5, §6, §9, §13.4 |
| Web2 and Web3 | Data comes from the apps people use and from chains (an address's transactions and tokens); a wallet can be an identity (`did:pkh`). No token or chain of its own, private data never goes on a chain, and no wallet is needed to use Oikome | §9, §11.1, §13.4 |
| Paid access | An owner may charge for chosen data. Payment goes through a payment plugin the owner picks (PayPal, Alipay, a Web3 wallet and others) straight into the owner's own account; Oikome never holds funds. A paid order is only the condition for a grant | §13.4 |

**Data exchange and compute sharing use the same trust mechanism with different authorizations.** Both rest on DID identities, peer relationships, deny-by-default and UCAN revocation (§9). Data is authorized with `space/query` / `space/read` plus grants and policies (§5.3, §6); compute with `compute/<method>` (§12). Neither implies the other.

> **Oikome decides *who may see what*, *where data comes from*, *where it lives* and *where a query or task is sent*. It does not decide *how to compute*.**

Analogy: Oikome is to embedded engines what a service mesh is to services. DuckDB and SQLite are excellent local engines but know nothing about identity, capability delegation, peers, or multi-Space namespaces. Oikome wraps them into a sovereign, federated, P2P-addressable data layer without re-implementing any of their query processing.

### 1.1 Design Principles (v3, Immutable)

1. **Compute is delegated.** Oikome never implements scan, join, aggregate, or index algorithms. If an engine cannot do something, we pick or configure an engine — we do not write an operator.
2. **Single Write Master per Space** (unchanged). Write delegation is explicit via UCAN.
3. **Space is the sovereignty unit; Location is the physical unit.** They are decoupled. Authorization, engine selection and sync policy attach to the Space; format and residency attach to the Location.
4. **Immutable files are the only thing that crosses the network.** P2P synchronization exchanges Iceberg snapshots (Parquet + manifests) and Lance versions (data files + manifest). Transactional state never replicates live; it is *published* as snapshots.
5. **Deny by default** (unchanged). No peer relationship → no catalog visibility → no query possibility. Unauthorized tables "do not exist". A public share (§13.4) is the owner's explicit grant to anyone; without one, a stranger sees nothing.
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
│     Compute sharing: tasks run with Oikome's own methods (UCAN)       │
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
                                     // | InPlace { place, path } (files viewed where they are, §7.7)
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
| In place | An `InPlace` Location is read-only and holds no data of its own. Its files stay where they are, and its index is a cache on the node (§7.7). |

### 3.3 Node Types

Three node types: a **Full** node runs where data is kept for good (a NAS, a server, an always-on computer), an **Edge** node runs on a desktop or laptop, and a **Browser** node runs in a web page (§10). The node type does not change *what engine code ships*; it changes *which `QueryEngine` implementations are registered*.

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

DuckDB and SQLite need real file paths, and DuckDB reads an Iceberg table as a list of local Parquet files. A Space on a storage is therefore never read in place: each node keeps a local copy of a whole snapshot (§7.5). The per-file `materialize()` cache planned earlier was dropped; a size-bounded cache only helps once an engine reads part of a table. The Iceberg `Storage` adapter in `oikome-vfs` is still unused. All of this is about Spaces kept on a storage. The owner's own files on a storage are viewed where they are (§7.7).

### 7.5 Storages: a NAS First, Cloud Drives Step by Step

A storage is a place other than the node's disk where a Space is kept: a NAS at home (3.4a), reached over WebDAV (Synology, QNAP, Nextcloud and most others) or mounted as a folder (SMB, NFS), and cloud drives through the folder their desktop client syncs (3.4b: iCloud Drive, Dropbox, OneDrive, Google Drive). A drive provider can read whatever is uploaded, so a drive is always encrypted on the client; a NAS may be. Drives over their API (3.4c), Baidu Netdisk (3.4d) and object storage (3.4e) follow.

A storage has two roles. It keeps Spaces, as this section describes. Its other files, the ones the owner put there, can also be viewed in place (§7.7), and it is a target for Save To (§7.8).

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
| Transactional Space (SQLite file) | **No** — rejected at `CREATE SPACE`; a published copy can go there (below) | SQLite's WAL needs file locks and shared memory on a local filesystem |
| Node registry (`.oikome/`), identity key, storage credentials and keys | No | They belong to one node |

**A collection a source fills.** Connectors write a transactional Space (§11), so its working copy always stays on the node's disk. A share mounted over SMB or NFS does not change this: it is still a network filesystem, where SQLite's locks and shared memory are not reliable. The owner may still keep such a collection on a storage. The node then publishes the Space after every pull (§7.3) and pushes the published snapshot to the storage like any analytical Space, so the storage holds a second, read-only copy that can be a few minutes behind the working copy. The apps put these collections on Local by default and ask before any other place (§13.4). Not built yet: `add_source` always creates the Space on the node's disk.

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

**Drives over their API (3.4c).** For nodes without a desktop client, Google Drive and Dropbox go through opendal's `gdrive` and `dropbox` services. Files are encrypted as above.

- **opendal is upgraded first,** from 0.52 to the current release (0.59). 3.4d's service needs it too.
- **Sign-in** is the OAuth authorization code flow with PKCE. The browser returns to `http://127.0.0.1:53917/`, a fixed port that the OAuth client registers. A node without a browser prints the sign-in link and takes back the address the browser was sent to, pasted in.
- **The OAuth client is the owner's own.** Its client ID and secret are entered once with the storage and kept like a password. None is built into Oikome.
- **Tokens.** The refresh token is kept like a password. The node renews the access token before it expires and revokes the token when the storage is removed.
- **Least permission.** Keeping Spaces needs access only to the files Oikome creates: Google's `drive.file`, or a Dropbox app limited to its own folder. Viewing the drive's other files in place (§7.7) needs read access to the whole drive (Google's `drive.readonly`, a Dropbox app with "Full Dropbox" access). It is asked for only when the owner turns that on.
- **Large files.** opendal's `gdrive` and `dropbox` writers send a file in one request. Google advises this only up to 5 MB, and Dropbox accepts at most 150 MB.
  - Oikome adds Google's resumable uploads and Dropbox's upload sessions to these two writers in its fork of apache/opendal. Oikome uses them as a git dependency, like 3.4d.
  - They are contributed upstream, an issue first and a pull request once they have run against real accounts.
  - Keeping a large file as parts on the drive was turned down: every reader would have to put the parts back together, and other opendal users gain nothing.
- **OneDrive stays on its sync folder.** opendal's `onedrive` (0.52) addresses files by path under the drive's root, which needs read and write access to the whole drive (`Files.ReadWrite`). This is looked at again after the upgrade.

**Baidu Netdisk (3.4d).** Baidu Netdisk is reached through its Open Platform (the xpan API). opendal has no service for it.

- **The service.** A `baidu-netdisk` service is written in Oikome's fork of apache/opendal, shaped like `aliyun-drive`. Oikome uses it as a git dependency. It is proposed upstream (apache/opendal#8481) after it has run against a real account.
- **Sign-in** works as above (OAuth authorization code; the platform also offers device codes).
- **One folder.** An Open Platform app can write only under `/apps/<app name>/`. The app is registered as `Oikome`, so a storage's folders are under `/apps/Oikome/`. Viewing in place needs the app to read the owner's other folders as well. Whether it can is still to be confirmed.
- **Uploads.** An upload is precreate, then one call per block, then create. Precreate needs the MD5 of every block up front. Blocks are 4 MB for free accounts, and members get larger blocks and files.
  - opendal's writer streams, so it does not know these hashes in advance.
  - Oikome uploads only local files. If the service cannot stream, `RemoteStore` hashes the file first and uploads it itself, as it does for WebDAV.
- **Limits.** Downloads are slow without a Super membership, and frequent calls are refused (errno 31034), which the node retries with backoff. Baidu Netdisk therefore suits Save To and backups better than Spaces that other nodes read often.

**Object storage (3.4e).** S3-compatible services go through opendal's `s3`: AWS S3, Cloudflare R2, Backblaze B2, MinIO, and Aliyun OSS and Tencent COS through their S3-compatible endpoints.

- **Keys.** The access key and secret are entered once and kept like a password.
- **Encrypted by default,** since the provider can read what is uploaded. A service the owner runs, such as MinIO at home, may turn encryption off, like a NAS.
- **Large files.** Multipart uploads and range reads suit large files such as video. Object storage also suits public snapshots (§13.4).
- **One writer per folder,** claimed as on WebDAV.

Phases that need accounts (3.4c–3.4e, drives in 3.9, flomo's MCP from 3.5) are written first. Once all of them are written, they are run against real accounts together. MinIO covers 3.4e in CI without an account.

Not done yet: drives over their API (3.4c), Baidu Netdisk (3.4d), object storage (3.4e), the keychain on Linux desktops, changing a storage's key, a size-bounded cache (every node holds the current snapshot of each Space it reads in full), restoring a writer from its storage, and `PUBLISH` into a Space on a storage.

### 7.6 Lance Locations

Lance is a columnar format with versions, built for vector search, full-text search and large objects. A Lance Location holds one Lance dataset per table. It is for data that is searched or read row by row rather than aggregated: note text and embeddings, documents, thumbnails, attachments. Originals that live elsewhere are referenced rather than copied (see the rules below).

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
- **Originals are referenced, not copied.** A photo, recording or document that lives in a place (§7.7) is named by its place, path and BLAKE3. Lance keeps what is made from it: text, embeddings and thumbnails. A Space made by Save To keeps its originals as files beside its tables (§7.8).

Not done yet: reading a pinned version (waits for the extension), `PUBLISH` (§7.3) into a Lance Space, peers searching over query shipping, and a search spanning several Spaces.

### 7.7 Viewing in Place

Files the owner already has, such as photos on a NAS, documents on a drive or a folder of CSV files, are browsed, previewed and queried where they are. Nothing is copied into a Space until the owner saves it somewhere (§7.8).

```sql
CREATE SPACE finance WITH PLACE = 'nas', PATH = 'Finance';            -- read-only
SELECT * FROM finance."2025";                                         -- Finance/2025.csv, read where it is
CREATE SPACE lisbon WITH PLACE = 'local', PATH = '/Users/me/Pictures/Lisbon';
SELECT name, taken FROM lisbon.files WHERE kind = 'image';            -- the index (below)
REFRESH SPACE lisbon;                                                 -- read it again now
```

- **Places.** A place is a folder on this computer (`local`) or on any storage (§7.5).
  - On this computer the folder is given by its full path, which must exist and be a folder, not a hidden one (`~/.ssh`), and not inside the data center's own `.oikome`. Mounted shares and the sync folders of cloud drives (3.4b) are folders on this computer.
  - On a NAS (WebDAV) the folder is a path within the storage. Its names are the owner's own (spaces, accents, other scripts); only `.`, `..`, empty segments, backslashes and control characters are refused. Oikome only reads there.
  - Oikome's own folders on a storage, the ones that keep Spaces, are skipped.
  - On a drive reached over its API (3.4c), viewing in place needs read access to the whole drive (§7.5). It is asked for when the first in-place Space is made on that storage.
- **Read-only.** An in-place Space is bound to an `InPlace` Location (§3.2). Oikome never writes, renames or deletes there, and DML and DDL are refused. Files change only through the owner's other apps.
- **Tables.** Each CSV, Parquet or JSON file at the top of the folder is a table named after the file, in lowercase letters, digits and `_` (`2025.csv` is `t_2025`), read by DuckDB's `read_csv_auto`, `read_parquet` or `read_json_auto` each time it is read. `captions.csv` names the photos beside it and is no table. A file whose path holds a quote, a backslash or a control character is listed in `files` but makes no table, since its path goes into DuckDB's SQL as a string. `files` is the index; its key is `path`.
- **The index is a cache.** For each in-place Space the node keeps an index in `.oikome/index/<location>/`.
  - `index.sqlite` holds a row per file (path, folder, name, kind, size, modified time; a photo's title, place, time taken and credit from `captions.csv` beside it, as the folder sources read them; a note's date from its front matter and its text), a thumbnail per photo (JPEG, 512 px on its longest edge, turned as its camera recorded), and an FTS5 trigram index of names, titles and text, kept by triggers, so search finds Chinese as the sources' index does (§8). Later it can hold the embeddings and descriptions Vane makes.
  - Its rows are also written as `files-<n>.parquet` after each scan that changed something, which DuckDB reads as the Space's `files` table (DuckDB's SQLite reader is an extension a node may not have).
  - It was planned as a Lance dataset; SQLite was chosen in 3.9 so that it is searched like the sources' collections and keeps thumbnails in one file. Being a cache, it can move to Lance with Vane.
  - Deleting it loses nothing but time: reading the place again rebuilds it.
  - It is not a copy in the sense of §7.8. The owner did not choose to make it, and originals never enter it.
- **Keeping up.** While it serves, the node scans each place again every minute, reading at most 20 s of new files each time, and in full on `REFRESH SPACE` (`refresh_space` in the apps). Size and modified time tell what changed; a changed `captions.csv` has its photos read again. Making an in-place Space reads its first files at once (up to 10 s) and the rest in the background. A drive's own change list can replace the scan later.
- **Reading files.**
  - Files on this computer and on mounted folders are read in place by DuckDB.
  - Files on WebDAV, drives and object storage are first fetched through the VFS into a size-bounded cache (`.oikome/cache/places/<location>/`, 2 GiB per place, the files read least recently going first), keyed by place, path, size and modified time, because DuckDB reads only local files here.
  - Photos for the apps are served at `/files/thumb/<collection>/<path>` (from the index) and `/files/original/<collection>/<path>` (from the place, with range reads), to the owner's session only and to no other site's page, and only for a photo the index lists.
  - Later, a DuckDB file system over the VFS could let engines read only the parts they need.
- **Offline.** When a place cannot be reached, the index still answers: `files`, search and thumbnails are there, the originals are not. Its tables are bound as views that fail with "… can't be reached right now", and opening an original says the same. Whether it can be reached is asked at most every 15 s, and on every scan.
- **Sharing in place.** An in-place Space is shared with grants and policies like any Space, but only by query shipping: the node reads the files when the peer queries. A share that would let them keep a copy is refused. To give a contact a copy, save it into a Space of its own (§7.8, §7.9). A principal's search finds files through the index and reads each again as itself (as for sources, §13.4); an AI assistant reading a photo gets its thumbnail, and a card plugin gets a collection's photos in the shape the photos folder source had, thumbnails as the pictures.
- **Apps.** An app's data cannot be read where it is, so its connector pulls it into a Space on the node (§11). That Space plays the part of the index: the node keeps it and refreshes it on an interval, and the owner never chose it as a copy.
- **Folder sources.** In 3.9, the `photos-folder` and `notes-folder` sources (§11) became in-place Spaces. When a node first starts on 3.9 it turns each into an in-place Space of the same name over the same folder, once: the source goes, its SQLite copy is deleted (the files stay where they are), and the collection's whole-Space grants are given again, so its shares keep working. Views, column grants and row policies written for the old `photos` and `notes` tables can't apply to `files` and go with them. The examples' photos and notes are viewed in place from the start; their tables are still read in by CSV sources, which merge repeated imports by their key.
- **Limits.** Hidden files and folders and links are skipped; at most 200,000 files, 32 folders deep, are indexed. Photos over 64 MiB and notes over 1 MiB are listed but not read for text or thumbnails.

### 7.8 Save To

A copy is made only when the owner asks for one. Save To takes a selection (files, a folder, a query's rows, an app's records) and writes it to a target the owner picks.

| | Files | A new Space |
|---|---|---|
| What lands | The files themselves, with their real names and formats, in folders made by the rules | Tables, with the originals as files beside them |
| Read by | Finder, Photos and other apps; Oikome views them in place (§7.7) | Oikome; shared and subscribed like any Space (§7.9) |
| Encryption | Encrypted by default on a drive or object storage, and then readable only through Oikome. The owner may turn it off so that other apps can open the files, and is told that the provider can then read them | As the storage's (§7.5) |

- **Rules.** A folder rule and a name rule are built from the item's own fields, for example `{taken:yyyy}/{taken:MM}` and `{taken:yyyy-MM-dd} {title}`. A file that would land on a name already taken gets a number.
- **Once or continuing.**
  - Once writes the selection as it is now: a fixed version.
  - A continuing rule is kept on the node and runs on its interval while the node serves. New and changed items that match are written. Items removed at the source stay in the target unless the rule says otherwise.
  - A continuing rule into a Space commits a snapshot each run, so subscribers receive new versions.
- **Only its own files.** The node records what each rule wrote (path and BLAKE3) and changes only those. It never overwrites or deletes a file it did not write.
- **Not Space storage.** A Space kept on a drive (§7.5) is sealed objects with hashed names, which only Oikome can read. Save To's files belong to the owner and are in plain sight. A Space made by Save To is kept like any other Space, on this computer or on a storage.
- **Permissions.** Saving into a drive needs only the least permission of §7.5, wherever the files come from.
- **Editing happens elsewhere.** Oikome views, picks, saves and shares. Photos are edited in other software, and the edited files are viewed in place or saved like any others.
- **Configuration, not SQL.** Rules are node configuration, like sources (§11.4): `add_save_rule`, `run_save_rule` and `remove_save_rule` in the API, and `oikome save` on the command line. There is no SQL statement for them.
- **`PUBLISH SPACE` (§7.3)** is the first case of Save To: from a transactional Space into a read-only analytical Space, once or on an interval.

### 7.9 Sharing Saved Spaces and Portfolios

There is no phase of its own for this (once 3.11): sharing a saved Space with contacts needs nothing new, portfolios come with Save To (3.10), and public sharing, with photos' location data removed, with public shares (3.8).

- **To contacts.** A Space made by Save To is shared and revoked with the existing whole-Space grants (§6, §13.4). Unlike an in-place Space or a view, it no longer depends on its source: a contact who subscribes keeps reading it when the source folder is offline, changed or deleted. The Space can also be kept on a NAS or object storage rather than on the laptop the source is on.
- **Portfolios.** A portfolio is a way of showing a Space: the order of its items, a cover, a title and captions. These are kept with the Space and sent with its snapshots, so subscribers see the same portfolio. The apps or a dashboard plugin show it, and the data underneath is the saved Space.
- **To anyone.** A saved Space can also be a public share (§13.4, 3.8). It does not depend on its source, so it is what a public share or a public snapshot is best made from. Photos in a public share lose their location data by default.

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

**Indexes the node keeps (M3a, `oikome-node::search`).** Each table a source writes gets a full-text index without anyone asking: a hidden FTS5 table `_oikome_fts_<table>` over the table's text columns, reading its rows (external content) through their `rowid`, with the `trigram remove_diacritics 1` tokenizer, so any three or more characters match inside words, case and accents aside, and words of languages written without spaces (Chinese, Japanese) are found. Triggers on the table keep it in step, so every write, a pull or the owner's own SQL, changes it in the same transaction; a pull makes it (and rebuilds it from the rows) before writing when it is missing or the text columns changed. Its tables and triggers are hidden like any `_oikome_*` name. The owner's search (`search`, §13.4) goes one Space at a time: through the index where there is one, ranked by BM25, with terms shorter than three characters matched by `LIKE`; any other collection is read up to 10,000 rows per table and matched by the node. A collection shared with the owner is not searched; it is searched where it is kept. What the owner types is never SQL: each term is an FTS5 string with its quotes doubled or an escaped `LIKE` pattern, bound as a parameter. The table's `rowid` keys the index, so a `VACUUM` that renumbered rows would call for a rebuild; the node never runs one.

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

Each node is a DID (`did:key`, Ed25519). Access is granted by UCANs that a node issues, delegates and revokes; libp2p finds nodes and Arrow Flight carries the data. Two clarifications for v3:

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
| `DoExchange` (compute request, then the input) | `compute/<method>` | Run a task over the input the caller sends; the result streams back (§12) |
| `DoAction("oikome.compute_methods")` | `compute/<method>` | The compute methods offered here that the caller's grant covers, with its limits |

`space/read` includes `space/query`. Data abilities (`space/*`) and compute abilities (`compute/*`) never cover each other. A capability may carry limits in the UCAN `nb` field (§12.3); a delegated token can only make them stricter.

**Transport (3.1c).** Nodes speak TLS by default. Each node presents a self-signed certificate for its own Ed25519 key, generated at start (`NodeIdentity::tls_identity`, rcgen); nothing is stored. A client knows the DID it expects, the root issuer of the token it presents, and accepts the server only if the certificate carries that DID's key. rustls (ring backend) checks that the handshake is signed with it. There is no CA and no host-name check. Plaintext `http://` is refused unless the node runs with `--allow-http` (`NodeConfig::allow_http`); `serve --plaintext` needs it too. Requests are signed either way.

**Discovery (3.1c, `oikome-discovery`).** libp2p finds nodes; data stays on Arrow Flight (across networks Flight runs over a libp2p stream, below).

- The libp2p identity is the DID key, so a `PeerId` and a DID name the same public key. The swarm runs tcp, noise, yamux, mDNS, identify and `/oikome/endpoint/1`.
- After connecting, a node asks the other for its Flight scheme and port. The host is the address the connection came from. The result, `DID → scheme://ip:port`, goes into the Peers table (§3.4).
- mDNS finds nodes on the local network; `serve --dial <multiaddr>` connects to a known one.
- `CREATE SPACE ... WITH TOKEN = '...'` without `PEER` resolves the owner (the token's root issuer) in the Peers table, again on every connection, so an owner that moved is followed.
- The Flight endpoint is recorded only for a peer connected from the same network (a private, link-local or loopback address); a peer on another network is reached as below.
- mDNS needs multicast, and on macOS the application firewall must let `oikome` receive it.

**Across networks (3.12, `oikome-discovery`).** No server of the project's, and none the owner runs, relays anything; nothing is published to a DHT or DNS. A node is reached from another network in one of two ways, over its one peer-to-peer port:

- **The router opens the port** (`portmap`): while the owner allowed something that needs it (below), the node asks its router to map the libp2p TCP port, by UPnP IGD (rupnp) and else PCP / NAT-PMP at the default gateway (crab_nat, gateway from netdev). The lease is an hour, renewed at half; the port is let go when nothing needs it and when the node stops. A router whose outside address is shared (100.64.0.0/10, or a private one: a second router) can't be reached, and says so. A node with an address of its own on the internet is reached directly without asking. IPv6: the node listens on the same port on `::` and gives its global addresses; asking the router's firewall to open it (PCP) is left for later.
- **A contact introduces them** (circuit relay v2 and DCUtR): when the router won't, the node keeps a place (a reservation) at up to three contacts that are reached directly. Another contact connects through one of them, and the two punch through their routers to a direct connection. The contact only passes the introduction: 16 KiB and 30 seconds per circuit, only between its own contacts, only while it is reached directly, and only while its owner lets it (Settings › Network, on unless turned off). Flight never runs on a relayed connection: a dial waits for the direct one, and relayed connections to a peer are closed once a direct one exists.
- **Only contacts get in from outside** (`gate::Gate`): a connection from the internet, or through another node, is turned away unless its key is a contact's. Connections from the local network are let in as before.
- **Addresses** travel in invites, as a libp2p peer record signed with the node's key (`at` in the code; checked against the inviter's DID when the invite is accepted), and between contacts over identify on authenticated connections, into the Peers table (`addrs`). When its outside address changes, a node connects to its contacts so they learn it. The libp2p port is kept in `.oikome/p2p.json`, so contacts find the node at the same port after a restart.
- **Flight over libp2p** (`/oikome/flight/1`, libp2p-stream): the serving process answers Flight on streams peers open as on TCP (`serve_with_peers`), with the same TLS pinned to the DID on top, so UCANs, signed requests and the access log are unchanged. A `PeerClient` tries the peer's Flight endpoint on a shared network first (when discovery saw it in the last ten minutes, or a Space names one), then the peer-to-peer stream. Other processes of the node (`oikome sql`) borrow the serving process's connections through `.oikome/peers.sock`, which only the owner can open.
- **When it is wanted** (`oikome-api::network`): only while a share or a lent computer the owner allowed is going on. Sharing a collection or lending this computer asks first (Deny / Allow, "Don't ask again"); an answer without "Don't ask again" is kept for that share or lending alone (`reach_for` in `node.json`: `share:<id>`, `lend:<did>`) and ends with it; with it, the answer becomes the rule (`reach_policy`: `ask`, `always`, `never`). The web pages and the assistants' endpoint stay on loopback either way.
- When neither works (both homes behind a provider's shared address, no UPnP, no common contact), Settings › Network says why and what to do: turn on UPnP, or connect through the owner's own VPN (Tailscale, WireGuard).

`docker/nat/compose.yml` puts two nodes in two homes behind Linux NAT routers (iptables MASQUERADE; miniupnpd on one, started halfway) and a contact on an internet segment; `tests/e2e/docker_nat.sh` checks that the two meet at the contact and read directly, that they can't once the contact stops helping, and that the opened port works without it. The same paths run in process in `crates/oikome-server/tests/server/across.rs`.

**Running nodes (3.1d).** `docker/oikome.Dockerfile` builds the node image: the `oikome` binary and the prebuilt libduckdb, with the node's data in `/data`. `docker/compose.yml` runs three nodes on one bridge network:
- `node-a` owns the data;
- `node-b` reaches it after dialing `node-a`'s libp2p address;
- `node-c` is given no address and has to find `node-a` by mDNS.

`tests/e2e/docker_nodes.sh` drives them, checking each step against its expected result: discovery, a UCAN-gated federated query over TLS with no `PEER`, a subscription, then revocation stopping both the query and refreshes while the copy stays readable. `.github/workflows/docker-e2e.yml` runs it on GitHub Actions on every push to `feat/**` or `ci/**`.

**Access log (M1b, `oikome-node::access_log`).** A serving node records every request a peer or a browser sends it, so the owner can see who read what and when.
- One row per call of each entry point (`get_schema`, `do_get`, `do_exchange`, the actions, `serve_envelope`): time, caller DID, method, Spaces, the id of the UCAN that let it in, outcome (`allowed`, `denied`, `failed`) and the reason. The reason is never sent to the caller.
- `fetch_file` is recorded only when refused; the subscription is recorded by its `oikome.snapshot` call.
- Rows live in `.oikome/access_log.sqlite`. Requests never wait for the log: a writer thread takes entries from a bounded channel (10,000), and entries are dropped (and counted) when it falls behind.
- Failed authentications carry no caller and can come from anyone who reaches the port, so they are merged into one row per method and minute, under a cap of their own (10,000 rows). The log keeps 90 days, at most 100,000 rows; the setting up of the data center (below) outlasts the 90 days.
- The owner's own first steps are kept with the requests, as the owner's (M2, written by `oikome-api`): setting up the data center, adding the examples and saving the recovery key (`owner_setup`, `owner_examples`, `owner_key`). A data center set up before has them added once, at the times its settings kept.
- When a contact or a plugin is removed, the name it had is kept beside the rows (the `names` table) as long as the rows are, so its lines still say who it was.
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

Connectors are for apps, whose data cannot be read where it is. Folders and storages are viewed in place instead (§7.7): the folder sources below (`photos-folder`, `notes-folder`) were replaced by folders viewed in place in 3.9, and a node turns its own into them when it first starts on 3.9. They stay in `oikome-connect` for that, and the apps no longer add them.

### 11.2 Contract

`oikome-connect` holds the contract and the built-in connectors:

```rust
#[async_trait]
pub trait Connector: Send + Sync {
    fn kind(&self) -> &'static str;      // "csv", "flomo-export", "flomo-mcp"
    fn tables(&self) -> Vec<TableDef>;   // name, columns (INTEGER / REAL / TEXT / BOOLEAN), source key
    /// Rows changed since `cursor` (everything when `None`).
    async fn pull(&self, cursor: Option<&str>) -> Result<Pull, ConnectError>;
}

pub struct Pull {
    pub upserts: Vec<(String, Vec<Row>)>, // table → new or changed rows (a value per column)
    pub deletes: Vec<(String, Vec<Row>)>, // table → keys the source reports deleted
    pub cursor: Option<String>,           // opaque to the node
    pub complete: bool,                   // full listing: rows not in it were deleted
}
```

Rows are plain values rather than Arrow streams: they go to SQLite one parameterized upsert at a time, and personal sources are small.

| Connector | Reads | Key | Pulls |
|-----------|-------|-----|-------|
| `csv` | A CSV file with a header; columns and types fixed when the source is added (`--key` names the identifying columns) | The key columns | Whole file, `complete`; an unchanged file (same BLAKE3) is skipped |
| `flomo-export` | `flomo_backup.zip`, its HTML or the unpacked folder; `.memo` → `.time`, `.content` | Creation time and place among memos of that second (the export has no ids) | Whole export, `complete`; an export without memos is refused rather than read as "delete everything" |
| `flomo-mcp` | flomo's MCP server (MAX, personal token): finds the tool that lists or searches memos (or the one named), pages through it, matches field names loosely | The memo id | Not `complete` (a search may not list every memo), so it never deletes. Written, not yet run against flomo: the module is the one place to adjust once a MAX account is at hand |
| `photos-folder` | The images (JPEG, PNG, GIF, WebP) of one folder, not its subfolders, hidden files or links; an optional `captions.csv` (`file`, `title`, `place`, `taken`, `credit`) names them. Table `photos` (`file`, `title`, `place`, `taken`, `photo` BLOB, `credit`) | The file name | Whole folder, `complete`; unchanged names, sizes and times are not read again |
| `notes-folder` | The Markdown and text files of one folder, such as a notes vault; `written` is the front matter's `date` (or `written`, `created`), else the file's time | The file name | Whole folder, `complete`, as above |

### 11.3 Where the Data Lands

- Each source writes to its own **transactional** Space of the same name, e.g. `flomo.memos`. Sources edit and delete records, so loading is an upsert by source key, which needs SQLite; analytical Spaces are append-only.
- A pull is applied in one SQLite transaction on a private connection: `INSERT … ON CONFLICT (key) DO UPDATE … WHERE` a value differs (so unchanged rows are not touched or counted), deletions, and for a `complete` pull the removal of rows it no longer lists, then the cursor in the Space's hidden `_oikome_source` table. A failed pull rolls back: data and cursor stay as they were, and the next run starts from the same place.
- Tables named `_oikome_*` are bookkeeping: the SQLite engine leaves them out of table listings, so `PUBLISH`, grants and federation never see them.
- Sharing uses the existing path: `PUBLISH SPACE flomo` (§7.3), then grants and policies (§6).
- Reshaping is done with views (§6.1), not inside the connector. Connectors only move data (principle 1).

### 11.4 Running

```sh
oikome source add flomo --flomo-export ~/Downloads/flomo_backup.zip       # pulls once
oikome source add flomo_live --flomo-mcp --token-stdin --every 30m       # flomo MAX
oikome source add bank --csv statement.csv --key txn_id --table transactions
oikome source pull flomo
oikome source list
oikome source remove flomo     # the Space and its data stay
```

- Sources are registered on the node (`.oikome/sources.json`: connector settings and interval, no credentials), from the command line and from the apps' Sources screen (M2), through the same node functions (`add_source`, `pull_source`, `remove_source`). There is no SQL statement for them.
- A source can be paused (`paused`): it is not pulled on its interval, and still is when asked. Its interval is the owner's to set: a minute (the default for the apps' sources), 5 or 30 minutes, an hour, a day, or only when asked.
- Each pull is kept for 30 days in the Space's hidden `_oikome_source_runs` table: when, how many rows it added or changed and removed, or why it failed. A successful pull's row is written in the same transaction as its data and cursor. A pull like the last one (nothing new again, or the same failure again) moves that row's time on and counts it, so a source read every minute does not fill the list.
- A connector runs only on the node that owns its target Space (principle 2): on demand, or on its interval while the node serves (`oikome serve` checks every minute, `--source-sync`).
- A token is entered once (standard input now, the apps later) and kept by the node like storage credentials (§7.5): OS keychain, else a private file; `OIKOME_SOURCE_<NAME>_TOKEN` overrides it. Tokens never enter Space data, the registry, snapshots, or anything sent to a peer.
- A source's collection is a transactional Space on the node's disk. When the owner keeps it on a storage, the copy there is published after each pull (§7.5).
- Connectors are in-process Rust code. Third-party connectors are not in M4: they come later, one plugin at a time.

---

## 12. Compute Sharing

Compute is a machine's CPU and GPU time. Compute sharing is how a node borrows another individual's machine, or lends its own: a requester sends a task and its input, the executing node runs it with one of Oikome's own compute methods and sends the result back. It uses the same trust mechanism as data exchange, with a separate authorization.

### 12.1 Compute Sharing vs Data Exchange

| | Data exchange | Compute sharing |
|---|---|---|
| Trust | DID identities, peer relationships, deny-by-default, UCAN revocation (§9) | Same |
| Authorization | `space/query`, `space/read`, grants and policies, issued by the data owner (§5.3, §6) | `compute/<method>` with limits, issued by the owner of the executing node (§12.3) |
| What moves | Query results or snapshots of the owner's Spaces | A task and its input to the executing node; the result back |
| Who computes | The data owner's node, over its own data | The executing node, over the input it was sent |
| What the other side sees | Only what grants and policies allow | The executing node sees the task input, and none of the requester's Spaces |

The two authorizations never imply each other: a peer I exchange data with cannot run tasks on my machine, and a peer that runs my tasks gains no access to my Spaces. Running SQL on a peer's Space (query shipping, §5.3) is data exchange, even though the peer's node does the computing. A browser node sending heavy queries to a Full Node of the same owner (§10) is the owner using their own resources and needs no compute grant.

### 12.2 Compute Methods

- **Tasks run only with Oikome's own compute methods**, the ones a node already uses for its own data. They are the standard of what a task can be. Oikome does not forward tasks to other services and does not run code a peer sends.
- **`sql` (the first method)** runs one DuckDB query over the table `input`, which holds the requester's input. DuckDB's aggregates, window functions and vector distance functions are all available. Each task gets a DuckDB of its own (`oikome-engine-duckdb::isolated`):
  - in memory, with external access disabled (no files, no network);
  - extensions neither loaded nor installed;
  - its configuration locked once `input` is loaded, with the memory limit and the thread count set;
  - interrupted when its wall time passes, and its result capped.
  So a task reads only its input: not the executing node's Spaces, not its files.
- **Later methods** use the same grants, scheduling and transport with another of Oikome's capabilities over the input, such as full-text or vector search or a graph query. Only `sql` exists now.
- **The executing node's owner offers methods**: `oikome compute offer|withdraw <method>`, kept in `.oikome/compute.json`. None is offered by default.
- **Apps, not users, write the tasks.** An app calls the node function `compute_on_peer` for what it needs, e.g. a phone dashboard hands a large aggregation to the computer at home; users never see the SQL. `oikome compute run` exists for development and tests.
- **Borrowed compute in the apps (M4)** sends only what the lender may read already. Home's daily-totals card is worked out by the node (`daily_totals`), and goes to a computer lent here only when all of these hold; otherwise this computer works it out with the same task SQL in the same isolated DuckDB (`run_task_here`), and the card says why when one lent here is in use:
  - the owner turned that computer on for use (`use_borrowed`, kept with it in `borrowed.json`);
  - the task is big: 100,000 rows or more;
  - the collection is shared with the lender, and the share is active;
  - read in a session acting as the lender, the input is the whole of it: a row policy that leaves them part of it keeps the task here. The input is that read (`compute_batches_on_peer`), so their grants decide what goes.
  A task sent that the lender refuses (paused, stopped), can't take (a limit, the quota) or can't be reached for is worked out here instead. An answer worked out there is kept for 10 minutes, unless the rows, the computer in use or the share change, so looking at Home again doesn't spend the lender's tasks. Each task sent is a line of the owner's Activity (`owner_computed_on`, merged by minute), from which the computer's dialog counts today's tasks and lists the latest.
- **The requester chooses the input** for each task: a query result, never a whole Space. The executing node sees it while the task runs and deletion afterwards cannot be verified, so what to send is the requester's decision.

### 12.3 Grants and Limits

- A compute grant is a UCAN with the ability `compute/<method>` (`compute/*` for every method offered) on the resource `node://<executing node's DID>`.
- Its limits are in the capability's `nb` field: wall time, input bytes, output bytes and memory (memory applies to `sql`). Along a delegation chain the strictest value of each applies, so a delegated token can only tighten them. A grant without limits gets the defaults: 30 s, 64 MiB in, 64 MiB out, 512 MiB of memory.
- Issued with `ISSUE UCAN ON COMPUTE 'sql' TO '<did>' LIMITS (time = '30s', input = '100MB', output = '100MB', memory = '512MB') EXPIRES IN 7d`. Tokens issued before 3.6 have no `nb` and are unchanged.

### 12.4 Quotas and Scheduling

- **Per-peer quotas**, set by the executing node's owner: `oikome compute quota <did> --concurrent 1 --per-hour 60`, kept in `compute.json`. A peer without one gets 1 task at once and 30 per hour.
- The node runs at most as many tasks at once as it has CPU cores. Up to 8 of a peer's tasks wait for their turn; more, or more than its hourly count, are refused.
- The grant is checked when a task arrives and again when its turn comes, so revoking the grant or withdrawing the method drops a waiting task. A running task ends at its wall time.

### 12.5 Protocol

- **`DoExchange`** carries one task. The first message's descriptor holds the signed request (`ComputeRequest`: the method and the task); the input batches follow, counted as they arrive and refused once over the input limit; the result streams back.
- **`DoAction("oikome.compute_methods")`** lists the methods offered here that the caller's grant covers, with its limits, for the requester's app to show.
- A refusal says only `permission denied`; the reason goes to the access log (§9). A limit reached is reported as such, with the limit.
- Browser nodes neither run nor send tasks in this version.

Open:
- Whether a WASM sandbox should later allow user-supplied task code.
- Reciprocal credit between peers instead of plain quotas.

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
├── oikome-network         Peer protocol, PeerClient, FlightEngine, DID-pinned TLS; compute tasks   [3.1]
├── oikome-discovery       libp2p discovery (mDNS, endpoint exchange) → Peers table                 [3.1]
├── oikome-server          Arrow Flight service (queries, snapshots, files), oikome binary         [3.1]
├── oikome-protocol        Peer protocol types, request digest, WebRTC record and frames            [3.2]
├── oikome-webrtc          WebRTC endpoint for browsers (str0m): sessions, DTLS certificate         [3.2]
├── oikome-wasm            Browser node: BrowserNode, HostEngine (duckdb-wasm), RemoteEngine, JS API [3.2]
├── oikome-connect         Connector trait, built-in connectors (CSV, flomo export, flomo MCP)     [3.5]
├── oikome-api             API for clients, plugins and MCP; caller is Owner or Principal (§13.4)   [M1b]
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
| Add | `html5ever` | **0.40** (its tokenizer only: `scraper`'s CSS selectors are MPL-2.0) | MIT / Apache-2.0 |
| Add | `zip` | **9** (reading deflated entries; already in the tree through libduckdb-sys) | MIT |
| Add | `arrow-csv` | **58** (with the Arrow line) | Apache-2.0 |
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
├── crates/           Rust core; adds oikome-api [M1b]; MCP is served by oikome-server [M3]
├── apps/             one thin shell per client: entry, platform glue, packaging; no screens
│   ├── web/          the web app, served by `oikome serve --web --web-root apps/web/dist`          [M1c ✅]
│   ├── desktop/      Tauri 2; runs a Full Node in process                                           [later]
│   │   └── src-tauri/
│   └── mobile/       Tauri 2; reaches the owner's node over WebRTC, as a phone browser does         [later]
├── packages/         TypeScript shared by every client
│   ├── screens/      every screen and feature, desktop and phone layouts, UI strings                [M1c ✅]
│   ├── ui/           design system: tokens and components                                           [M1c ✅]
│   ├── client/       NodeClient interface, data hooks, HTTP and browser-node implementations        [M1b]
│   ├── node/         browser-node runtime: host, store, transport, wasm build                       [M1a ✅]
│   ├── api/          TypeScript types generated from oikome-api                                     [M1b]
│   └── plugin-sdk/   the bridge every plugin page loads first (`sdk.js`) and its guide             [M2]
├── plugins/          Oikome's own plugins, built into the node: spending-by-category, photo-memories [M2]
├── docs/             architecture.md (this document), prd.md (the apps' PRD), prototype/ (the clickable design prototype, its translations and design references)
├── design/           DESIGN.md, design.json                                                         [later]
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
 apps/web (browser)          apps/desktop (Tauri 2, later)      AI tools (MCP clients)
       │ HTTP                        │ in-process call                  │ MCP (stdio or HTTP)
 oikome-server --web                 │                   oikome-server /mcp (HTTP)  [M3]
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
| `list_tables`, `read_rows` | Tables and views, each with its row count and, for a table a source fills, its key columns (so the web app gives every photo and note an address and opens it with `read_row`); a page of rows (at most 1,000) as Arrow |
| `search`, `read_row` | The owner's search (M3a, §8): plain words, every one in a row's text; at most 100 hits, best first, across the owner's collections or one, each with its collection, table, primary key, the column it matched and an excerpt with the matches as UTF-16 ranges; collections not searched are listed (shared with the owner, or failed). `read_row` opens a hit of a collection a source fills by its key. The web app's ⌘K palette (and a collection's Search in here) searches as you type and groups hits by what their collection reads as (notes, photos, rows); a hit opens its item at `/collections/<name>?table=…&item=…`, one `item` per key column, so every item has an address of its own (M3b). On a collection's page a photo opens large and a note in full, with where it came from, where it is kept and who can see it, and ← → move to the one before or after, across pages (M3c) |
| `run_sql` | The owner's statements in one session (at most 256 KiB); the last result as Arrow (cut at 10,000 rows or about 32 MiB, flagged by `oikome-truncated: true`) or `{rowsAffected}` |
| `list_contacts`, `add_contact`, `rename_contact`, `remove_contact` | Names for the DIDs the owner shares with (`contacts.json`). Removing a contact revokes their shares and whole-Space grants |
| `share_space` | A whole-Space grant plus a UCAN (`space/query`, or `space/read` to allow copying an analytical Space) for 1 to 366 days; the token is returned once, alone and as an invite code. Only to contacts. `reach` carries the owner's answer to whether the contact may reach it from outside this network (§9, 3.12); without one nothing changes |
| `add_invite`, `list_invites`, `accept_invite`, `decline_invite` | Invites someone sent this node: to a collection, or to use a computer (a `compute/sql` grant, with the lender's limits in the code). The code is `oik1-` and base64url JSON: the token, the sharer's Flight address (left out when it only says the node listens on every address, so discovery finds it), the sharer's signed peer-to-peer addresses (`at`, 3.12; kept when accepted, checked against the sharer's DID) and the sharer's name; a token alone, or a code without `at`, is taken too. Adding one checks it is for this node, not by it, signed and not ended, and keeps it waiting in `.oikome/identity/invites.json` (owner-only, it holds tokens). Accepting creates the remote Space through `create_space` (no SQL is built from the code), named after the shared Space or numbered when taken, and adds the sharer as a contact under its name; a new share of a Space already added replaces it. Declining forgets it; nothing is sent back |
| `lending`, `set_lending` | Lending this computer (§12) to contacts: on or paused (offering `sql` or withdrawing it, grants kept), to whom, and how much each may use (the per-peer quota: 1 to 8 tasks at once, 1 to 1,000 an hour). Someone newly lent to gets a `compute/sql` grant for a year and an invite code, returned once; someone left out, or removed as a contact, has theirs revoked. `reach` as for `share_space`, for the people lent to |
| `network`, `set_network` | Being reached from outside this network (§9, 3.12): how (directly, through a contact, only here, the router refused, its address is shared), what for (the shares and lending allowed and still going), the rule (`ask`, `always`, `never`) and whether contacts may meet here; `set_network` changes the rule and helping |
| `list_borrowed`, `stop_borrowing`, `use_borrowed`, `borrowed_jobs` | Computers lent to this node, accepted from invites and kept in `.oikome/identity/borrowed.json` (owner-only). Each is asked, for at most 3 s, which methods it lends now: available, not lending now (paused there or stopped for this node; the lender doesn't say which), can't reach, or ended. `use_borrowed` turns one on for big summaries (§12.2); `borrowed_jobs` counts the tasks sent there since a time, lists the latest, and names the collections shared with its owner, the only ones that may go |
| `daily_totals` | A table's totals by day for Home: the last 14 days up to the latest with data. The day and amount columns must be the table's own; worked out here or on a computer lent here (§12.2), with where and, when one is in use and wasn't used, why |
| `list_shares`, `revoke_share` | Shares: issued UCANs whose capabilities all name one Space (tokens issued with SQL for several Spaces or `*` are not listed). Revoking the contact's last active share of a Space removes their grant on it |
| `list_access` | The access log (§9), newest first, by DID or Space. Each row says who it was as the owner knows them (`who`, `name`): you; a person, by their contact name, else the name they gave in an invite or with a computer they lend, else the one they had when removed; a plugin, by its name, also once removed; or unknown. A name a caller gives itself in a request is not used: anyone could claim one |

- The API never builds SQL from request fields: names are checked against the catalog and operations call the node directly. Only `run_sql` runs SQL, the owner's own.
- **MCP** (M3e) is served by the node itself, over HTTP: `POST /mcp` on the web API's loopback port (`oikome-server::mcp`), answered with plain JSON. It exposes an assistant's three operations only, as the tools `list_collections`, `search` and `read_row`, and never an Owner command.
  - Each request carries an assistant's token (`Authorization: Bearer oik_…`) and runs as that assistant; a request with an `Origin` (a web page) is refused with 403, one whose `Host` isn't this server earlier still, one with no valid token with 401; each assistant may send 120 requests a minute.
  - Both eras of the protocol on one endpoint: a request carrying per-request `_meta` is served as revision 2026-07-28, its `MCP-Protocol-Version`, `Mcp-Method` and `Mcp-Name` headers checked against its body (`HeaderMismatch` otherwise) and `server/discover` answered; `initialize` and what follows it as 2025-11-25, 2025-06-18 or 2025-03-26, without sessions. GET and DELETE are 405: there is no stream.
  - A row read comes as fields (text, numbers, dates) with a photo in it as an image (up to 4 MB). Tool failures an assistant can act on (a collection it may not read, a bad argument) are tool results with `isError`; what went wrong inside stays in the node's log.
  - Connecting one, on the app's AI Assistants screen, gives the owner a `claude mcp add --transport http oikome http://127.0.0.1:<port>/mcp --header "Authorization: Bearer …"` command for Claude Code, or the address and header for another app. Claude Desktop isn't offered yet: its custom connectors are remote servers, and whether one can reach this computer is still to be checked. No bridge process (`oikome mcp`) is needed, and nothing opens the data directory besides the node.
- **Plugins** (M2) are cards on Home: a folder of web files with a manifest (`oikome-plugin.json`: id, name, maker, version, what it reads as a shape and the kinds of column it needs, card size, entry page). Installed ones are kept in `.oikome/plugins/<id>/` (Oikome's own are built into the node); each is a Principal with a DID made at install and whole-Space grants on exactly the collections the owner chose. The web server serves `/plugins/<id>/…` with a policy of its own: CSP `default-src 'none'` with `connect-src`, `frame-src`, `worker-src` and `form-action` `'none'`, `sandbox allow-scripts` (an opaque origin: no cookies, storage or app API), `frame-ancestors 'self'`, a Permissions-Policy denying devices, and DNS prefetch off; the app's own `frame-src 'self'` stops a plugin frame navigating elsewhere. CSP doesn't cover WebRTC (Chromium ignores `webrtc 'block'`; a sandboxed frame still sent STUN packets), so the server injects the bridge (`packages/plugin-sdk/sdk.js`) as the page's first script, which removes the `RTC*` constructors before plugin code runs; nested frames, workers and `eval` are blocked, so no fresh copy can be had. The app reads a plugin's collections through `plugin_read`, which runs as the plugin, so the node's grants decide (a record claiming more changes nothing), and posts the rows as plain values (dates ISO, images `data:` URLs, up to 1,000 rows and 32 MB) with the style to that frame alone; the frame may ask only to open one of its collections. Reads are in Activity as the plugin's (allowed ones at most once per 10 minutes per collection, refusals always). `list_plugins`, `inspect_plugin`, `install_plugin`, `set_plugin_collections`, `remove_plugin`.
- **AI assistants** (M3d) are Principals too, made like a plugin's: a DID when the owner connects one, with whole-Space grants on exactly the collections chosen, kept in `.oikome/assistants.json` (0600). An assistant reaches the node with a token (`oik_` and 32 random bytes), shown to the owner once; only its BLAKE3 hash is kept, compared in constant time, and its last use is noted at most once a minute. It may do three things, as itself: list what it can read (each table's columns as it sees them, its rows and its key), search it, and read a row by its key. Its search goes through the same code as the owner's but in a session acting as the assistant: where a table has an index and a key, the index only finds rows, which are read again as the assistant by their keys with the values bound as parameters (`query_params`, routed and access-controlled as any query), so its column grants and row policies decide what it gets, and a row whose words are only in columns it may not see is left out; a table with no index or no key is read as the assistant. A collection it may not read is one it is told doesn't exist. Every search is a line of Activity with its words (`detail`), and its reads of one collection in one minute are one line with a count; refusals always show. Ending its access drops its grants and its token at once; removing it keeps its name for Activity. `list_assistants`, `connect_assistant` (its answer holds the token), `set_assistant_collections`, `end_assistant_access`, `remove_assistant`; it reaches the node over MCP (above).
- **Public shares** (3.8, not built). A share to anyone rather than to a contact. Nothing is public until the owner shares it (principle 5).
  - What: a collection, a view or chosen columns, query-only by default; copying only when the owner allows it.
  - Who: anyone with a DID, one made in the browser on the spot or a wallet's (`did:pkh`, signed in with Sign-In with Ethereum, EIP-4361). Callers are named in Activity by their DID like anyone else.
  - Limits per caller, set by the owner: requests per minute, rows and run time per query. A request over them is refused, not queued.
  - Reach: strangers never need the owner's address. Live queries go through a relay (§17); a public collection can also be published as a read-only snapshot to a storage or to IPFS or Arweave. Its files are named by their BLAKE3 hash and the owner signs its manifest, so anyone can mirror it and check what they got.
  - Being found: a public share may be listed where people look for it, if the owner chooses. Private shares never are.
- **Payment plugins** (M5, not built). An owner may charge for a public share or a share to a contact.
  - A payment plugin connects one payment service the owner chooses: PayPal, Alipay, a Web3 wallet and others. The owner sets it up with their own merchant account or wallet address, and the money goes straight there. Oikome has no payment channel of its own and never holds or moves funds.
  - What is priced: an item (a collection, a view, chosen columns), for a period or per query; a copy may be priced apart from query-only access.
  - Contract: `create_order` (item, price, buyer's DID → a payment link or QR code), `order_status` (pending, paid, refunded) and `refund`. Paid means confirmed by the payment service's servers, or for a wallet, by the node reading the chain through a public RPC until the transfer has enough confirmations. A node behind a NAT usually cannot receive a service's callback, so the plugin asks the service for the order's state.
  - The node issues the grant when an order is paid (a UCAN for exactly what was bought, for its period) and revokes it on a refund. Orders are in Activity.
  - Permissions are the reverse of a card's: a payment plugin may reach its own service's hosts and reads no data; it sees only orders (item, price, buyer's DID). How it runs (process, sandbox) is decided with M5.
  - A copy cannot be taken back once sold. Query-only access, a watermark per buyer on copies and security mode (§17.4) limit what a buyer can pass on; the apps say so when the owner prices a copy.
  - Merchant qualification, tax and content rules are between the owner and the payment service.

How each client reaches data (screens depend only on `NodeClient`):

| Client | Runs in | Reaches data through | Layout |
|--------|---------|----------------------|--------|
| Web, desktop browser | Page served by the Full Node | HTTP on loopback, owner session | Desktop |
| Desktop app (later) | Tauri 2 window | In-process calls to `oikome-api` | Desktop |
| Phone app (later) | Tauri 2, paired with the owner's node (§13.5) | WebRTC with a UCAN issued at pairing | Phone |

**Phones.** Pairing a phone comes with the phone app. A phone browser would need the browser node's page over HTTPS, which the owner's loopback server cannot give it, and a page from a public origin or a self-signed certificate on the local network was turned down. The screens have a phone layout already, which the phone app reuses. Which Owner operations a paired phone may perform is decided with it. How each kind of device pairs and reaches the node is in §13.5.

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
- **Words for people:** Collection (a Space), Contact, Share, Stop sharing, Activity (the access log), Your ID (the DID), invite code (the token, with where the sharer is and its name). A collection shared with you says whom it is from (`from_did`, `from_name`) and until when (`expires_at`), and is left by dropping it. The web app runs no SQL of the owner's: `run_sql` stays in the API for the command line and tests, and data comes in through importing (later).
- **Screens:** Home (what the data center holds, where and who reads it, what came in last, recent Activity, the room on every place, and the recovery key until it is stored), Collections (a cover per collection), a page per collection, Sources and a page per source (its status, recent syncs, how often it is read, a Pause switch and Remove), Sharing, Plugins, Activity, Settings. Activity says who each line was as `list_access` names them, and opens what a line means and where to act on it; it filters Everyone, People, Plugins and You. The sidebar lists the places collections are kept in; the card under it shows the room on the one picked (`list_storages` reports free and total space for this computer and a folder).
- **Two styles,** picked under Settings › Appearance and kept by the browser: Ink (the default: black ink on white, hairlines and no shadows, data in IBM Plex Mono, states as ink glyphs, crimson only for what needs the owner) and Bold (neubrutalist: 2px outlines, hard shadows, bright fills, Archivo Black). Both are sets of values for the same tokens; the faces ship with the app under the SIL Open Font License.
  - A collection shows what its first table holds, read from the columns rather than from names: a column of images is photos, a few columns with long text are notes, anything else is a table. Images reach the page as `data:` URLs, which the CSP already allows.
  - Times written without a zone (`TIMESTAMP`) show as written; `DECIMAL` values with their scale.
- **First run:** the data center's name is filled in, so Continue alone goes through. `add_examples` then makes four examples, once, one of each kind: photos of a weekend in Lisbon, a short guide to Oikome (notes), three weeks of spending that end with that weekend (a table) and two weeks of a home's hourly temperatures and humidity (readings). They are plain files, like a notes app's vault: written into `Examples/<name>/` of the folder the owner picked for new collections, or of the data folder (photos with a `captions.csv`, notes as Markdown, tables as CSV), and each folder is the source (§11) of the collection of its name, read again every minute. A file already there is left as it is; a removed example stays removed, and its files stay. The six photos are from Wikimedia Commons (CC0 and CC BY), credited in `captions.csv`, in each row and in `NOTICE`; the rest is written for Oikome, and the readings are computed from a fixed formula. The apps show numbers over time with nothing else beside them as *readings*: the latest values and a line of one measure (a line, since a temperature has no zero worth drawing bars from), with the rows below; a column suffix such as `_celsius` or `_pct` names its unit.
- **Where a collection is kept:** `create_space` takes a `storage`, this computer when left out; `list_storages` lists this computer and each storage, with the number of Spaces on it and the default for new collections (`set_default_storage`; `add_examples` puts the examples there). A removed Space leaves its files, and the name stays taken (`conflict`). Sources in the apps: `list_sources` (with the last read and the last failure, kept with the source until a read succeeds), `add_source` (a CSV file whose first column names each row, picked with the node's folder listing, which also lists CSV files and counts photos and notes; a folder of photos or notes is refused and viewed in place instead, below), `pull_source`, `pause_source`, `set_source_every` (a minute, 5 or 30 minutes, an hour, a day, or only when asked), `list_source_runs` (the last 30 days of syncs), `remove_source` (the collection stays; the same file or folder added again, by its real path, goes back into it while it is there and no other source fills it, as `node.json` remembers) and `show_source_files` (opens a registered source's own folder or file in the system's file manager, an argument list and no shell). Removing a collection stops its source; the files stay. Days and times kept as text (`2025-11-07 09:40:00`, as SQLite and files hold them) read as dates in the apps. A node keeps its data in `~/Oikome` (`C:\Users\<name>\Oikome` on Windows) unless `--root` or `OIKOME_ROOT` says otherwise: the owner's, in plain sight like a notes app's vault, and not in Documents, which iCloud and OneDrive often sync, which breaks a live database (cloud drives are added as encrypted places instead). First run suggests the account's first name for the data center ("Rosh's data center") when the system knows one. Home's cards are the owner's to arrange, in place on Home (`set_home_cards`, kept in `node.json`): recently added, a photo, a table's daily totals over two weeks, the latest readings, activity and places, each with the columns of twelve it spans (4 to 12). A card kept without a width takes the app's own, the last in its row stretching to the row's end; `node.json` from before cards had widths still reads. The node changes `node.json` one change at a time, each written through a temporary file of its own. This computer's storage is called *Local* and is always there; a place added later is removed with `remove_storage`, refused (`conflict`) while a collection is on it. Removing it forgets its credentials and the locations removed Spaces left on it, never its files, and new collections go to Local when it was the default. A collection a source fills goes on Local by default; choosing another place for it asks first and says that the working copy stays on this computer and the place gets a copy after every sync (§7.5). Making a NAS or drive the default asks the same once.
  - The owner picks a place instead of typing one: Welcome and Settings offer this computer (its data folder, or another folder), the cloud drives and drives found on it (`find_places`), a NAS, or another folder. A web page cannot ask the system for a folder, so the node lists folders (`list_folders`, owner only, hidden ones left out) and the page browses them, as NAS and media server apps do.
  - `add_storage` takes what was picked and calls the node's own `add_storage`: files go in an `Oikome` folder inside a picked folder, a folder inside a cloud drive is a drive (always encrypted), a password goes to the system keychain, and the storage is checked before it is kept. The data center's own folder (identity, catalog, Activity) stays where the node runs.
- **Folders viewed in place (3.9, §7.7):** `view_in_place` makes a collection that reads a folder where it is (a full path on this computer, or a path within a NAS) and is named after it; `list_spaces` says which folder it views, how many files its index knows, when it was read and whether it could be reached. `refresh_space` reads it again now; `list_files` lists one folder of it from the index (its folders with how many files each holds, and a page of files, photos first), or files of every folder of one kind for a cover; `read_file` gives one file with all of a note's text; `show_folder` opens a folder on this computer in the file manager. Photos are served at `/files/thumb/…` and `/files/original/…` (§7.7). The apps' Collections page and Sources page view a folder in place where they used to add a folder source.
- **Recovery key:** the identity seed, shown to the owner as 56 Crockford Base32 characters in 14 groups. It is made of a version byte, the 32-byte seed and two BLAKE3 check bytes (`oikome-identity::recovery`).
  - Reading it forgives case, spaces and look-alike characters, and rejects typos.
  - `recovery_key` returns it (owner only, never logged); `confirm_recovery_key` records that it was stored. The node's name and that record live in `.oikome/node.json`.
  - `oikome identity restore` reads a key from standard input and writes it as the node key. It only does this on a node that holds nothing yet: no Spaces, contacts or issued tokens. A running node keeps its identity until it is restarted, so the web app only explains this step; the desktop app can do it in process later.

**Scope of the apps.**
- They serve the owner's personal data center: onboarding, Spaces, sources, sharing, AI assistants, plugins and settings. Node operations (engines, logs, peers' internals) stay in the CLI. Members and family roles come later.
- The UI is English; other languages arrive through i18n later.
- Visual design: Apple-style minimal on a low-chroma cream palette. One crimson primary action per view; blue only for links and focus. Settings and lists are grouped inset rows; a task beside a list (a new share) opens as a side panel, not a modal; removing something sits behind a More (…) menu and is confirmed inline; a field in focus only darkens its border. Its tokens and rules move into `design/` with DESIGN.md.
- **Storage.** The web app's onboarding shows where the data is kept; choosing a NAS (§7.5, phase 3.4a) moves into the apps with a storage screen. Cloud drives come through their sync folders (3.4b, always encrypted) and later over their API (3.4c); the apps offer each as it arrives. Credentials and keys are entered in the apps and kept by the node (§7.5). Baidu Netdisk (3.4d) and object storage (3.4e) follow. Browsing files where they are comes with 3.9 (§7.7), and Save To with 3.10 (§7.8). Serving Oikome over WebDAV is planned but not scheduled.

### 13.5 Devices: Pairing and Getting Online

Nothing here is built yet. The phone and desktop apps arrive later (§14.1), and the web prototype draws the flows. This section sets what a device is to Oikome, how each kind pairs, and how it reaches the node.

**Two kinds, by which way the data flows.**
- **Devices that read** the data center: phones, tablets, another computer, and later perhaps a TV. Each runs an Oikome app and is paired. It gets a UCAN for its DID and is listed under Sharing › My devices. Removing it revokes the token.
- **Devices that make data:** lights, plugs, sensors, appliances, robot vacuums, watches and scales. Oikome reads them and they read nothing, so they get no grant. Their data comes in as a source (§11), nearly always through something in between.
- One thing can be both. A phone is a paired device, and its photos are a source. A watch with the app reads, while its heart rate comes in through the phone's health app.
- Oikome reads devices and never controls them. It is not a smart-home hub: control and automation stay with Home Assistant or the maker's app.

**Common devices.** Getting a device onto the home network is the job of its maker's app or the hub, never Oikome's.

| Device | Gets online by | Usually paired by | To Oikome |
|--------|----------------|-------------------|-----------|
| Phone, tablet | Wi-Fi, cellular | An app: scan a code or sign in | Reads; scans a code (below) |
| Another computer | Wi-Fi, Ethernet | Found on the network, or a code typed in | Reads; found and confirmed by six digits, or takes a code |
| TV, projector | Wi-Fi, Ethernet | The TV shows a code that a phone scans or types, as YouTube and Netflix sign in | Reads, if a TV app is made; the TV shows a code and a paired phone scans it |
| Watch | Bluetooth to the phone; some also Wi-Fi | Through the phone (Apple Watch, Wear OS) | Both: reads through the phone's pairing, and what it records comes through the phone's health app |
| Earbuds, speakers | Bluetooth; speakers Wi-Fi | Pairing mode near the phone; the maker's app | Neither: nothing worth keeping, nothing to show |
| Lights, plugs, switches, sensors | Zigbee, Thread, Wi-Fi, Bluetooth | A gateway set to let devices join (Hue: the bridge's button); Matter: the code on the box | Makes data, through a hub |
| Fridge, washer, air conditioner, robot vacuum | Wi-Fi | The maker's app gives it the Wi-Fi password over Bluetooth or a temporary hotspot, then binds it to the maker's account. Matter 1.2 and later define these types too | Makes data, through a hub or the maker's cloud |
| Camera, doorbell | Wi-Fi | The maker's app | Makes data; proposed to leave out for now (privacy, size) |
| Scale, blood-pressure monitor | Bluetooth, Wi-Fi | The maker's app, synced to the phone's health app | Makes data, through the phone |
| Meter, charger, solar inverter | Wi-Fi, Zigbee, a maker's gateway | The maker's app or gateway | Makes data, through a hub |

**Pairing a device that reads.**

Every way ends the same. The node issues a UCAN to the device's DID. The device can read Home and the collections it is given, read only, until the owner changes that. What differs is how the owner confirms it is the right device: a one-time secret carried by a code, valid for minutes and used once, or six digits shown on both screens.

| Way | For | How | Status |
|-----|-----|-----|--------|
| Scan this computer | Phone, tablet | The app scans a QR code on this computer. The code holds the node's signed access record (§10: DID, address, certificate fingerprint) and the secret | Decided (2026-11-02); comes with the phone app |
| Found on the network, digits compared | Another computer | The other computer opens "Join a data center" in its desktop app, and this one finds it by mDNS (§9). Both screens show six digits computed from both DIDs, so a device in between would show different ones. The owner clicks Pair | In the prototype; comes with the desktop app |
| Pairing code | Another computer, anything with a keyboard | A code from this computer, typed or pasted on the other. It holds this computer's address and the secret, so no discovery is needed. Used when discovery fails, for example on a network that blocks multicast | In the prototype; comes with the desktop app |
| The device shows, a phone scans | TV, car screen | The device shows a QR code. A paired phone scans it, and the owner approves on the phone | Proposed; no TV app is planned |
| Through the phone | Watch | A watch app reads through the phone's pairing | Proposed; not planned |

- **Not possible:** a web browser on another device. It would need an HTTPS page, the same reason as for phones (§13.4).
- **Words in the apps:** Add a Device opens the flow, as Add Source and Add Place do. Pair is only the button that confirms matching digits.

**How a paired device reaches the node.**
- **On the same network.** A paired device reaches the node's address directly. The phone app uses WebRTC, as the browser node does (§10). Another computer runs a node of its own and reaches this one as a peer, over Arrow Flight with TLS checked by DID (§9). The browser node has no way from another network yet: WebRTC there would need its own hole punching (no STUN or TURN of the project's, §17.5). The apps say so when adding a device.
- **Security mode** (§17.4) does not stop pairing: mDNS then announces a random identifier rather than the DID, and the six digits still confirm the device.
- **Away from home:** another computer running a node reaches this one from another network since 3.12 (§9): the router opens the port while the owner allows it, or a contact introduces them. The phone app gets the same when it arrives (M7).

**How data from devices that make it gets in.**

| Through | Examples | Path | Status |
|---------|----------|------|--------|
| A home hub | Home Assistant, which also brings in Xiaomi devices through Xiaomi's own integration | Device → hub → Oikome. Oikome talks only to the hub at home, but the hub may reach some devices through their maker's cloud | The way for now (the prototype's smart-home source); the Home Assistant connector is not built |
| The maker's cloud | SmartThings, Tuya, LG ThinQ | Device → maker's cloud → Oikome, with the owner's token. The data is on that cloud already | Only where the maker publishes an API (§11); not scheduled |
| Directly | Matter devices shared to Oikome with a pairing code from Apple Home, Google Home or Home Assistant (Matter lets several controllers share a device). Devices with a local API: Hue Bridge, Shelly, ESPHome | Device → Oikome, through no one else | Undecided (below) |
| The phone | Watch, scale, blood-pressure monitor | Device → phone's health app → Oikome | Today as an export file (Apple Health `export.xml`, §11.1) |

**Open questions.**
- What another computer becomes: a reader like a phone (the prototype's choice), a second copy of the data center, or a second place to manage it.
- Which Owner operations a paired device may perform. This is decided with the phone app (§13.4).
- Whether Oikome talks to Matter devices itself. Three options:
  - **(A)** Only through hubs, as now.
  - **(B)** A read-only Matter controller, added through a hub's "share device" and entirely local. This is the recommended next step, once its license and dependencies pass the dependency policy.
  - **(C)** Its own hub with Zigbee, Z-Wave and Bluetooth radios. Not recommended: it would rebuild Home Assistant.
- Whether one Devices page shows both kinds: readers with pairing and removal, and devices that make data with what they come through and which collection they fill.
- A NAS or home server running Oikome is proposed not to count as a device. It is either the data center itself or a place collections are kept (§7.5).

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
| **3.4c Drives over their API** | Google Drive and Dropbox through opendal (upgraded to the current release) for nodes without a desktop client, encrypted as in 3.4b. OAuth with PKCE, on a loopback port or with a pasted redirect, using the owner's own OAuth client. Tokens are kept and refreshed by the node and revoked on removal. Spaces need only the least permission. Large files use resumable or session uploads, added to opendal in Oikome's fork and contributed upstream. OneDrive stays on its sync folder (§7.5) | A Space on Google Drive and one on Dropbox, reached over the API, are written by one node and read by another with the key. A file larger than one upload request is stored and read back intact. Tokens are only in the keychain or the node's private file. An expired or revoked token asks for sign-in again, and a failed push does not fail the write |
| **3.4d Baidu Netdisk** | An opendal `baidu-netdisk` service in Oikome's fork, proposed upstream (apache/opendal#8481) and used as a git dependency. OAuth sign-in, folders under `/apps/Oikome/`, encrypted. Refused calls are retried with backoff (§7.5) | A Space on Baidu Netdisk is written by one node and read by another with the key. A rate-limited call is retried without failing the push. The service passes opendal's behavior tests against a real account |
| **3.4e Object storage** | S3-compatible services through opendal's `s3` (AWS S3, R2, B2, MinIO, and the OSS and COS endpoints). Keys are kept by the node. Encrypted by default, optional for a service the owner runs (§7.5) | A Space on MinIO (in CI) and one on a hosted service are written by one node and read by another. Without the key the hosted copy cannot be read. A large file is uploaded in parts and read by range |
| **3.5 Connectors** ✅ | `Connector` contract (`oikome-connect`); upsert and cursor in one SQLite transaction; scheduled and on-demand pulls; tokens kept by the node; CSV and flomo connectors (export file; MCP written, run against flomo once a MAX account is at hand) | flomo memos land in `flomo.memos`; a second pull touches only changed memos; an edited memo is updated, not duplicated; a failed pull leaves data and cursor unchanged |
| **3.6 Compute sharing** ✅ | `compute/<method>` UCAN abilities with `nb` limits, apart from data grants; only Oikome's own compute methods, offered by the executing node (first: `sql` in an isolated DuckDB); tasks over Arrow Flight `DoExchange`; per-peer quotas | Node B runs a task for node A within its quota; a task over its limits is refused; revoking the UCAN drops A's queued tasks; A's compute grant gives no access to B's Spaces, and a data grant does not let A run tasks |
| **3.7 Security** | Security mode (§17.4) in the node, the API (`security_mode`, `set_security_mode`, owner only) and the apps' Settings › Security; the warnings of §17.3; the node key in the OS keychain; the browser node's key non-extractable; the first-run disk check; `SECURITY.md`. The network part of §17.4 (a network key apart from the DID, for contacts that introduce others, 3.12) | Every step below also works with security mode off. With it on: mDNS carries no DID, and a contact still finds the node on the local network; a contact's query is in Activity with its SQL and row count; a copy is kept sealed on the contact's node and deleted there when the share is revoked or ends; a borrowed task sends only the columns it reads, the lender keeps nothing after returning the result and signs a receipt, and both sides record what was sent; `--allow-http` is refused. In both modes, on macOS and Windows the node key moves into the keychain at the first start (checked by hand; tests use the file store) and stays an owner-only file elsewhere; a browser node's key cannot be exported; `SECURITY.md` says how to report a vulnerability |
| **3.8 Public shares** | Shares to anyone (§13.4): query-only by default, limits per caller, callers by any DID including a wallet's (`did:pkh`); read-only public snapshots with a signed manifest on a storage or on IPFS or Arweave; photos in a public share lose their location data by default (§7.9) | Set when 3.8 is planned. Live queries by strangers need the gate of §9 (contacts only from outside) to let callers of a public share in; public snapshots do not |
| **3.9 Viewing in place** ✅ | In-place Spaces over folders on this computer and on storages (§7.7). They are read-only, with a table per CSV, Parquet or JSON file. The node keeps an index (files, text, thumbnails) as a cache, and previews use range reads. Whole-drive read access is asked for when needed. The folder sources of M2 move to in-place Spaces | A folder on this computer, one on a NAS and one on a drive are browsed and their CSV files queried, with nothing written to them. Search and thumbnails work from the index while the place is offline. Deleting the index and refreshing rebuilds it. A contact granted an in-place Space queries it and is cut off when the grant is revoked |
| **3.10 Save To** | Save rules (§7.8): plain files or a new Space as the target, folder and name rules, once or continuing. A rule changes only the files it wrote. A saved Space keeps a portfolio (order, cover, title, captions) that travels with it when it is shared (§7.9) | Photos saved once land as plain files in the rule's folders, under its names, and open in other apps. With a continuing rule, a new photo appears at the next run, unchanged files are not written again, and a file the rule did not write is never overwritten. A rule into a Space commits a snapshot each run. A contact given a saved Space sees its portfolio's order, cover and captions, and keeps reading it after its source folder is removed |
| **3.12 Reaching home from anywhere** ✅ | Nodes cross networks without any relay (§9): the router opens the node's one peer-to-peer port (UPnP, PCP, NAT-PMP) while a share or lending the owner allowed needs it, asked first (Deny / Allow, "Don't ask again"); otherwise a contact both sides reach introduces them and they punch through to a direct connection (16 KiB and 30 s a circuit, contacts only); only contacts get in from outside; addresses travel signed in invites and between contacts, never to a DHT or DNS; Flight runs over a libp2p stream with TLS pinned to the DID; Settings › Network | Two nodes behind routers that open nothing read each other's shares after meeting at a contact, directly; with the contact not helping they can't; with UPnP on one router they read through the opened port. A stranger with a valid token is turned away from outside until made a contact. Nothing is opened unless allowed, an answer without "Don't ask again" ends with its share or lending, and the web pages and assistants stay on loopback (Docker NAT check in CI; in process in `across.rs`) |
| **3.13 Serving over WebDAV** | Collections and saved files served over WebDAV, read-only by default, so other apps open them like a drive | Set when 3.13 is planned |

No phase skipping (AGENT.md §4.1). Phases that need accounts (3.4c–3.4e, drives in 3.9, flomo's MCP from 3.5) are written first. Once all of them are written, they are run against real accounts together.

### 14.1 Apps Track

The user-facing apps (§13.3, §13.4) run beside the core phases. They do not block the core phases, and the core phases do not wait for them. A milestone starts only when its gate phases are done; within the track, too, there is no skipping.

| Milestone | Goal | Gate | Exit criterion |
|-----------|------|------|----------------|
| **M1a Layout** ✅ | `web/` split into `packages/node` and `e2e/`; root `package.json` with npm workspaces | 3.2 ✅ | `browser-e2e` passes from the new paths with no change to the tests' assertions |
| **M1b API** ✅ | `oikome-api` with the Owner operations of §13.4 (Spaces, rows, SQL, contacts, sharing and revoking, access log); whole-Space grants; the access log; `oikome serve --web` and `oikome web open`; `packages/api`; `packages/client` with its HTTP implementation. The recovery key and phone pairing are designed separately before M1c | 3.0–3.2 ✅, M1a | e2e starts `oikome serve --web` on an empty temporary data directory and, through `packages/client`, creates a Space, writes and reads rows, shares the Space with a contact and revokes it, after which the contact's query fails. A request without the session cookie or with a foreign `Origin` is refused |
| **M1c Usable** ✅ | `packages/ui`, `packages/screens` and `apps/web`: the M1 screens (onboarding with the examples and the recovery key, Home, Collections and a page per collection, sharing and contacts, Activity, settings with storage), desktop and phone layouts; `--web-root`; `oikome identity restore`. Phone pairing moves to the phone app | M1b | In e2e on CI (`web-app.spec.mjs`), in Chromium against a local node started from an empty data directory: onboarding and the recovery key; the examples opened as photos, notes and a table; a collection of one's own filled through the node and browsed; the collection shared with a contact (a second node) and stopped, after which the contact's query fails; both in Activity. Each M1 screen renders at 1280 and 390 px without sideways scrolling, and no text fails WCAG AA |
| **M2 Connect** ✅ | Sources screen and sync status, Home dashboard, built-in cards, dashboard plugins v1 (`plugin-sdk`, data contract, plugin grants), reference plugins | 3.5, M1c | A connector's data appears on Home; a plugin reads only what its grant allows and cannot reach the network |
| **M3 Find** ✅ | Full-text search, photo and note views, AI Assistants screen, MCP over HTTP (`/mcp`) | 3.3 ✅, M2 | An MCP client lists and searches only the Spaces granted to it; revoking the grant in the app cuts it off; Owner commands are not offered over MCP |
| **M4 Extend** ✅ | Borrowed compute in the apps: Home's big summaries worked out on a computer lent here, for collections shared with its owner only (§12.2). Third-party plugins and the plugin directory move out of M4, to come one plugin at a time; drives over their APIs stay with 3.4c | 3.4b, 3.6, M3 | With a contact lending to this node and a collection of at least 100,000 rows shared with them, turning their computer on makes its daily totals worked out on their node, to the same totals as here, and both Activities say so; a collection not shared with them, or one they may see only part of, is never sent; when they pause or can't be reached, the totals are worked out here |
| **M5 Pay** | Pricing in the apps (what, for how long or per query, query-only or copy); payment plugins with the order contract (§13.4); the buyer's side for contacts and strangers; orders in Activity. Comes last, with third-party plugins, after the core phases 3.4c–3.10 | 3.8, M4 | Set when M5 starts |
| **M6 Desktop app** | `apps/desktop` (Tauri 2): the Full Node in process with the same screens (§13.3); the web app embedded in the binary; installers for macOS, Windows and Linux, so no command line is needed | M4 | Set when M6 starts |
| **M7 Phone app and devices** | `apps/mobile` (Tauri 2): pairing by QR code (§13.5), My Devices and Add a Device; the phone reads Home and collections over WebRTC, away from home once 3.12 is done | M6, 3.12 | Set when M7 starts |
| **M8 Languages** | The apps in more languages: strings are translatable already, and the prototype has Chinese, Japanese and others; user data is never translated | M1c | Set when M8 starts |
| **M9 Family** | Family members, each with an identity of their own and what they may see | M7 | Set when M9 starts |
| **M10 Plugins from others** | Third-party source and card plugins, one plugin at a time, with signing and a plugin directory; how their code runs is decided for each kind of plugin (§13.4) | M4 | Set when M10 starts |

**Order (2026-11-06).** Numbers name the phases; they are not the order. Done: 3.9 (so nothing is copied into Oikome just to be seen), then 3.12 (brought forward, so contacts reach each other from home to home). Next: 3.8, 3.7, 3.10, 3.4c–3.4e (their real accounts tested together), M6, M7, M8, M9, 3.13. Last: M5 and M10.

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
| Drive APIs change, rate-limit or expire tokens (3.4c, 3.4d) | opendal no longer runs its Dropbox and Aliyun Drive tests against the real services, so Oikome checks the drive services with real accounts itself. The sync-folder path needs no API. A failed upload leaves the previous snapshot readable, and refused calls are retried with backoff |
| Google's `drive.readonly` is a restricted scope: a published app needs Google's verification, and while an OAuth client is in testing its refresh tokens expire after 7 days | Viewing a drive in place is asked for separately from keeping Spaces (`drive.file`). During testing the owner signs in again each week. Verification comes before release |
| A Baidu upload needs every block's MD5 before it starts, and downloads without a membership are slow | `RemoteStore` hashes local files before uploading them. Baidu Netdisk is suggested for Save To and backups rather than for Spaces that are read often |
| opendal does not take the Baidu Netdisk service, or takes it slowly | Oikome keeps using its fork, or moves the service into a crate of its own. Nothing else changes |
| Save To writes into folders the owner also uses | A rule changes only the files it wrote, recorded with their BLAKE3, and deletes only if it says so |
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
| 2026-11-01 | Desktop and phone apps on Tauri 2 (later) | Its backend is Rust and links our crates directly, so the desktop app is a Full Node with a UI and no command line; the same framework also targets iOS and Android; Apache-2.0 / MIT |
| 2026-11-01 | The apps offer local folders and NAS as storage; cloud drives as Locations stay undecided, with §7.5 and phase 3.4 unchanged until then | The product rule is that data stays at home, which conflicts with keeping it on a drive provider's servers; browsing existing drive files and serving WebDAV are planned separately |
| 2026-11-02 | Storages: a NAS first (3.4a, WebDAV or a mounted folder); cloud drives step by step, each only once its files are encrypted on the client (3.4b onward). Settles the entry above | Data on a NAS stays at home; a drive provider can read what is uploaded, so encryption comes first |
| 2026-11-02 | A Space on a storage is copied, not read in place: the writer keeps it on its disk and pushes each snapshot (files, then a snapshot file); readers copy the newest complete snapshot and read it like a subscription. `materialize()` dropped | DuckDB reads Iceberg as lists of local Parquet files and the `lance` extension attaches only local directories; Iceberg commits over WebDAV have no compare-and-swap; the subscription code already copies verified snapshots, so one mechanism serves Iceberg and Lance, and encryption later wraps each file |
| 2026-11-02 | One writing node per folder, recorded in `oikome-space.json`; passwords and tokens only from the environment; a failed push does not fail the write | WebDAV has no conditional create, so the record is a guard, not access control; servers and containers have no keychain (it comes with 3.5); failing a committed write invites a retry that duplicates rows |
| 2026-11-02 | 3.4b: drives through their sync folders first, over their API in 3.4c; every file on a drive sealed whole (XChaCha20-Poly1305 STREAM, names from a keyed hash), one random key per storage | Sync folders cover iCloud Drive, Dropbox, OneDrive and Google Drive with no OAuth app; whole-file sealing in `RemoteStore` covers data, indexes, snapshots and names alike, which Parquet modular encryption would not |
| 2026-11-02 | 3.5: connectors are in-process Rust (third-party ones as M4 plugins) and registered on the node (`sources.json`, CLI and apps), not by SQL; rows go to SQLite as values, one parameterized upsert each; the cursor lives in the Space's hidden `_oikome_source` table so it commits with the data; flomo through its export (keyed by creation time and place within the second) and its MCP server (tools found at run time, never deletes) | A connector is node configuration with a credential, like a storage; the hidden table makes "data and cursor change together" a single SQLite transaction; flomo's export has no ids, and its MCP tools are not published |
| 2026-11-02 | 3.6: compute means CPU and GPU time; tasks run only with Oikome's own compute methods, offered one by one by the executing node's owner, starting with `sql` in a DuckDB of its own per task (in memory, no files or network, no extensions, configuration locked); request and response over `DoExchange`; `compute/<method>` grants with `nb` limits that only tighten; per-peer quotas; no forwarding to external services. Settles §12's open questions on workloads and accounting | The methods Oikome already runs are the standard of what a task can be, so nothing new has to be trusted; a fresh locked-down DuckDB keeps the executing node's Spaces and files out of reach; quotas are the simplest accounting that keeps a peer from taking a whole machine |
| 2026-11-02 | Credentials are entered once (apps, or standard input to the CLI) and kept by the node: OS keychain on macOS / Windows, else a private file beside the node key; the environment overrides; this replaces "environment only" for 3.4a | People should not have to set environment variables; the apps call the same node functions (`add_storage`, `set_storage_secret`, `export_storage_key`); servers and containers keep the environment |
| 2026-11-01 | App tests run against a local node (temporary data directory, test data written through the CLI or the API); no fake data and no mock `NodeClient`. So the API (M1b) comes before the screens (M1c), and each screen arrives with the milestone whose data it shows | One test chain instead of a fake one followed by a real one; screens exercise the real API from the start, and there are no fixtures to drift from it |
| 2026-11-01 | Whole-Space grants (`GRANT SELECT ON SPACE`) in their own registry; they cover a view only when its expansion stays in the Space | Sharing is per Space in the apps and should include tables created later; a relation name such as `"*"` could not stand for the Space safely; owner-rights views would otherwise open other Spaces |
| 2026-11-01 | The access log is SQLite under `.oikome/`, written by a thread behind a bounded channel; failed authentications merged per method and minute | Requests never wait for logging; the owner can page and filter it; unauthenticated callers cannot push real entries out |
| 2026-11-01 | The web API's login link is written to an owner-only file and opened with `oikome web open`, never printed | A token in terminal output stays in scrollback and logs; whoever can read the data directory is the owner anyway |
| 2026-11-01 | Registry writes take a file lock and re-read the files first; `refresh` reads under a shared lock | With the web API, `serve` writes the registry too; a `REVOKE UCAN` from `oikome sql` must not be undone by its next write |
| 2026-11-02 | Phone pairing comes with the phone app, not the web app | A phone browser needs the browser node's page over HTTPS; the loopback server cannot serve it, and a public origin or a self-signed local certificate was turned down. The screens already have a phone layout for the app |
| 2026-11-02 | Recovery key: Crockford Base32 groups with two check bytes; restored only into an empty node, with `oikome identity restore` | No dependency (unlike a word list); a typo is caught instead of restoring another identity; a running node cannot swap identities, so restore happens while it is stopped |
| 2026-11-02 | The web app starts with examples (one weekend in Lisbon: photos, notes, spending) and shows each collection as what it holds; no SQL workbench in the apps | A first screen with something in it shows what Oikome is for and gives every test the same data; owners are not database users, and their data arrives by importing |
| 2026-11-02 | Web app on React 19 and Vite 7, served from a directory (`--web-root`) under a strict CSP | React's ecosystem and Tauri templates for the later apps; Vite 8 requires lightningcss (MPL-2.0); embedding the build waits for packaging |
| 2026-11-01 | Visual design: Apple-style minimal, low-chroma cream palette, one crimson primary action per view, blue only for links and focus | Calm and familiar for ordinary users; color marks the one thing to do instead of decorating; every text pair is checked against WCAG contrast |
| 2026-11-03 | A collection a source fills keeps its SQLite working copy on the node's disk; kept on a storage, it gets a published copy pushed there after every pull. The apps default these collections to Local and ask before any other place | SQLite needs a local filesystem, and a mounted SMB or NFS share is still a network one. Owners think "my photos are on the NAS"; publishing and pushing already exist, so the copy there is the same kind of second copy as for any Space on a storage |
| 2026-11-04 | Activity keeps the owner's first steps (setting up, the examples, the recovery key) as the owner's lines, through this computer; setting up is kept past pruning, and a data center set up before gets them once from its settings | Activity is where the owner sees what happened to their data, and the first run is where that starts |
| 2026-11-04 | Activity names each caller as the owner knows them: you, a person, a plugin, or unknown. A removed contact's or plugin's name is kept with the log as long as its lines; a name a caller gives itself is not shown | Whether someone is a contact is not who they are; a removed plugin's or contact's lines must still say who it was; a self-given name could be anyone's |
| 2026-11-04 | Sources pause rather than stop; removing one asks first and keeps its collection, and the same file or folder added again goes back into it. Each source has a page with its recent syncs (30 days, kept in its Space, runs alike in a row kept as one) and how often it is read | Stop used to unregister the source, so it vanished and came back as a new collection; owners mean pause; a source read every minute would fill a history with "nothing new" |
| 2026-11-04 | Security is stated as who can see what (§17). Defaults are sound rather than maximal. One switch per node, security mode, off by default, adds protection to the same features and never turns one off: local discovery without the DID, copies kept sealed and deleted when a share ends, borrowed tasks that send only the columns they read and leave nothing behind, a fuller Activity, a network key apart from the DID on relays. A third-party audit, managed relays, two-hop relaying, post-quantum key exchange and team-wide policy are left for later | Owners need to know exactly who can see their data. An open-source project can make the defaults safe for ordinary use and leave the costly guarantees for later, some as services. A mode that turned off sharing or borrowing would make owners choose between safety and the product; protection added to the same features does not |
| 2026-11-04 | The owner may open chosen data to anyone, strangers included, within limits the owner sets (public shares, query-only by default). Deny by default still holds: a public share is the owner's explicit grant | Exchange is not only between people who know each other: owners publish knowledge to people they have never met. Limits per caller keep a stranger from taking more than was offered or wearing the node out |
| 2026-11-04 | Oikome covers Web2 and Web3: wallets as identities (`did:pkh`), chains as sources, public snapshots on IPFS or Arweave. No token or chain of its own, no private data on a chain, and no wallet needed | Owning one's data is the point, not a chain. A token turns users into speculators, and in China token business is tightly restricted. What Web3 does bring (wallet identities, public verifiable data, on-chain data) fits the existing DID, connector and content-addressing pieces |
| 2026-11-04 | Owners may charge for chosen data through payment plugins they pick (PayPal, Alipay, Web3 wallets and others), paid straight into their own accounts. Oikome never holds funds; a paid order is only the condition for a grant and a refund revokes it; payment plugins reach their service but read no data | Decentralized means no payment channel is chosen for the owner, and a platform that collects money and pays it out for others needs a payment licence. Keeping money and data in separate hands means neither can leak the other |
| 2026-11-04 | Devices are told apart by which way the data flows (§13.5). Devices that read are paired: a phone or tablet scans a code; another computer is found on the network and confirmed by six matching digits, or takes a pairing code. Each gets a UCAN, read only until the owner changes it, and removing it revokes the token. Devices that make data come in as sources, through a home hub, the maker's cloud, the phone, or directly. Paired devices need the same network until NAT crossing. The apps say Add a Device, and Pair only for the confirming step | Pairing and removal use the grants and revocation the node already has, so nothing new has to be trusted. Appliances and sensors never read the data center, and Oikome reads devices rather than controlling them. Add matches the other Add actions and covers devices that need no pairing; Pair is the step where two screens are compared |
| 2026-11-04 | Files are viewed where they are, through read-only in-place Spaces whose index is a cache on the node. A copy is made only by Save To, into plain files or a new Space, once or continuing. A saved Space is shared on its own, also as a portfolio. Sharing it with anyone goes through public shares (3.8), with photos' location data removed by default. Phases 3.9–3.11. Settles the 2026-11-01 entry on browsing drive files | The owner should see every place and app without copying. Copying is a choice, with a target and rules. A saved Space no longer depends on its source. Oikome views, picks, saves and shares, and editing stays in other software |
| 2026-11-04 | Storages are sources as well as targets. Viewing a drive in place needs read access to the whole drive: it is asked for separately, used while testing, and goes through Google's verification before release. Keeping Spaces and Save To keep the least permission. 3.4c is Google Drive and Dropbox, after upgrading opendal to the current release. Large files go up through Google's resumable uploads and Dropbox's upload sessions, added to opendal's writers in Oikome's fork and contributed upstream. OneDrive stays on its sync folder | Viewing in place cannot work with only the files Oikome creates, and asking separately keeps the broad permission optional. Upload sessions are how both drives take large files, so readers see one file and every opendal user gains; parts on the drive would need reassembly on every read. opendal's `onedrive` needs read-write access to the whole drive, and 3.4d needs the current release |
| 2026-11-04 | 3.4d: Baidu Netdisk is an opendal service written in Oikome's fork and proposed upstream (apache/opendal#8481), not a client inside Oikome. 3.4e: object storage through opendal's `s3`, encrypted by default | One service serves every opendal user, and Oikome's own code stays the VFS. If upstream declines, the fork or a crate of Oikome's own keeps it. One S3 service covers AWS, R2, B2, MinIO and the S3 endpoints of OSS and COS, and its range reads and multipart uploads suit video |
| 2026-11-05 | What was "later" gets a place: 3.12 reaching home from anywhere (relays), 3.13 serving over WebDAV (first 3.14; a chain source, first 3.13, was dropped: a chain is a way to pay, M5, and to be a caller, `did:pkh` in 3.8, not a source); M6 desktop app and installers, M7 phone app and devices, M8 languages, M9 family, M10 plugins from others. Order: 3.9, 3.8, 3.7, 3.10, 3.4c–3.4e, M6, 3.12, M7, M8, M9, 3.13, then M5 and M10 | One roadmap to read from start to end; installing without a command line comes before phones, and phones away from home need relays first |
| 2026-11-05 | 3.9 viewing in place: an in-place Space's index is a SQLite file (FTS5 trigram, thumbnails) whose rows are also written as a Parquet `files` table, not the planned Lance dataset; tables are the folder's top data files read by DuckDB each time; a NAS's are fetched into a 2 GiB cache; offline tables fail with a message; the folder sources of photos and notes are turned into in-place Spaces once, keeping names and whole-Space grants and deleting the SQLite copies | The owner wants nothing copied into Oikome just to be seen; SQLite searches Chinese as the sources' index does and keeps thumbnails in one file, and the index is a cache that can move to Lance; DuckDB reads Parquet without an extension |
| 2026-11-05 | The node's runtime threads get 8 MiB stacks | A contact's query of a Space shared with them (DataFusion over Flight) overflowed tokio's default 2 MiB in a debug build once 3.9 added to the path; deep query plans need room |
| 2026-11-05 | 3.11 folded into other phases: sharing a saved Space with contacts uses the existing shares, portfolios come with Save To (3.10), and photos lose their location data in public shares (3.8). M5 Pay and third-party plugins come last, after the core phases 3.4c–3.10 | Nothing in 3.11 but portfolios was new; the owner puts the phases that protect and show their own data before charging and before others' code |
| 2026-11-05 | M4 narrowed to borrowed compute: third-party plugins (and with them the plugin directory and signing) come later, one plugin at a time; drives over their APIs stay with 3.4c | Plugins come in kinds that run in different places (cards in the browser; sources and, in M5, payments on the node), so how third-party code runs is decided with the first real plugin of each kind rather than for all of them in advance |
| 2026-11-05 | Borrowed compute in the apps: only big tasks (100,000 rows or more) of collections shared with the lender go, the input read as the lender, and only when that is the whole of what the task needs; otherwise, and when the lender refuses or can't be reached, the same task runs here in the same isolated DuckDB | The owner doesn't want data shown to anyone it isn't shared with: what goes is what the lender may read already, so borrowing reveals nothing new, and a lender who may see only part never gets a task whose answer would be wrong; small tasks are faster here than over the network |
| 2026-11-05 | M3e: MCP is served by the node over HTTP at `/mcp` on the web API's loopback port, per assistant token, both protocol eras (2026-07-28 per-request metadata; 2025-11-25 and earlier after `initialize`, without sessions), three reading tools only; no `oikome-mcp` crate or stdio bridge | Claude Code connects to it with one `claude mcp add` command; no second process contends for the data directory; serving both eras lets clients on either SDK generation connect; the token in the client's settings is all it needs, and ending access stops it at once |
| 2026-11-05 | M3d: an AI assistant is a Principal with whole-Space grants, reaching the node with a token whose hash alone is kept; its search finds rows through the index and reads them again as itself, by key, with bound parameters | Grants, column grants and row policies apply to assistants as to anyone, without a second permission model; the index reads past them, so it only names rows; a token in an assistant's settings is all it needs, and losing the node's file gives nothing away |
| 2026-11-05 | M3a: the owner searches by full text only; meaning is left to the AI assistants that connect (no embedding model in the node). The node keeps a trigram FTS5 index for each table a source writes, maintained by triggers in the same transaction, and `search` goes one Space at a time, reading collections with no index | Full text needs no model, no download and no CPU spent on embeddings; trigram finds words in any language, Chinese included; triggers keep the index right whatever writes the table, without code in each write path |
| 2026-11-06 | 3.12: nodes cross networks without any relay. The router opens the node's one peer-to-peer port (UPnP, PCP, NAT-PMP) only while a share or lending the owner allowed is going on, asked first like an operating system's prompt (Deny / Allow; an answer without "Don't ask again" ends with its share or lending); otherwise a contact both sides reach introduces them (libp2p circuit relay v2, 16 KiB and 30 s a circuit, contacts only, only while reached directly) and they punch through to a direct connection (DCUtR). Only contacts get in from outside | An open-source project runs no servers, and owners shouldn't have to; a contact passing a few kilobytes of introduction is not a relay of data. Being reachable from the internet is the owner's call each time, as with any app that accepts connections |
| 2026-11-06 | 3.12: Arrow Flight between networks runs over a libp2p stream (`/oikome/flight/1`) with the same TLS pinned to the DID, not over a second mapped port; TCP only, no QUIC; reachability from the router's answer and the node's own addresses, not AutoNAT; ports mapped with rupnp and crab_nat, not `portmapper` | One port to open and one gate in front of it; UCANs, signed requests and Activity unchanged. QUIC would bring a second TLS stack (aws-lc) beside ring. Contacts may not be connected to confirm reachability. `portmapper` (and `igd-next`, and libp2p's `upnp`) pull in an MPL-2.0 HTTP client, outside the licence policy |

---

## 17. Security

Oikome keeps people's private data and moves it between them, so it has to say exactly what it protects, from whom, and what it does not. As an open-source project it aims at sound defaults that an ordinary owner can rely on without changing a setting, not at the strongest protection against every adversary. Owners who want less exposure turn on security mode (§17.4), which gives up some convenience. Stronger guarantees with a real cost are left for later (§17.6).

### 17.1 What Is Protected

| What | How | Where |
|------|-----|-------|
| Traffic between nodes | TLS by default; plaintext only with `--allow-http`. The certificate must carry the key of the DID the client expects: no CA, no host names. Every request is signed for the receiving node and accepted once | §9 |
| Traffic with a browser | DTLS against the fingerprint in a record signed by the node's DID; every request signed by the browser's DID | §10 |
| What a caller may read | Deny by default; the UCAN is verified before any capability; column grants and row policies inside the engine's SQL; ungranted tables do not exist; peers and Principals are read-only | §6, §13.4 |
| Files on a storage | Sealed on the client (XChaCha20-Poly1305, names from a keyed hash): always on a cloud drive, on a NAS when chosen | §7.5 |
| Passwords, tokens and storage keys | OS keychain, else an owner-only file; never in the registry, SQL, logs, snapshots or anything sent to a peer | §7.5, §11.4 |
| Plugins | A sandboxed frame without network; reads only the collections granted to it | §13.4 |
| The owner's oversight | Requests from peers, browsers and plugins, and the owner's first steps, are kept in Activity | §9 |

### 17.2 Who Can See What

| Who | Sees | Does not see |
|-----|------|--------------|
| A contact with a query share (`space/query`) | Result rows of the granted tables and columns | Anything else; anything after the share is revoked |
| A contact with a copy share (`space/read`) | Whole tables, copied to their node | Nothing more. The copy stays after revocation (§5.3); with security mode on, their node deletes it, as long as it runs an unmodified Oikome |
| A computer you borrow (§12) | The task and the input you send it | Your Spaces |
| A contact you lend this computer to | Their own task's result | Your Spaces: a compute grant gives no data access |
| An AI tool over MCP (M3) | The Spaces granted to it | Owner commands |
| Anyone on the same local network | That an Oikome node is there and its address; with security mode off, also its DID | Traffic, which is encrypted |
| A storage provider | Encrypted: how many snapshots and files there are and their rough sizes. An unencrypted NAS: everything | Contents and names of encrypted files |
| A relay, once nodes cross NATs (§9) | IP addresses, which keys talk to each other, when and how much | Contents, which are encrypted end to end; with security mode on, which DIDs the keys belong to |
| Someone with the computer or its disk | `~/Oikome` and the node key, unless the OS encrypts the disk | — |
| A script running on the browser node's origin | The seed in IndexedDB (§10) | — |
| A stranger with a public share (3.8) | What the owner made public, within the owner's limits per caller | Anything else; the owner's address, when served through a relay or a published snapshot |
| A buyer (M5) | What they paid for, for its period | Anything else; nothing new after a refund |
| A payment plugin (M5) | Orders: item, price, buyer's DID | Any data |
| A payment service | The payment, as for any payment: amount, accounts, the item's name | The data that was bought |

Out of scope: a compromised operating system or a tampered build of Oikome; someone who watches the network and infers who talks to whom from the traffic; quantum attacks (signatures and key exchange are not post-quantum).

### 17.3 Defaults

Always on, whatever the mode: TLS (`--allow-http` is for development), signed requests, UCAN checks, deny by default, the plugin sandbox, encryption on cloud drives and Activity.

With security mode off (the default):

- Local discovery (mDNS) is on.
- A share may allow copying; the apps say that a copy cannot be taken back.
- Borrowing compute is allowed; the apps say that the lender's computer sees what is sent to it.
- Crossing networks (3.12, §9): no relays, the project's or anyone's. The router is asked to open the node's one peer-to-peer port only while a share or lending the owner allowed is going on, asked each time unless told not to ask again. Only contacts get in from outside. A node reached directly helps its contacts meet (a few kilobytes a meeting, never their data) unless the owner turns it off. Addresses are never published to a public DHT or DNS; they travel in invites and between contacts. A public share may be listed where people find it, if the owner chooses (§13.4).
- On the first run the apps say so when the OS reports the disk unencrypted (FileVault on macOS, BitLocker on Windows).

### 17.4 Security Mode

One switch per node, off by default: Settings › Security in the apps, `oikome security on|off` on the command line, `security_mode` in `.oikome/node.json`.

**Security mode never turns a feature off.** Every feature works with it on as with it off. It adds protection to how data moves and how it is analysed; where software alone cannot protect something, it makes the exposure visible and recorded.

| | Off | On |
|---|-----|----|
| Local discovery | mDNS announces the node's DID and address | mDNS announces a random identifier that changes; the node says who it is only to a node that proves it is a contact |
| Contacts' queries | In Activity: who, which Spaces, allowed or refused | Also the SQL as the owner's semantic layer ran it, and the number of rows returned |
| Copy shares (`space/read`) | The copy stays on the contact's node after revocation (§5.3) | The copy is sealed on the contact's node under a key of its own. The contact's node deletes the copy and its key when it learns of the revocation at its next refresh, and in any case when the token ends. The share carries this requirement (UCAN `nb`), so the owner's mode decides, whatever the contact's |
| Borrowing compute (§12) | The task's input is sent as the task needs it | Only the columns the task reads are sent. The lender's node runs the task in memory with external access off, deletes the input and the result once it has returned it, and signs a receipt. Both sides record what was sent (columns, rows, hash). The apps show whether the lender runs security mode too |
| Crossing networks (3.12) | The DID's key is the network identity | A network key apart from the DID, changed regularly and bound to it by a signature only contacts see, so a contact that introduces two others, or anyone watching the opened port, cannot link the traffic to the identity |
| Share and invite lifetime | Chosen by the owner, 1 to 366 days | 30 days unless the owner picks longer; the apps remind the owner before a share ends |
| Keys and tokens on this computer | The node key in the OS keychain (3.7); invites and borrowed grants in owner-only files | Those files are also sealed with a key kept in the OS keychain |
| Disk not encrypted | A warning on the first run | The warning stays on Home until the OS reports the disk encrypted |
| Public shares (3.8) | Served as the owner set them | Served only as a published snapshot, never from the node's own address; strangers' queries are recorded with their SQL |
| `--allow-http` (development) | Accepted | Refused |

- **What it relies on.** Deleting a copy and discarding a borrowed task's input are done by the other node's Oikome. A modified build there could keep them, and nothing in software can prove otherwise. The apps say so where they offer a copy share or a borrowed computer.
- Turning it on applies to what the node does next. It cannot undo copies already made or addresses peers have already seen.
- The apps ask before turning it off.
- It is set per node. A policy that sets it for a whole team comes with teams.

### 17.5 Known Gaps

| Gap | Plan | When |
|-----|------|------|
| The node key is an owner-only file | Kept in the OS keychain where there is one, as storage credentials are; the file remains for servers and containers | 3.7 |
| The browser node's seed is readable by scripts of its origin | A non-extractable WebCrypto Ed25519 key (§10) | 3.7 |
| Local discovery announces the DID on any network | With security mode on, a changing random identifier instead, and the DID only to contacts (§17.4) | 3.7 |
| No published way to report a vulnerability | `SECURITY.md`: private reporting and supported versions; this section is the published threat model | 3.7 |
| An opened port can be found by anyone scanning the internet, and the network identity is the DID | Only contacts get past the handshake, and every request is still checked (§9). The network key of §17.4 | 3.7 |
| A node whose router opens nothing and has no contact that is reached directly can't be reached from outside | Settings › Network says so and suggests UPnP or the owner's own VPN (Tailscale, WireGuard); PCP's IPv6 firewall opening and QUIC hole punching (UDP holes open more often than TCP ones) | Later |

### 17.6 Left for Later

Each has a real cost and is outside the open-source scope for now; some may be offered later as services.

- A third-party security audit.
- Managed relays with guaranteed capacity.
- Relaying in two hops run by different operators, or over Tor, so that no single relay knows who talks to whom.
- Post-quantum key exchange.
- Hardware-backed keys. The Secure Enclave holds only P-256 keys, so it would wrap the seed rather than hold it.
- Reproducible builds.
- A team-wide policy that turns security mode on for every member.

---

*This document is the canonical architecture of Oikome (v3.0). All engineering implementation proceeds from this specification. Earlier versions (v1, v2) are kept only in the git history.*
