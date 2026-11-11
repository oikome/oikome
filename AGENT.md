# AGENT.md — Oikome Development Collaboration Protocol

> **Purpose**: Defines the rules for collaboration between human developers and AI agents on the Oikome project.
> **Document language**: Collaboration documents (this file, issues, PR descriptions) are in **English**; plans and replies to the user are in **Chinese**.
> **Code language**: All source code, comments and identifiers are in **English**.
> **License**: Apache-2.0 (in preparation for a later donation to the Apache Software Foundation).

---

## 1. Project Overview

Oikome is an **AI-native, multimodal private data center** that usually runs on personal resources and supports trusted data exchange.

- **Private**: data belongs to its owner (a person, a family or a small team), lives on storage the owner controls (local disk, NAS, cloud drives), and is brought in from apps by connectors.
- **Multimodal**: text, images, audio, files and vectors are first-class data, just like tables.
- **AI-native**: AI tools and agents connect through MCP like any other app and see only the data granted to them; data is stored in forms AI can use directly (text, vectors, search indexes); views and names tell AI what the data means; computation such as embedding and inference also runs on personal resources.
- **Personal resources**: storage and compute come from individuals, not from a platform's cloud: first one's own machines, and when they are not enough, other individuals' machines through compute sharing.
- **Trusted data exchange**: data flows in both directions between data centers that trust each other, under revocable grants; the owner can also open chosen data to anyone (strangers included), accessed within limits the owner sets (public shares).
- **Web2 and Web3**: data comes from the apps people use, and can also come from chains (an address's transactions and tokens); a wallet can serve as an identity (`did:pkh`). No token, no chain of its own, no private data on a chain, and no wallet is needed to use Oikome.
- **Paid access**: the owner can charge for chosen data, paid through a payment plugin the owner picks (PayPal, Alipay, Web3 wallets, etc.) straight into the owner's own account; Oikome never handles funds, and payment is only the precondition for issuing a grant.

Data exchange and compute sharing use the same trust mechanism (DID, peer relationships, deny by default, UCAN revocation), with separate authorization (data: `space/query` / `space/read`; compute: `compute/<method>`); neither includes the other.

Underneath is a **decentralized local federated query semantic layer** — embedded, sovereign, P2P-native.

Oikome is responsible for "who can see what, where data comes from, where it is stored, and where queries and tasks are sent", **not for "how to compute"**: computation is delegated to pluggable embedded engines (DuckDB for analytics, SQLite for transactions; in the browser, provided by the JS host), and DataFusion acts only as the federation coordinator that merges the Arrow results of several engines. The spec is `docs/architecture.md`.

- **Core language**: Rust (compiles to WASM, memory-safe)
- **Compute engines**: DuckDB 1.5.6 (analytical Spaces; Lance Spaces are read, written and searched through the `lance` extension, graph queries go through `duckpgq`, extensions are downloaded with `oikome extensions install`), SQLite (transactional Spaces; FTS5 full-text search, sqlite-vec vector search)
- **Federation coordination**: Apache DataFusion + `datafusion-federation` (merges only, reads no files)
- **Catalog system**: Apache Iceberg (analytical Spaces) + the Space/Location registry under `.oikome/`
- **Storage formats**: Apache Parquet (analytical, the P2P sync unit), SQLite files (transactional), Lance (vector / full-text / large objects, 3.3)
- **In-memory format**: Apache Arrow
- **Network transport**: Arrow Flight (gRPC), TLS certificates bound to the node DID; libp2p only for node discovery (mDNS); browsers connect to a Full Node over a WebRTC data channel (str0m, DTLS fingerprint vouched for by a DID-signed access record)
- **Cryptography**: ed25519-dalek + Blake3 (3.1)
- **Browser side**: `oikome-wasm` (wasm-bindgen) + duckdb-wasm (OPFS) + IndexedDB + WebRTC (`packages/node/src/transport.js`); federated queries run as a single task in the browser (`Coordinator::single_task`); build with `packages/node/scripts/build.sh` (`npm run build` at the repository root) (`wasm` profile; needs a clang that can compile wasm32, Homebrew LLVM on macOS)

### Architecture Layers (Bottom Up)

```
L0: Identity and network layer (DID, UCAN, libp2p, Arrow Flight, snapshot sync, compute sharing)
L1: Catalog and storage layer (Space/Location registry, Iceberg metadata, Parquet write path, VFS: local disk / NAS / S3 / cloud drives)
L2: Engine abstraction layer (QueryEngine trait: DuckDB | SQLite | host engine)
L3: Federation layer (DataFusion coordinator: pushes SQL down per engine, merges Arrow streams)
L4: Semantic layer (<space>.<table> naming, routing by statement class, view / UCAN policy rewriting)
L5: Interface layer (SQL API, HTTP, WASM, CLI, MCP) + connectors (Connector: apps' official APIs, export files, chains' public RPC)
```

### Design Principles (Inviolable)

1. **Compute is delegated**: do not implement scan, join, aggregation or index algorithms; if an engine cannot do something, switch engines or configure the engine; do not write operators.
2. **Single write master per Space**; write permission is delegated explicitly through UCAN.
3. **Space (logical) and Location (physical) are separate**: several Spaces may bind the same Location, at most one of them writable.
4. **Only immutable files cross the network**: P2P syncs only Iceberg snapshots and Lance versions; transactional Spaces are shared externally by publishing Parquet snapshots.
5. **Deny by default**: no peer relationship = no catalog visibility = no query permission. A public share is the owner's explicit grant to anyone; without a public share, a stranger sees nothing.

---

## 2. Communication Rules

### 2.1 Language Policy

| Context | Language |
|---------|----------|
| This file (AGENT.md) | English |
| Development plans / roadmap | Roadmap (in this file): English; plans shown to the user: Chinese |
| Product requirements (`docs/prd.md`) | English |
| Code comments | English |
| Source code identifiers | English |
| Commit messages | English, **one line only** (e.g. `feat(semantic): add views and row/column policies`), no body and no trailers of any kind |
| Issue / PR descriptions | English |
| API docs (docstrings) | English |
| User-facing docs | English |

### 2.2 Agent Startup Protocol

Before each development session starts, the agent **must**:

1. Read `AGENT.md` (this file).
2. Read `docs/architecture.md` for architecture context; requirements for the user-facing apps (web, later desktop and phone) are in `docs/prd.md`, and the prototype in `docs/prototype/` is authoritative for the interface.
3. Check the current git branch and status.
4. Review any pending tasks/plans for this session.

**Local toolchain**: `rust-toolchain.toml` pins stable. DuckDB is not compiled: builds link the official prebuilt libduckdb, downloaded once into `target/duckdb-download/` (`DUCKDB_DOWNLOAD_LIB=1` in `.cargo/config.toml`). On macOS, tests and a locally built `oikome` find it through `DYLD_LIBRARY_PATH` set to that download's directory.

---

## 3. Apache Foundation Compliance Requirements

To keep the project eligible for donation to the Apache Software Foundation, all code **must** follow these rules:

### 3.1 License Header

Every source file must include the standard Apache-2.0 header:

```rust
// Copyright 2025 The Oikome Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
```

### 3.2 Dependency Policy

- **Permissive licenses only**: Apache-2.0, MIT, BSD-2/3-Clause, ISC
- **Forbidden**: GPL, LGPL, AGPL, SSPL, proprietary software, "non-commercial" licenses
- **Preferred**: for equal functionality, prefer Apache-2.0 over MIT
- **Documentation**: record all dependencies and their attribution in the `NOTICE` file
- **Automated check**: `cargo deny check licenses` (rules in `deny.toml`); must pass after adding a dependency

### 3.3 Governance

- Keep commits few: combine commits by roadmap phase or complete feature, not split by sub-step
- No anonymous contributions; every author's identity must be identifiable
- Do not copy code from Stack Overflow or other sources without verifying its license
- No binary blobs in the repository
- Write PR bodies following `.github/PULL_REQUEST_TEMPLATE.md` (Summary, Related issue, Documentation impact, Validation, Checklist); the title is one Conventional Commits line

### 3.4 Code of Conduct

- Use inclusive and professional language in all communication
- Constructive criticism; no personal attacks
- Respect different backgrounds and skill levels

---

## 4. Development Workflow

### 4.1 Phased Implementation

The project follows a strict phased roadmap:

v3 roadmap (details in `docs/architecture.md` §14):

| Phase | Goal | Status |
|-------|------|----------|
| 3.0-A Engines | `QueryEngine` abstraction, DuckDB/SQLite engines, Space/Location catalog, single-Space queries pushed down whole | ✅ |
| 3.0-B Federation | DataFusion coordinator + `datafusion-federation`, cross-Space join, snapshot pinning | ✅ |
| 3.0-C Semantic layer | Views, column grants and row policies (SQL rewriting), principals read-only + deny by default, enumeration resistance | ✅ |
| 3.0-D Publishing | `PUBLISH SPACE`: SQLite → Iceberg snapshot (overwrite), scheduled publishing | ✅ |
| 3.1 Network | DID/UCAN, Arrow Flight remote tables, libp2p discovery, snapshot subscription | ✅ (3.1a remote queries, 3.1b snapshot subscription, 3.1c node discovery and encrypted transport, 3.1d three Docker nodes passing in CI) |
| 3.2 Browser | WASM core + host engine bridge (duckdb-wasm); federated queries through a Full Node over a WebRTC data channel | ✅ 3.2a local queries; 3.2b WebRTC and federated queries |
| 3.3 Multimodal | Lance Locations (through DuckDB's `lance` extension: vectors, full text, large objects), DuckPGQ, SQLite FTS5/sqlite-vec | ✅ 3.3a Lance Spaces; 3.3b SQLite full-text and vector search; 3.3c DuckPGQ (on Lance Spaces only) |
| 3.4a NAS | Spaces on a NAS at home (WebDAV or a mounted folder): the writing node pushes a snapshot after each write, other nodes pull the newest complete snapshot into a local copy; one writing node per folder; SQLite not on a NAS; passwords kept by the node (see 3.4b) | ✅ |
| 3.4b Cloud drives | Client-side encryption (every file and file name encrypted before upload); drives connected through their desktop client's sync folder (iCloud Drive, Dropbox, OneDrive, Google Drive; `--drive` must be encrypted); credentials and keys kept by the node (OS keychain, a private file when there is none), entered once by the user in the UI or on the command line | ✅ |
| 3.4c Drive APIs | Google Drive and Dropbox over their APIs for nodes without a desktop client, always encrypted: opendal 0.59.5; Google sign-in with the authorization code + PKCE back to the web app's loopback address, Dropbox with PKCE + offline access and its authorization code copied from Dropbox's page into Oikome (no callback); the OAuth client is the owner's own app, entered once and remembered by the node; refresh tokens kept with the node's secrets and forgotten on removal, never revoked; Google `drive.file`, Dropbox's file scopes, with Full Dropbox access for viewing other folders in place; the drive's room from its API. Large uploads use resumable sessions, Google's in `third_party/`, Dropbox's start/append/finish in `oikome-vfs`. OneDrive stays on its sync folder | ✅ Google Drive; Dropbox sign-in and uploads verified; in-place and second-node reads awaiting verification |
| 3.4d Baidu Netdisk | Write a `baidu-netdisk` service in Oikome's fork of apache/opendal, following the structure of `aliyun-drive` (upstream issue #8481), used by Oikome as a git dependency, encrypted the same way: OAuth sign-in; folders under `/apps/Oikome/`; back off and retry when rate-limited; open the upstream PR after the combined real-account testing | Not started |
| 3.4e Object storage | RustFS only, over the S3 protocol through opendal `s3`: keys kept by the node; encrypted by default, a RustFS the owner runs may be unencrypted; multipart upload, range reads | ✅ Done |
| 3.5 Connectors | `Connector` contract (`oikome-connect`), upsert by the source's primary key + cursor committed in the same transaction (cursor in the Space's hidden table `_oikome_source`), scheduled and manual pulls, tokens kept by the node; first batch: CSV, flomo export file, flomo MCP (code written, waiting for a real test with a MAX account) | ✅ |
| 3.6 Compute sharing | `compute/<method>` UCAN capabilities with `nb` limits (separate from data grants); only Oikome's existing compute methods, opened one by one by the provider (first: `sql`, an isolated DuckDB per task); request-response over Arrow Flight `DoExchange`; per-peer quotas | ✅ |
| 3.7 Security | Security mode (architecture §17.4: when on, features work as usual, it only adds protection) in the node, the API (`security_mode`, `set_security_mode`, owner only) and Settings › Security; the warnings of §17.3; the node private key in the OS keychain; the browser node's private key non-extractable; disk encryption check on first run; `SECURITY.md`. Its network part is a network key apart from the DID, for contacts that introduce others (3.12) | Not started |
| 3.8 Public shares | Shares to anyone (architecture §13.4): query-only by default, limits per caller, access by any DID (including a wallet's `did:pkh`); read-only public snapshots with a signed manifest, on a storage or on IPFS / Arweave; photos in a public share lose their location data by default. Live queries by strangers wait for cross-NAT connections | Not started |
| 3.9 Viewing in place | Files in local folders and on each storage are browsed, previewed and queried in place, without copying: in-place Spaces are read-only, one table per CSV / Parquet / JSON file; the index the node keeps (files, text, thumbnails) is a cache and can be rebuilt if deleted; previews use range reads; read access to the whole drive is requested when needed; M2's folder sources become in-place Spaces | ✅ (folders on this computer and on a NAS read where they are; the index is a SQLite cache with thumbnails and full text; the folder sources of photos and notes became folders viewed in place) |
| 3.10 Save To | The owner saves on purpose, to plain files or a new Space: folder rules and naming rules; once or continuing; a rule changes only the files it wrote itself; a saved Space keeps a portfolio (order, cover, title, captions) that travels with it when shared (3.11 was folded into 3.8 and 3.10) | Not started |
| 3.12 Reaching home from anywhere | Crossing networks without any relay: the router opens the node's one peer-to-peer port (UPnP, PCP, NAT-PMP) while a share or lending the owner allowed needs it, asked first (Deny / Allow, "Don't ask again"); otherwise a contact both sides reach introduces them and they hole-punch to a direct connection; only contacts get in from outside; addresses signed in invites and between contacts (no DHT or DNS); Flight over a libp2p stream with TLS pinned to the DID; Settings › Network | ✅ (in process on one computer, and in Docker behind Linux NAT routers with miniupnpd in CI; a real home router is still to be tried) |
| 3.13 Serving over WebDAV | Collections and saved files served over WebDAV, read-only by default, so other apps open them like a drive | Not started |

User-facing track (in parallel with the core phases above, neither blocks the other; each milestone waits for its prerequisite phases to finish, details in architecture §14.1):

| Milestone | Goal | Prerequisites | Status |
|-------|------|------|----------|
| M1a Layout split | `web/` split into `packages/node` and `e2e/`, root `package.json` with npm workspaces; browser-e2e passes unchanged | 3.2 | ✅ |
| M1b API | `oikome-api` (collections, tables and rows, SQL, contacts, sharing and revoking, access log), whole-Space grants (`GRANT SELECT ON SPACE`), access log, `oikome serve --web` and `oikome web open`, `packages/api`, `packages/client` (HTTP implementation); the recovery key and phone pairing are designed separately before M1c | 3.0–3.2, M1a | ✅ |
| M1c Usable | `packages/ui`, `packages/screens`, `apps/web` (React 19 + Vite 7): first launch and recovery key, Collections, sharing and contacts, Activity, SQL workbench, settings, desktop and phone layouts; `--web-root`; `oikome identity restore`. Phone pairing moves to the phone app | M1b | ✅ |
| M2 Connect | Sources screen and sync status, Home dashboard, built-in cards, dashboard plugins v1 (`plugin-sdk`, data contract, plugin grants), reference plugins | 3.5, M1c | ✅ (PR #11) |
| M3 Find | Full-text search, photo and note views, AI Assistants screen, MCP over HTTP | 3.3, M2 | ✅ (M3a–M3e: full-text search, search in the web app, photo and note views, assistant grants, MCP and the AI Assistants page) |
| M4 Extend | Borrowed compute in the apps (big summaries of collections shared with the lender only); third-party plugins and the plugin directory moved out, to come one plugin at a time; drives over their APIs stay with 3.4c | 3.4b, 3.6, M3 | ✅ |
| M5 Pay | Pricing in the UI (content, by duration or per use, query-only or copyable); payment plugins and order API (architecture §13.4); purchase flow for contacts and strangers; orders recorded in Activity | 3.8, M4 | Not started; comes last, with third-party plugins |
| M6 Desktop app | Tauri 2 app running the Full Node in process with the same screens; the web app embedded in the binary; installers for macOS, Windows, Linux | M4 | Not started |
| M7 Phone app and devices | Tauri 2 phone app: pairing by QR code, My Devices / Add a Device, reading Home and collections over WebRTC (away from home once 3.12 is done) | M6, 3.12 | Not started |
| M8 Languages | The apps in nine languages, the prototype's; picked per device; examples and Oikome's plugins in the language; user data never translated | M1c | ✅ Done |
| M9 Family | Family members with their own identity and what each may see | M7 | Not started |
| M10 Plugins from others | Third-party source and card plugins, one at a time, with signing and a plugin directory | M4 | Not started; last, with M5 |

**Order (2026-11-11).** Numbers name the phases; they are not the order. Done: 3.9 (so nothing is copied into Oikome just to be seen), 3.12 (brought forward), M8 (brought forward), 3.4e and 3.4c (Google Drive; Dropbox sign-in and uploads verified; in-place and second-node reads awaiting verification). Next: working with data in the apps (explored first), then 3.8, 3.7, 3.10, M6, M7, M9, 3.13. Last: M5 and M10.

User-facing tests always connect to a local node (temporary data directory, test data written through the command line or the API), with no fake-data layer and no mock client; so the API (M1b) comes before the screens (M1c), and screens such as Sources, the Home dashboard and the AI assistant come with M2 and M3.

**Rule**: No skipping phases. A phase's exit criteria must be met before moving to the next phase. Phases that need accounts are written first, and once all of them are written they are tested together against real accounts, including 3.4c–3.4e, the cloud drives in 3.9, and 3.5's flomo MCP.

### 4.2 Task Execution Protocol

Each task follows this flow:

1. **Read AGENT.md** (this file).
2. **Read the relevant architecture sections** in `docs/architecture.md`.
3. **Communicate and design** — **before writing code, discuss the design fully with the user; implementation starts only after the user confirms it. Coding without confirmation is forbidden.**
4. **Make a plan** — if the task spans several files or is complex, use Plan Mode to write a detailed implementation plan and get the user's approval.
5. **Write tests first** — TDD first, especially for storage and query engines.
6. **Implement** — with idiomatic, concise Rust.
7. **Run tests** — unit tests, integration tests and relevant benchmarks.
8. **Verify Apache compliance** — license headers, dependency checks.
9. **Request code review** — review is required before merging.

### 4.3 Skills Usage Rules

Claude Code provides various skills to standardize the development process. The rules below are in two levels: mandatory (MUST) and recommended (SHOULD).

#### Mandatory (MUST) — do not proceed unless the condition is met

| Trigger | Required Skill | Notes |
|---------|----------------|------|
| Before creating a new feature, component or module | `superpowers:brainstorming` | First explore requirements, design constraints and feasible options |
| Multi-step implementation tasks (across files or complex logic) | `superpowers:writing-plans` | Write a detailed implementation plan; execute only after the user confirms it |
| Executing a written plan | `superpowers:executing-plans` | Execute in stages, with review checkpoints |
| Feature development (may change several files) | `superpowers:using-git-worktrees` | Use an isolated git worktree so the main branch is not polluted |
| Implementing any feature or bug fix | `superpowers:test-driven-development` | Write tests first, then the implementation code |
| Hitting a bug, test failure or unexpected behavior | `superpowers:systematic-debugging` | Debug systematically; no fixes based on guesswork |
| Before claiming a task is done, tests pass or it is ready to merge | `superpowers:verification-before-completion` | Run verification commands and confirm the output; evidence before assertions |

#### Recommended (SHOULD) — judge by situation

| Situation | Recommended Skill |
|------|-----------|
| Researching external technical details such as the Iceberg / Parquet / Arrow specs | `deep-research` |
| 2+ independent tasks that can be handled in parallel | `superpowers:dispatching-parallel-agents` |
| Subagents can carry out implementation subtasks independently | `superpowers:subagent-driven-development` |
| Requesting code review after finishing a task | `superpowers:requesting-code-review` |
| Handling code review feedback | `superpowers:receiving-code-review` |
| Deciding how to integrate a finished development branch | `superpowers:finishing-a-development-branch` |
| Reviewing the current diff for correctness and efficiency issues | `code-review` |
| Simplifying code, improving reuse and efficiency | `simplify` |
| Running the app to verify a change actually works | `run` / `verify` |
| Security-related code in Phase 2+ | `security-review` |

#### Not applicable (NOT APPLICABLE)

The following skills are unrelated to the Oikome project and **must not** be used:

- Frontend/UI: `frontend-design`, `impeccable`, `design-taste-frontend`, `baseline-ui`, `high-end-visual-design`, `minimalist-ui`, `industrial-brutalist-ui`, `ui-ux-pro-max`, `fixing-accessibility`, `fixing-metadata`, `fixing-motion-performance`, `gpt-taste`, `image-to-code`, `imagegen-frontend-web`, `imagegen-frontend-mobile`, `redesign-existing-projects`, `stitch-design-taste`, `brandkit`
- Figma: `figma:figma-use`, `figma:figma-generate-design`, `figma:figma-generate-diagram`, `figma:figma-generate-library`, `figma:figma-implement-design`, `figma:figma-code-connect`, `figma:figma-create-design-system-rules`, `figma:figma-use-figjam`
- Other: `claude-api`, `init`, `keybindings-help`

---

### 4.4 Code Quality Standards

- **Rust idioms**: follow `rustfmt` and `clippy` (no warnings allowed in CI).
- **Error handling**: use `thiserror` or `snafu` for structured errors; no `unwrap()` in production code.
- **Documentation**: every public API must have rustdoc comments.
- **Unsafe code**: keep it minimal; every `unsafe` block must have a safety comment stating its invariants.
- **Testing**: coverage target >80% for core modules (storage, catalog, execution).
- **Benchmarks**: use `criterion.rs` for performance-critical paths.

### 4.4 Module Organization

```
oikome/
├── Cargo.toml                  # workspace root; all shared dependency versions locked together in [workspace.dependencies]
├── Cargo.lock                  # committed (duckdb locked to an exact version)
├── rust-toolchain.toml         # stable + rustfmt + clippy
├── deny.toml                   # cargo-deny license allowlist
├── LICENSE / NOTICE            # Apache-2.0 / dependency attribution
├── AGENT.md                    # this file
├── crates/
│   ├── oikome-core/           # errors, SpaceUri, SpaceKind/EngineKind/AccessMode
│   ├── oikome-vfs/            # OpenDAL VFS; RemoteStore: a Space's folder on a storage (WebDAV, mounted folder)
│   ├── oikome-connect/        # connectors: Connector contract, CSV, flomo (export file, MCP)
│   ├── oikome-catalog/        # Iceberg Catalog + Space/Location registry + snapshot resolution
│   ├── oikome-storage/        # Parquet write path, Compaction
│   ├── oikome-engine/         # QueryEngine trait, TableSource, BlockingWorker
│   ├── oikome-engine-duckdb/  # analytical engine (bundled DuckDB)
│   ├── oikome-engine-sqlite/  # transactional engine (SQLite rows → Arrow)
│   ├── oikome-semantic/       # Oikome statement parsing, statement classification, view expansion and permission rewriting
│   ├── oikome-federation/     # DataFusion coordinator, EngineSqlExecutor
│   ├── oikome-node/           # OikomeSession, EngineRegistry, routing, subscribed copies
│   ├── oikome-identity/       # DID, UCAN issue / delegate / revoke, request signing
│   ├── oikome-network/        # inter-node protocol, PeerClient, FlightEngine
│   ├── oikome-discovery/      # libp2p node discovery (mDNS, exchanging Flight endpoints), written to the Peers table
│   ├── oikome-protocol/       # inter-node protocol types and request digests, WebRTC access records and framing (shared by native and browser)
│   ├── oikome-webrtc/         # WebRTC endpoint for browsers (str0m): sessions, DTLS certificate, access records
│   ├── oikome-wasm/           # browser node: BrowserNode, HostEngine (duckdb-wasm), RemoteEngine, JS interface
│   ├── oikome-api/            # unified interface for the layers above (no transport), callers are owner or principal; generates the types in packages/api
│   └── oikome-server/         # Arrow Flight service, the owner's web API (serve --web), the oikome command line
├── tests/                      # oikome-tests (TPC-H Q1–Q5) + tools/data-gen + e2e/ (end-to-end scripts for Docker nodes)
├── docker/                     # node image (oikome.Dockerfile) and three-node compose
├── package.json                # npm workspaces: packages/*, e2e
├── apps/
│   └── web/                    # web app (React + Vite), served by oikome serve --web --web-root apps/web/dist
├── packages/
│   ├── api/                    # API types generated from oikome-api (@oikome/api)
│   ├── client/                 # NodeClient interface and HTTP implementation (@oikome/client)
│   ├── screens/                # all screens and UI text, shared by every client (@oikome/screens)
│   ├── ui/                     # design system: tokens and components (@oikome/ui)
│   └── node/                   # JS host for the browser node (duckdb-wasm, IndexedDB, WebRTC) and wasm build script (@oikome/node)
├── e2e/                        # Playwright: tests for the browser node, web API and web app; browser node test page in harness/ (@oikome/e2e)
├── third_party/                # patched third-party code (datafusion-federation, see README)
├── .github/workflows/          # docker-e2e (three Docker nodes), browser-e2e (headless Chromium running the browser node)
└── docs/                       # architecture.md (canonical architecture), prd.md (user-facing PRD), prototype/ (clickable design prototype prototype.html, translations i18n/, photos lisbon/, design references references/)
```

Planned: `oikome-connect` (3.5), `oikome-cli`.

Planned for the user-facing apps (directories etc. are created only when the matching work starts; layout in architecture §13.3):
- Root: desktop and mobile (later) under `apps/`, `packages/plugin-sdk` (M2), `plugins/` (M2), `design/` (later). `apps/*` and `plugins/*` are added to workspaces once created.
- M1a has already split `web/` into `packages/node` and `e2e/`, and added `package.json` at the root.

---

## 5. Naming Conventions

### 5.1 Code Identifiers

| Item | Convention | Example |
|------|------------|---------|
| Crate names | `kebab-case`, prefix `oikome-` | `oikome-catalog` |
| Module names | `snake_case` | `iceberg_catalog` |
| Type names | `PascalCase` | `SchemaRegistry` |
| Function names | `snake_case` | `commit_snapshot` |
| Constants | `SCREAMING_SNAKE_CASE` | `DEFAULT_ROW_GROUP_SIZE` |
| Error types | `PascalCase` + `Error` suffix | `CatalogError` |
| Trait names | `PascalCase` | `SpaceResolver` |
| Generic parameters | Single uppercase letter | `T`, `K`, `V` |

### 5.2 Domain Terms

Use these exact spellings in code:

| Concept | Code identifier | Notes |
|---------|-----------------|---------|
| Space | `Space` | Sovereignty unit (logical), SQL qualified name `<space>.<table>` |
| Location | `Location` | Physical residency (Iceberg namespace, SQLite file or Lance dataset), can be bound by several Spaces |
| SpaceKind | `SpaceKind` | `Analytical` (Iceberg + Parquet) / `Transactional` (SQLite) |
| Engine | `QueryEngine` / `EngineKind` | Compute engine (DuckDB / SQLite / host) |
| Node | `Node` | Peer entity |
| Catalog | `Catalog` | Local metadata registry |
| Snapshot | `Snapshot` | Immutable Iceberg snapshot |
| Manifest | `Manifest` | File manifest |
| UCAN | `Ucan` / `UCAN` | Capability token |
| DID | `Did` / `DID` | Decentralized identity |
| VFS | `Vfs` / `VFS` | Virtual file system |
| OPFS | `Opfs` / `OPFS` | Origin private file system |
| CID | `Cid` / `CID` | Content identifier |
| FTS | `Fts` / `FTS` | Full-text search |

---

## 6. Testing Strategy

### 6.1 Test Levels

```
Unit tests          → per module, fast, mocked dependencies
Integration tests   → across crates, real I/O (temporary files)
Benchmarks          → criterion.rs, track performance regressions
Conformance tests   → TPC-H queries verify SQL correctness
End-to-end tests    → Docker-based multi-node scenarios; Playwright (e2e/) for the browser node and user-facing apps
```

### 6.2 Test Data

- Unit tests use deterministic synthetic data.
- TPC-H integration tests use SF 0.001; the data is generated by `cargo run -p test-data-gen -- --scale-factor 0.001 --force` into `tests/oikome-local` (gitignored, not committed).
- Tests that need the network (e.g. the DuckDB `sqlite` extension) are marked `#[ignore]` and run separately with `--features sqlite-scanner-tests -- --ignored`.
- User-facing (web, desktop, phone) tests connect to a local node: e2e starts `oikome serve` with a temporary data directory and writes test data through the command line or the API; no fake-data layer and no mock `NodeClient`.
- Real user data must not be used in tests.

### 6.2.1 Local Build Directory

`target/` does not shrink by itself: for every new combination of packages and features (`-p a` vs `-p b`, tests vs binaries), Cargo compiles another copy of the dependencies and test programs, the old ones stay, and every test program links DuckDB, DataFusion and Lance. On 2026-11-02 it grew to 183 GB this way and filled the disk.

- For builds and tests, prefer `--workspace`, then filter with `--test`, `--lib`, `--bin`, rather than switching `-p` each time, so the dependencies' feature combination stays the same. `.cargo/config.toml` has aliases for the fixed forms: `cargo node` (builds `oikome`), `cargo t --test api` (runs one test file; `cargo t --lib -- <name>` runs unit tests by name), `cargo lint` (clippy on all targets, warnings as errors).
- With consistent usage and merged tests, `target/` is usually around 20 GB and needs no routine cleaning. Above 40 GB, or after upgrading Rust (all artifacts of the old version become invalid), run `cargo clean`; the cost is a full rebuild next time (ten-odd minutes to 40 minutes). Do not use `cargo sweep --maxsize` or `--time`: it counts the incremental compilation cache toward the total but never deletes it, so it deletes dependencies still in use instead (on 2026-11-04 it deleted everything not used in the last 80 minutes, and DataFusion and Lance both had to be rebuilt), and dependencies in constant use were compiled early, so by time they look "old" too. Before a full verification run, check the size of `target/`; above 40 GB, tell the user and let the user decide whether to clean. Each git worktree has its own `target/`.
- The debug profile for development keeps only line-number debug info (for both dependencies and this repository's crates), so error stacks still have files and line numbers; when you need a debugger to inspect variables, temporarily add `--config profile.dev.debug=true` to the command.

### 6.3 CI Requirements

- `cargo fmt --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --workspace`
- `cargo doc --no-deps` (ensure no broken links)
- License header check (via `apache-skywalking-eyes` or a similar tool)
- Dependency license audit (`cargo deny check licenses`)
- Browser node: `.github/workflows/browser-e2e.yml`, builds wasm and `oikome`, then runs `e2e/` with Playwright in headless Chromium (`remote.spec.mjs` starts its own `oikome serve --webrtc-listen`)
- Network end-to-end: `.github/workflows/docker-e2e.yml`, three Docker nodes plus a WebDAV service (rclone, acting as a NAS) run `tests/e2e/docker_nodes.sh` (triggered on pushes to `feat/**` and `ci/**` and on PRs to `main`)

---

## 7. Security and Cryptography Rules

### 7.1 Use of Cryptography

- **Ed25519**: use the `ed25519-dalek` crate for signing/verification.
- **Hashing**: use `blake3` for content addressing and checksums.
- **Randomness**: use `rand::thread_rng()` or `getrandom`; `Math.random()` is strictly forbidden in WASM.
- **Key storage**: never log private keys; store them in the OS keychain or encrypted OPFS. Stored passwords, tokens and encryption keys are kept by the node: in the system keychain on macOS / Windows, and on machines without a keychain in `.oikome/identity/storage_access.json` (0600, beside the node private key); the environment variables `OIKOME_STORAGE_<NAME>_*` take precedence, for deployments; `OIKOME_SECRET_STORE=file` keeps credentials in that private file even where there is a keychain, for data centers made to develop or test with (a rebuilt binary is a new program to the keychain, which asks again for every credential). Tests use only in-memory or file stores and never touch the machine's keychain: the node, API and server tests configure `MemorySecrets`, and the browser tests run nodes with `OIKOME_SECRET_STORE=file`. The node private key is currently a 0600 file and moves into the system keychain from 3.7; servers and containers still use a 0600 file.

### 7.2 UCAN Implementation

- JWT format, standard claims (`iss`, `aud`, `att`, `exp`, `prf`).
- Signatures must be verified before any capability check.
- Short-circuit on expiry; revoked tokens are not processed further.
- The CRL (certificate revocation list) is propagated over P2P.

### 7.3 P2P Security

- Deny by default: no peer relationship = no catalog visibility = no query permission. Public shares are the exception: they are the owner's explicit grant.
- Enumeration resistance: unauthorized tables return "does not exist".
- Every Arrow Flight RPC carries UCAN metadata.

### 7.4 Protections That Cannot Be Made Switchable

The following protections are not affected by security mode and cannot be turned off by a switch, configuration option or environment variable (architecture §17.3): TLS (`--allow-http` remains, for development only, and is refused when security mode is on), request signing, UCAN verification, deny by default, the plugin sandbox, cloud-drive encryption (files on a cloud drive are always encrypted; encryption on a NAS stays optional), Activity logging. Security mode only adds protection on top of these and turns off no feature (§17.4).

---

## 8. Documentation Requirements

### 8.1 Code Documentation

- Every public function, struct, enum and trait must have rustdoc.
- Include examples in doc comments wherever possible.
- Document panic conditions and safety invariants.

### 8.2 Architecture Documentation

- `docs/architecture.md` is the canonical text; v1 and v2 are deleted and remain only in git history.
- Requirements for the user-facing apps are in `docs/prd.md`, and `docs/prototype/prototype.html` is authoritative for the interface; when one of the three changes, keep all three in sync.
- Any architectural deviation must be recorded and approved.
- Update `AGENT.md` when development practices change.

### 8.3 User Documentation

- API reference generated from rustdoc.
- Deployment guides for Docker, NAS and browser embedding.
- Web3 integration tutorials (DApp backends, social PDS, on-chain analytics).

---

## 9. Commit and Review Checklist

Before marking any task complete:

- [ ] Code compiles without warnings (`cargo build`)
- [ ] All tests pass (`cargo test`)
- [ ] No Clippy warnings (`cargo clippy`)
- [ ] Formatting passes (`cargo fmt`)
- [ ] All new files include the license header
- [ ] No new dependencies with incompatible licenses
- [ ] Documentation updated (rustdoc + user docs if the API changed)
- [ ] Benchmarks run (if performance-critical code was touched)
- [ ] CHANGELOG.md updated (for user-facing changes)

---

## 10. Decision Log

Records important architecture or process decisions:

| Date | Decision | Rationale |
|------|----------|-----------|
| 2025-05-30 | Initial AGENT.md created | Establish a development protocol for phased implementation |
| 2026-06-08 | Execution core migrated to DataFusion (v2) | Give up the self-built optimizer/executor |
| 2026-09-27 | Pivot to a decentralized local federated query semantic layer (v3): computation delegated to DuckDB/SQLite, DataFusion only does the federated merge | Self-built OLAP is beyond the team's scope; the differentiation is sovereignty, federation and P2P, not operators |
| 2026-10-30 | Repositioned as an AI-native, multimodal private data center (usually running on personal resources, supporting trusted data exchange): added cloud drives (3.4), connectors (3.5), compute sharing (3.6), v3 layers unchanged; data exchange and compute sharing use the same trust mechanism with different authorization | Data and compute stay in individuals' hands, and individuals call on each other instead of handing them to big platforms; see `docs/architecture.md` §1, §7.5, §11, §12 for the architecture |
| 2026-10-31 | 3.2a browser node: duckdb-wasm as the local engine (SQLite later); `HostEngine` stores only an id, JS objects live in a thread-local table, results use Arrow IPC; DataFusion drops compression/parquet, and datafusion-federation 0.5.5 is patched; browsers reach a Full Node over a WebRTC data channel (3.2b) | `QueryEngine` and DataFusion stay unchanged; only 0.5.7 fixes the forced default features, but it needs DataFusion 55; browsers cannot verify a certificate by DID, while WebRTC can pin a DID-signed fingerprint. See architecture §10 |
| 2026-11-01 | 3.2b: the Full Node provides the WebRTC endpoint with str0m (pure-Rust crypto), browsers connect directly with WebRTC-direct (no signaling needed), and the server does not verify the browser's DTLS certificate; requests are envelopes with the same fields as a Flight call, served by `serve_envelope` through the same authentication, authorization and semantic layer; the coordinator in the browser runs as a single task (one partition, `MergePartitionsExec` instead of `CoalescePartitionsExec`, which spawns tasks), chrono with `wasmbind` enabled | No C crypto library in the server and no signaling service; browsers are authenticated by signed requests, the same as native nodes; DataFusion 54 spawns Tokio tasks to merge partitions and reads the clock through `std`, both of which panic in the browser. See architecture §10 |
| 2026-11-01 | 3.3c: graph queries (DuckPGQ) only on Lance Spaces; graph definitions stored in the catalog (`graphs.json`), and a graph is created first when DuckDB does not have it; owner only, within one Space | DuckPGQ does not support building graphs over views, and Iceberg tables are views in DuckDB; the approach of copying them into tables for every snapshot was not adopted. See architecture §8 |
| 2026-11-01 | 3.3b: transactional Spaces support FTS5 and sqlite-vec 0.1.9; `CREATE VIRTUAL TABLE` is read by Oikome itself (only fts5 and vec0 are recognized, the contents of the parentheses are passed to SQLite as written); statements containing `MATCH` are parsed with the SQLite dialect instead; search only for the owner, within one Space; `PUBLISH` skips virtual tables | sqlparser cannot read module arguments, the generic dialect has no `MATCH`, and a custom dialect would change how other statements are parsed; see architecture §8 |
| 2026-11-01 | 3.3a: DuckDB upgraded to 1.5.6; Lance Spaces (`FORMAT = 'lance'`) attached to DuckDB as a whole, with reading, writing, index building and all three kinds of search handed to the `lance` extension; search only for the owner and subscribed copies, within one Space; reads use the latest version, and subscription lists one version's files with `lance-table` (indexes go along); no `MODES` declaration, capabilities checked by format and statement; extensions downloaded only by `oikome extensions install`; DuckDB uses a new connection for every statement (the extension caches datasets per connection) | The extension cannot read by version or list a version's files; search bypasses column grants and row policies; see architecture §7.6, §8, §9 |
| 2026-10-30 | 3.1d: the network phase's exit criteria are verified on GitHub Actions with three Docker nodes (one of them found only through mDNS) | No container environment on the local machine; CI runs on every push, and the Docker bridge network also verifies mDNS, which the local firewall blocks |
| 2026-10-30 | 3.1c: TLS between nodes by default, certificates self-signed with the node's DID key and verified by DID; plaintext http needs `--allow-http`; libp2p only for discovery, with results written to the catalog's Peers table, `PEER` optional | The DID is already the identity requests are signed with, so verifying certificates by it needs no CA or domain names; with discovery results in the catalog, both the `serve` and `sql` processes can use them. See architecture §3.4, §9 |
| 2026-10-30 | 3.1b snapshot subscription: only tables the other side may see in full are copied, the rest are marked `restricted` and can only be queried remotely; revocation only stops later syncs, files already copied cannot be taken back; VFS integration postponed to 3.4 | Files cannot be filtered by column grants and row policies; once data is copied it cannot be taken back, so this is stated rather than left implicit. See architecture §5.3, §9 |
| 2026-10-30 | Lance as a third Location format, for vectors, full text and large objects; analytical Spaces no longer use DuckDB vss/fts | Lance indexes live in the dataset and sync with its versions, so subscribers need not rebuild them; tabular data still uses Iceberg + Parquet. See §7.6 |
| 2026-11-01 | User-facing apps: serve only the owner's personal private data center, no operations or developer screens, members later, UI in English throughout (i18n later); new root directories `apps/` (a thin shell per client), `packages/` (shared TypeScript), `plugins/`, `design/`, `e2e/`, root `package.json` with npm workspaces; directories are created only when the matching work starts; new crate `oikome-api` (no transport, callers are owner or principal), HTTP provided by `oikome serve --web`, the desktop app calls it in process, and later `oikome-mcp` exposes only what a principal can do; desktop and phone later on Tauri 2; storage locations only local folders and NAS, cloud drives as storage locations undecided | Web, desktop and phone are clients on the same layer, and the screens live in shared packages reused by each client; one set of handling logic serves every client, and AI assistants cannot reach commands only the owner has; the product principle is that data does not leave home, which conflicts with putting data with a cloud drive provider, so architecture §7.5 and 3.4 stay unchanged until it is decided. See architecture §13.3, §13.4, §14.1 |
| 2026-11-01 | M1a: `web/` split into `packages/node` (`@oikome/node`, exporting host, store, transport, wasm) and `e2e/` (`@oikome/e2e`, test pages in `harness/`); the root npm workspaces at first contain only `packages/*` and `e2e`, with `apps/*` and `plugins/*` added once created; the test server maps directories by URL prefix (`/pkg/`, `/node_modules/`, `/`) and rejects out-of-bounds paths; the wasm build and remote tests follow `CARGO_TARGET_DIR` | Only moves things, no behavior change, browser-e2e's assertions unchanged; external dependency versions exactly as in the lockfile. See architecture §13.3 |
| 2026-11-02 | 3.4 split into 3.4a NAS and 3.4b cloud drives: a NAS at home first (WebDAV or a mounted folder), cloud drives supported step by step, each opened only once it has client-side encryption. A Space on a storage is not read in place: the writing node writes locally as usual and pushes a snapshot after each write (files first, then the snapshot file), other nodes pull the newest complete snapshot into a local copy and read it like a subscribed copy; one writing node per folder (recorded in `oikome-space.json`, to prevent mistakes, not access control); passwords read only from the environment variables `OIKOME_STORAGE_<NAME>_PASSWORD` / `_TOKEN`, the keychain comes with 3.5; a failed push does not make the write statement fail; `materialize()` dropped | Data on a NAS still stays at home; a cloud drive provider can read uploaded content, so encryption comes first. DuckDB reads Iceberg as a list of local Parquet files, the `lance` extension can only attach local directories, and Iceberg commits over WebDAV have no atomic compare-and-swap; subscription already copies verified snapshots, so one approach serves both Iceberg and Lance, and encryption later just wraps a layer around it. See architecture §7.4, §7.5 |
| 2026-11-02 | 3.6: compute means resources such as CPU and GPU; compute methods are only the ones Oikome already has (this is the standard for computation), opened one by one by the provider (`oikome compute offer`), the first being `sql`: one in-memory DuckDB per task, with external access off, no extensions installed or loaded, configuration locked once the inputs are loaded, memory and timeout set by the limits, output limited; no forwarding to external services; request-response over `DoExchange`, with `oikome.compute_methods` listing the methods the other side can use; UCAN `compute/<method>` with `nb` limits (time, input, output, memory), which delegation can only tighten; `space/*` and `compute/*` do not cover each other; per-peer quotas (by default 1 at a time, 30 per hour), authorization re-checked before a task starts, queued tasks refused after revocation; SQL stays closed-loop in the backend, apps call `compute_on_peer`, and `oikome compute run` is only for development and testing | Using existing compute methods means no new code has to be trusted; an isolated DuckDB cannot reach the provider's Spaces and files; quotas are the simplest accounting that keeps a peer from taking over the machine. See architecture §12 |
| 2026-11-02 | 3.5: connectors are only in-process Rust code (third-party connectors as M4 plugins), registered on the node (`sources.json`; the command line and the future Sources screen call the same set of functions), no new SQL statements; the cursor is stored in a hidden table of the Space and committed in the same transaction as the data; flomo exports have no id, so the creation time plus a sequence number within the same second is the primary key; flomo MCP discovers tools at run time and never deletes, to be tested once there is an account; HTML parsed with html5ever's tokenizer (scraper's selectors are MPL-2.0) | A connector, like a storage, is node configuration with credentials; "data and cursor change together" is one SQLite transaction; paths that need an account are written first without mock services, and tested together once the features are complete. See architecture §11 |
| 2026-11-02 | 3.4b: cloud drives go through sync folders first, the API is in 3.4c; every file on a drive is encrypted whole (XChaCha20-Poly1305 in segments, file names as a keyed hash), one random key per storage, and another node reads with the key exported by `export-key`; credentials are entered once by the user in the UI (for now, on the command line's standard input), and the node stores them in the system keychain, or in a 0600 private file when there is no keychain, with environment variables taking precedence; this replaces 3.4a's "read only from environment variables" | Sync folders cover the mainstream drives without an OAuth app; whole-file encryption covers data, indexes, snapshots and file names at once; ordinary users should not have to set environment variables themselves, and the UI calls the node's `add_storage` / `set_storage_secret` / `export_storage_key` directly. See architecture §7.5 |
| 2026-11-01 | User-facing tests connect to a local node (temporary data directory, test data written through the command line or the API), with no fake data and no mock client; M1 changed to the API first (M1b), then the screens (M1c), and each screen is built with the milestone that has its data | Only one test chain, no testing with fake data first and then all over again with real data; screens use the real API from the start, so they cannot drift from it. See architecture §14.1 |
| 2026-11-01 | M1b: new crate `oikome-api` (no transport, two kinds of callers, owner and principal; the API layer does not build SQL); `oikome serve --web` listens only on this machine, a one-time login link is written to `.oikome/web/login`, readable only by the owner, opened by `oikome web open`, never printed; whole-Space grants `GRANT SELECT ON SPACE` are stored separately, and a view counts only if after expansion it reads only this Space; the access log is stored in `.oikome/access_log.sqlite`, written by a background thread, with failed authentications merged per minute; registry writes take a file lock and re-read first; Space names are validated at the catalog layer; types generated into `packages/api` with ts-rs | The web app starts writing the registry inside the serve process, and a token revoked by `oikome sql` must not be overwritten; sharing is per collection and must include tables created later too, while views may span Spaces; a token left in the terminal history is unsafe. See architecture §6.1, §9, §13.4 |
| 2026-11-03 | The web app switches to two styles: Ink (default: black ink on white, hairlines without shadows, data in IBM Plex Mono, states as ink glyphs, deep crimson only where something needs handling) and Bold (neo-brutalism), switched in Settings › Appearance and saved per browser; fonts bundled with the app (OFL); the sidebar lists each storage location, and clicking one shows its capacity (`list_storages` returns free / total space for this computer and the folders); no underlines on buttons | The user wanted it more refined and stylish, set apart from ordinary dashboards, with multi-style dashboards supported later; seeing the capacity of only one place was not enough. See architecture §13.4 and the design skill's style-library.md |
| 2026-11-02 | Storage can be chosen and added in the web app: Welcome and Settings offer "this computer (the data directory or another folder) / cloud drives and external drives found / NAS / another folder", the node lists folders for the web page to browse (`list_folders`, `find_places`), `add_storage` calls the node's existing `add_storage` (password into the keychain, checked before saving, an `Oikome` folder created in the chosen folder, a folder inside a cloud drive encrypted as a drive), the chosen one becomes the default for new Collections (`set_default_storage`), and the examples go there too | The user wanted to choose the location in the very first step; a web page cannot get the system's folder picker, and the web UIs of NAS and media servers also have the server list directories; the data center's own directory (identity, catalog, access log) stays on this machine. See architecture §13.4 |
| 2026-11-03 | The default data directory becomes `Oikome` in the home directory (`~/Oikome` on macOS/Linux, `C:\Users\<name>\Oikome` on Windows), still changeable with `--root`/`OIKOME_ROOT` | Users see their own, visible folder the first time they use it; not in "Documents" because iCloud/OneDrive often sync it, which would corrupt databases in use; not in a hidden app-data directory because the files are the user's. See architecture §13.4 |
| 2026-11-03 | Examples and user data are plain files, like an Obsidian vault: one example of each kind (photos of Lisbon, an Oikome guide, three weeks of spending ending with that trip, a home's temperature and humidity readings), written on first use into `Examples/<name>/` of the chosen folder (the data directory if none was chosen) (photos plus `captions.csv`, Markdown notes, CSV tables); each folder is the source of the Collection of the same name, read once a minute, and edits made with any app are synced; new `photos-folder` and `notes-folder` connectors, source names may contain `-`, and the reason a read failed is recorded on the source; the web app gets a Sources page (list, add, sync now, stop, show in Finder) | The user wants data that is visible, editable and kept in a folder they choose; sources are already the §11 mechanism, so using them for the examples also makes the examples a demo of "connected data appears on Home". See architecture §11, §13.4 |
| 2026-11-02 | The web app has examples on the first visit (`add_examples`: a weekend in Lisbon, photos, notes, spending, added only once, not added again once deleted); a Collection is shown by its content (image column → photos, long text → notes, anything else → table), with Home and a separate page for each Collection; a storage can be chosen at creation (`create_space`'s `storage`, `list_storages`); the SQL workbench removed, `run_sql` kept only for the command line and tests | New users see right away what Oikome can store, and tests have the same data; ordinary users don't write SQL, and data will come in later through "Import". See architecture §13.4 |
| 2026-11-02 | M1c: the web app uses React 19 + Vite 7 (Vite 8 requires lightningcss, which is MPL-2.0 and does not meet the dependency policy), pages are served from the directory given by `--web-root`, strict CSP; UI terms Collection, Contact, Share, Activity, Your ID (the SQL workbench was later removed from the web app, see the next entry); the recovery key is a grouped Crockford Base32 code (14 groups, with a 2-byte checksum), restorable only into an empty node (`oikome identity restore`, run while the node is stopped); phone pairing moved to the phone app | A phone browser needs an HTTPS page, which the local loopback server cannot provide, and neither a public origin nor a self-signed LAN certificate was adopted; the grouped code needs no word-list dependency and catches typos. See architecture §13.4, §14.1 |
| 2026-11-03 | The working copy of a collection filled by a source (transactional SQLite) is always on the node's local disk, not even on a mounted SMB/NFS share; when the owner puts it on a NAS or cloud drive, a snapshot is published and pushed there automatically after every pull, a read-only second copy (not implemented yet; `add_source` currently creates it only locally); the web app defaults to Local, asks for confirmation before any other place, and explains that the working copy stays on this machine | SQLite relies on the local file system's locks and shared memory, which are unreliable on network file systems; what users have in mind is "my photos are on the NAS", and publishing and pushing already exist, so the copy there is a second copy like any other Space on a storage. See architecture §7.5, §11, §13.4 |
| 2026-11-04 | Home's Arrange changed to match the prototype, editing in place on Home: cards get a dashed outline, drag to reorder (arrow keys on the keyboard), drag the bottom-right corner to change width, a minus to remove, Add a Card at the end; each card stores its width (4–12 of 12 columns), and cards without a stored width (the first arrangement, old records with only an id) use the app's default width, with the last card in a row filling the rest of the row; node settings change one thing at a time, each with its own temporary file; the web app gains the prototype's motion: pages rise and fade in on change, Home numbers roll, bars grow one after another, lines draw from the left, Bold's clickable cards lift on hover, the icon spins while syncing, a progress bar on the first read, colors transition when switching styles; data refreshes do not replay them, and all are off when the system asks for reduced motion | The user, comparing with the prototype, pointed out the missing motion and that Arrange did not match the prototype; two saves at once wrote the same temporary file and the later one failed, which happened when adjusting cards in quick succession. See architecture §13.4 |
| 2026-11-04 | Accepting shares (M2c): invite codes generated at the API layer (`oik1-` plus base64url JSON: token, the sharer's Flight address, name; no address when listening on all addresses, relying on discovery), core unchanged, the command line still uses tokens; received invites are first stored on the node (`.oikome/identity/invites.json`, 0600) and shown on the Sharing page as the prototype's prompt cards, to accept or decline; accepting creates a remote Space with `create_space` (no SQL built), named with the sharer's name, numbered if taken, and the sharer automatically becomes a contact; sharing the same Space again replaces the previous share; a Collection shared with you shows whom it is from and until what day, and can be left (Leave); cover and counts are also read from the sharer | With the address and name in the invite code, the other side adds it by pasting once, without writing PEER and REMOTE by hand; the invite code contains a token, so it is stored as a credential; storing first and choosing later means nothing is lost on refresh, and invites delivered directly by the other node later can go into the same list. See architecture §13.4 |
| 2026-11-04 | Lending compute (M2d): the Compute card on the Sharing page follows the prototype with a "Lend this computer" switch plus a Change dialog: which contacts to lend to, how many tasks at once (1/2/4), how many per hour (10/30/100), mapping to the core's per-peer quota; pausing only withdraws the `sql` offer, grants are kept, and resuming needs no new invite code; someone newly lent to gets a one-year `compute/sql` grant and an invite code, and those removed from it or deleted as contacts have their grants revoked; the other side accepts with the same Add an Invite, the node stores the grant (`.oikome/identity/borrowed.json`, 0600), and the list asks the other side live whether it is lending (available / not lending to you right now / can't reach / ended); Activity writes compute requests as "ran a task", "asked whether it is lent to them", "was refused" | The prototype's "number of cores, hours per day" are not controlled by the core, so the UI offers only limits the core actually enforces; without a borrower side the invite code could only be used on the command line, so the borrower side first gets as far as accepting and seeing availability, and actually handing tasks to the other side comes with M4's borrowed compute; the other side pausing and revoking both return a refusal, not told apart by design. See architecture §12, §13.4 |
| 2026-11-04 | Dashboard plugins v1 (M2e): a plugin is a folder of web files with `oikome-plugin.json` (what it reads: a shape plus the column types it needs; card size; entry page), installed in `.oikome/plugins/<id>/`, and Oikome's own two (spending by category, photos from a year ago) are built into the node; each plugin gets its own DID at install and has whole-Space grants only on the Collections the owner chose, the app reads as the plugin through `plugin_read`, and the node's grants decide; the pages are served separately by the server: CSP with all connections, child frames, workers and forms set to none, `sandbox allow-scripts`, devices disabled, DNS prefetch off; Chromium does not honor CSP `webrtc 'block'`, and testing showed STUN packets could still be sent from inside the sandbox, so the server injects the SDK as the first script, which deletes the RTC constructors first, and nested frames, workers and eval are all blocked; plugin reads are recorded in Activity (once per 10 minutes for the same plugin and Collection, every refusal recorded); new Plugins page and From your plugins in Add a Card; also unified spacing per the prototype (card headers, list rows, page margins, two-column proportions, dialogs) | M2's exit criterion is that a plugin can read only data within its grant and cannot reach the network; authorization is left to the node rather than the page, so even a broken page cannot read anything else; network isolation was tested item by item (fetch, images, beacon, WebSocket, navigation, pop-ups, WebRTC), and WebRTC is the one that CSP cannot govern. See architecture §13.4 |
| 2026-11-04 | Activity records the owner's own first steps: setting up the data center, adding the examples, saving the recovery key, written as "You · This computer"; the "set up the data center" line is not removed by the 90-day cleanup; data centers set up earlier get these recorded once, using the time stored in settings | Activity is where the owner sees what happened to their data, and it should start from the first run. See architecture §9 |
| 2026-11-04 | Activity writes "who" the way the owner knows them: you, a person, a plugin or Unknown; a person's name is, in order, the contact name, the name they gave in an invite or when lending compute, then the name recorded at removal; when a contact or plugin is removed, its name is recorded in the `names` table beside the access log and kept as long as the records; names callers give themselves in requests are not used | Being in contacts or not is not identity; the records of removed plugins and contacts must still say who it was; anyone can impersonate a self-given name. See architecture §9, §13.4 |
| 2026-11-04 | Sources can be paused (`paused` in `sources.json`: not read when due, can still be read manually), and sync frequency can be 1, 5 or 30 minutes, hourly, daily or manual; removal asks first and keeps the collection, and adding the same file or folder again (by its real path, recorded in `node.json`) goes back to the original collection; each source has a detail page, sync records are kept for 30 days in the collection's hidden table `_oikome_source_runs`, with consecutive identical results merged into one row; the More menu becomes a top-level popover, no longer clipped by the table | The old "Stop" unregistered the source: it vanished from the list, and adding it again could only create a new collection; what users meant by stop was actually pause; a source read every minute would flood the records with "nothing new". See architecture §11, §13.4 |
| 2026-11-04 | Devices are split into two kinds by which way data flows (not implemented yet, comes with the phone and desktop apps). **Devices that read data** must pair: phones and tablets scan a QR code on this computer; another computer is found by mDNS on the same network, and both sides check 6 digits computed from both DIDs, or enter or paste a pairing code (when discovery fails, e.g. the network does not allow multicast). After pairing, the node signs a UCAN for the device's DID, read-only by default, revoked on removal. **Devices that produce data** (lights, sensors, appliances, robot vacuums, watches, scales) come in as sources, through a home hub (Home Assistant), the maker's cloud (public APIs only), the phone's health app, or a direct connection. Paired devices must be on the same network for now (no STUN, TURN or relay); the UI entry is called Add a Device, and only the confirm button after checking the digits is called Pair. Undecided: what another computer counts as; which management operations a device can perform; whether to integrate Matter read-only; whether to have a unified Devices page | Pairing and removal reuse the node's existing grants and revocation, so nothing new has to be trusted; appliances and sensors never read the data center, and Oikome only reads devices, never controls them, and is not a smart-home hub; Add is consistent with the other Adds and also covers devices that need no pairing. See architecture §13.5 |
| 2026-11-04 | All docs moved into `docs/`: the architecture document `oikome_architecture_v3.md` renamed to `docs/architecture.md`, v1 and v2 deleted (kept only in git history); the user-facing PRD moved from Claude Docs to `docs/prd.md` and synced once against the architecture document and the prototype; the design directory outside the repository, `~/Documents/oikome-design/`, moved whole into `docs/prototype/`, the original directory deleted after checking they matched, and from now on only the copy in the repository is edited (`prototype.html`, `i18n/`, photos in `lisbon/pick/`, `references/`); `.gitignore` no longer ignores `docs/` | The docs were too scattered: the architecture at the root, the PRD outside the repository, the prototype without version control, so changes did not line up; v3 is the current architecture, and keeping the version number and old versions only leads to misreading |
| 2026-11-04 | Viewing in place + Save To + sharing: files in local folders and on each storage are browsed and queried in place through read-only in-place Spaces, with the index as a cache on the node; only when the owner chooses to save (Save To) is anything copied, to plain files or a new Space, once or continuing; a saved Space is shared on its own and can also be shown as a portfolio; sharing with anyone goes through public shares (3.8), photos without location data by default; added 3.9–3.11; photo editing is done in other software | The owner wants to see the data in any storage and app without copying it first; copying is a deliberate choice with a target and rules; a saved Space is decoupled from its source. See architecture §7.7–§7.9 |
| 2026-11-04 | Storages are both sources and targets: viewing a cloud drive in place needs read access to the whole drive, requested separately, used first during testing, and put through Google's review before the official release; storing Spaces and Save To use only the least permission; 3.4c does only Google Drive and Dropbox, first upgrading opendal to the latest release, with large files using both drives' chunked uploads (added in the forked opendal and contributed upstream, not stored as several parts on the drive), and OneDrive stays on its sync folder; 3.4d writes Baidu Netdisk as an opendal service, kept in Oikome's fork, with a PR upstream after the combined real-account testing (apache/opendal#8481), no separate client written inside Oikome; 3.4e object storage through opendal `s3`, encrypted by default | With access only to the files it created itself, viewing in place is impossible, and requesting separately keeps whole-drive access optional; opendal's `onedrive` needs whole-drive read-write access, and 3.4d also needs the latest release; one service is usable by every opendal user, and if upstream does not take it, it stays in the fork or moves into a crate of its own; one S3 service covers AWS, R2, B2, MinIO and the S3 endpoints of OSS and COS. See architecture §7.5 |
| 2026-11-06 | 3.12 without any relay: the router opens the node's one peer-to-peer port only while a share or lending the owner allowed is going on, asked first (Deny / Allow; without "Don't ask again" the answer ends with its share or lending; with it, it is the rule, changed in Settings › Network); otherwise a contact both sides reach introduces them (circuit relay v2, 16 KiB and 30 s a circuit, contacts only, only while reached directly, can be turned off) and they hole-punch (DCUtR). Only contacts get in from outside. Flight over a libp2p stream (`/oikome/flight/1`) with the same TLS; TCP only; ports mapped with rupnp + crab_nat (`portmapper`/`igd-next` pull in an MPL-2.0 client) | Open source runs no servers and owners shouldn't have to; being reachable is the owner's call each time. See architecture §9, §16 |
| 2026-11-05 | The "later" work is scheduled: 3.12 relays, 3.13 WebDAV (first 3.14; the chain source first numbered 3.13 was dropped: a chain is a way to pay, M5, and a caller's identity, 3.8, not a source); M6 desktop app and installers, M7 phone app and devices, M8 languages, M9 family, M10 plugins from others; order 3.9, 3.8, 3.7, 3.10, 3.4c–3.4e, M6, 3.12, M7, M8, M9, 3.13, then M5 and M10 | The owner wants the whole road in one place. See architecture §14 |
| 2026-11-05 | 3.9 viewing in place (`oikome-node::inplace`): `CREATE SPACE … WITH PLACE, PATH` binds a read-only `InPlace` Location; the index under `.oikome/index/<location>/` is SQLite (rows, 512 px JPEG thumbnails via the `image` crate, FTS5 trigram) exported as a Parquet `files` table, rebuilt when deleted; top-level CSV/Parquet/JSON files are DuckDB views read each time (a NAS's through a 2 GiB cache), offline ones fail with a message; scanned every minute (20 s of reading) and on `REFRESH SPACE`; API `view_in_place`, `refresh_space`, `list_files`, `read_file`, `show_folder` and `/files/thumb|original/…`; `add_source` refuses photo and note folders; existing folder sources become in-place Spaces once at start, keeping names and whole-Space grants, deleting the SQLite copies; shares of in-place Spaces are view-only; the runtime's threads get 8 MiB stacks | Nothing is copied into Oikome just to be seen (owner, 2026-11-05); SQLite instead of the planned Lance so search finds Chinese as for sources; the stack fix after a contact's federated query overflowed 2 MiB. See architecture §7.7, §13.4 |
| 2026-11-05 | 3.11 folded: portfolios into 3.10, photos' location removed in public shares into 3.8, saved Spaces shared with contacts through the existing shares; order: core phases 3.4c–3.10 first, M5 Pay and third-party plugins last | Only portfolios in 3.11 were new. See architecture §7.9, §14 |
| 2026-11-05 | M4 scope: only borrowed compute; third-party plugins, the plugin directory and plugin signing move out of M4 and come one plugin at a time; 3.4c stays a core phase | How third-party code runs differs by plugin kind (cards in the browser, sources and payments on the node), so it is decided with the first real plugin of each kind. See architecture §14.1 |
| 2026-11-05 | M4 borrowed compute: Home's daily totals are worked out by the node (`daily_totals`, the task SQL fixed, the two columns checked against the table and quoted); sent to a computer lent here only when the owner turned it on (`use_borrowed`), the table has 100,000 rows or more, the collection has an active share with the lender, and read as the lender (a session with their DID as principal) it is all there; the input sent is that read (`compute_batches_on_peer`); else the same SQL runs here (`run_task_here`); answers from there kept 10 minutes per rows, computer and share; each task sent is an owner Activity line (`owner_computed_on`) | Nothing goes that the lender can't read already, and no partial input gives a wrong answer; the same isolated DuckDB on both sides gives the same totals; the card and the computer's dialog say where and why. See architecture §12.2 |
| 2026-11-05 | M3e MCP: served by the node over HTTP at `/mcp` on the web API's loopback port (`oikome-server::mcp`), per assistant token, no `Origin`, both protocol eras (2026-07-28 per request; 2025-11-25 and earlier after `initialize`, no sessions), tools `list_collections`, `search`, `read_row` only; no `oikome-mcp` crate or stdio bridge; the connect dialog gives Claude Code a `claude mcp add --transport http …` command | One process holds the data; clients of either SDK generation connect; Claude Desktop waits until it is checked whether its connectors can reach this computer. See architecture §13.4 |
| 2026-11-05 | M3d AI assistants: each is a Principal with whole-Space grants (like a plugin) and a token shown once, kept as a BLAKE3 hash; it may list, search and read a row, as itself; its search finds rows through the index and reads them again as the assistant by key with bound parameters (`OikomeSession::query_params`), so column grants and row policies hold; searches (with their words) and reads go to Activity | One permission model for everyone; the index reads past grants, so it only names rows. See architecture §13.4 |
| 2026-11-05 | M3a full-text search: no vectors in M3 (meaning is left to the AI assistants that connect); the node keeps a hidden trigram FTS5 index (`_oikome_fts_<table>`) for each table a source writes, kept in step by triggers in the same transaction and made by the pull when missing; `search` (owner only) goes one Space at a time, through the index where there is one and reading up to 10,000 rows per table elsewhere, skips collections shared with you, and returns excerpts with UTF-16 match ranges; `read_row` opens a hit by its key | Full text needs no model or download and spends no CPU on embeddings; trigram finds words in any language, Chinese included; triggers keep the index right whatever writes the table; terms are bound as FTS5 strings or `LIKE` patterns, never SQL. See architecture §8, §13.4 |
| 2026-11-09 | M8 languages: the prototype's nine; a TypeScript catalog per language in `packages/screens/src/i18n/`, typed by the English one and checked by `npm run check-i18n --workspace @oikome/screens`; the language is the device's choice (browser storage), the browser's by default, loaded before the first paint and applied at once; dates and numbers by `Intl`; the node's failures for people carry `reason` and `params` (`ApiError`) and the apps word them; `add_examples` takes `language`, the examples' words are in `crates/oikome-api/assets/examples/<language>.json` and collection names stay; plugin manifests may have `translations` and the SDK passes `language`; source failures and SQL errors stay as they come | The types catch a missing key; the node keeps one language and each app words it; collection names are typed in SQL and name folders. See architecture §13.4 |
| 2026-11-09 | 3.4e supports one object storage, RustFS (MinIO, hosted services and the region dropped): `StorageKind::S3` (address, bucket, access key id; the secret key as the storage's password), files under `Oikome/` in the bucket, encrypted by default, written in place (no rename) in 8 MiB parts, keys only as given (no environment, `~/.aws` or instance metadata); plaintext to this computer needs no `--allow-http`; opendal 0.59.5 without default features (TLS with ring); RustFS installed from its official Homebrew tap for tests, here and in CI; Google Drive's gaps in opendal (such as whole-file one-shot uploads) are fixed upstream, with a patched `opendal-service-gdrive` in `third_party/` meanwhile | One server to support keeps the form (no region) and the tests small, and RustFS is open source and maintained (MinIO's open-source project is archived); a bucket shows a PUT whole or not at all; no other account's keys sign by accident. See architecture §7.5 |
| 2026-11-10 | 3.4c is Google Drive (Dropbox later): `StorageKind::Gdrive`, always encrypted, files in an `Oikome` folder at the top of the drive, written in place; sign-in with PKCE through `start_google_drive` and `GET /oauth/google/callback` on the web app's loopback port, let in by a one-time state (no cookie comes with Google's redirect); the OAuth client is the owner's own, entered with the drive (the ID in the registry, the secret in the keychain); the refresh token forgotten on removal and never revoked; no viewing in place (`drive.file` only) | A loopback callback needs no fixed port or pasted address; nothing of the project's stands between an owner and their drive, an app registration neither (owners who don't want to register one use the sync folder); Google revokes all of a project's tokens at once; a rename on Drive leaves the replaced file in the trash. See architecture §7.5 |
| 2026-11-10 | Collections and folders viewed in place browse as in a file manager: covers or a list kept per browser, by kind and place, sorted by name, last change, size or kind (`list_files` `sort`/`descending`, fixed SQL by name; `list_spaces` `sizeBytes`/`changedAtMs`; `FileItem.modifiedMs`, `FileFolder.size`/`modifiedMs`); a path bar from the place, folders first; photo tiles show the photo, note tiles their first lines; a folder viewed in place is called by its own name. The place picker: kind first (folder or Local, NAS, cloud drive, object storage), then which drive and how it connects. Folders look like folders (Folder on the cover, the place first, a Folders filter); subfolders are covers (`FileFolder.photos`/`notes`/`cover`), files preview themselves (the index keeps a CSV's first lines); one navigation, the top crumb, `?folder=` per folder; first run: a dashed "Add another place" row beside the places; a folder place is picked in the folder browser | The owner's Finder request and approved prototype (2026-11-09/10); a folder's files are paged, so the node sorts them. See architecture §13.4, prd.md |
| 2026-11-10 | Every file of a folder viewed in place opens as itself under its file name, with a quiet when–where–who line and the rest under Info: photos large, notes in full, CSV/TSV/Parquet/JSON-lines as paged tables read where they are through the new owner-only `read_file_rows` (any depth, captions included, no SQL, at most 64 MiB), unsupported files by name and size with Show in Finder through `show_folder`'s optional path. ← → move through all files at the unchanged `?table=files&item=<path>` address; the folder page's table picker is removed, not the relations search, sharing and AI use | The owner approved Quick Look's content-first model; opening a CSV directly is clearer than a second table browser, and text CSV/TSV cells stay as written so a bad cell cannot keep the file from opening. See architecture §13.4, prd.md |
| 2026-11-10 | 3.4c adds Dropbox beside Google Drive: `StorageKind::Dropbox`, OAuth + PKCE with offline access through `start_dropbox` and the authorization code copied from Dropbox's page into the authenticated `finish_dropbox` API (no callback), the owner's Scoped app remembered in the secret store, files always encrypted; App Folder by default, Full Dropbox for viewing other folders in place. opendal's `dropbox` crate handles reads, vendored in `third_party/` with a small patch so PKCE refresh tokens work without an App secret; `oikome-vfs` streams upload sessions (start/append/finish) | The owner asked for Dropbox as one commit and authorized browser verification; the copied code avoids registering an exact loopback port while PKCE keeps the code insufficient on its own. Using opendal directly avoids the detour of forking it, while a session uploader avoids holding a whole file in memory. See architecture §7.5 |

---

*This is a living document. Update it as practices evolve.*
*All agents **must** read this file before every development session.*
