# Product Delivery Plan

> **Status**: consolidated 2026-11-16. This is the single implementation queue for the current app work. It preserves the agreed content-viewing stages and adds the data-source, analysis, dataset interoperability, plugin and network work identified through the PR 20 review. Code, the PRD and the prototype must be updated together as each stage lands.

## Rules

- Adding content targets an existing collection by default. Creating a collection is always a separate, explicit choice.
- A source and its destination are different things. A connector must not silently become a collection merely because it was configured.
- Rendering follows a declared data contract. Source names, collection names and hard-coded Smart Home exceptions are not presentation rules.
- Installing a plugin registers capabilities; it does not implicitly start a source, grant data or put a card on Home. Each capability is configured explicitly.
- Home shows real node state only. Prototype data belongs to `docs/prototype/`; generated examples are identified as examples and are not represented as user activity.
- Existing media and safe-editing commitments keep their places. New work must not silently displace them.

## Current behavior, verified in code

- PR 21 separates `SourceDef.name` from `SourceDef.space`. `add_source` requires an existing transactional collection and table and does not create one implicitly; legacy records without `space` retain their original destination. `add_in_place_folder` merges whole folders into file collections and converts only empty analytical collections with no source.
- PR 22 adds owner-only local UTF-8 text saves with real content hashes, source/copy choices, exclusive creation, confirmed atomic replacement, preserved text encoding and revocable confirmation records. Known non-text formats are excluded before actual UTF-8 bytes determine editability, including code, configuration, extensionless text and CSV/TSV/JSON text tables. It does not edit connector tables or remote/read-only bindings.
- PR 23 adds read-only PDF first-page grid thumbnails and a full view with a lazy PDF.js display chunk and Worker, current-page rendering, pagination, zoom, text selection and on-demand resources. No engine request occurs before a PDF tile approaches the viewport. MCP Access also has guided standard configurations for Kimi Code and Codex beside Claude Code, with the same scoped grants and one-time tokens.
- Built-in connectors are CSV, flomo export/MCP and the legacy photo/note folder connectors. There is no Home Assistant connector or Smart Home source plugin in the node. Smart Home appears in the PRD and prototype only.
- PR 24 extends one package manifest with independent `cards`, `sources`, `preview` and `mcpClients`. Installation has no data, source or Home side effects. Source declarations reuse existing connectors and preview declarations do not execute third-party code; those are not open host runtimes.
- Home has Arrange and Add a Card. Adding a plugin card separately configures its package's collection grants. The arrangement persists in `.oikome/node.json`; unresolved saved cards show an unavailable state without erasing their layout and recover when their package or data returns. Legacy unarranged installs keep their old cards; new installs never appear implicitly.
- MCP Access is manifest-driven, including external installed adapters. Official Kimi Code, Codex and Claude Code packages are available directly; the generic writer safely merges only declared JSON/TOML settings and never selects a model/profile.
- Activity is persisted in `.oikome/access_log.sqlite`. Home previously filtered out owner and plugin events, which could make it empty while Activity had rows. That filter is removed in PR 19.
- Series presentation currently comes from generic time-and-number schema inference. It is not a Smart Home special case, but connectors still need an explicit semantic contract so inference is only a fallback.
- PR 25 adds a persistent Network entry beside the desktop node identity and in the phone top bar. Its popover reads the existing owner API on open and every 30 seconds, shows the real reachability state, active reasons for opening the port and current peer-to-peer listen address, and deep-links to Settings → Network. It keeps no duplicate configuration state and shows no online indicator for local-only or unavailable networking.

## PR 19: content, file targets and navigation consistency

- Keep the content descriptor, safe text/table viewers and structured-content behavior already delivered.
- Add Files defaults to an existing collection. New Collection is an explicit destination, not the default. Keep in Place and Copy to Oikome remain separate choices.
- Copy selects files before asking for a new collection name. A suggested name comes from the first file and remains editable; original filenames are preserved. Duplicate names in one copy are rejected before submission, and existing files are never replaced or silently renamed.
- Rename the Chinese Sources navigation to Data Sources and AI Assistants to MCP Access. The MCP page emphasizes clients and their collection grants.
- Home and Activity read the same unfiltered activity log.
- Validation: real API tests plus browser coverage for existing and explicit-new destinations, duplicate-name handling, per-file provenance, Home activity, all languages, desktop and phone.

## PR 20: media and retrieval results

Keep the agreed scope in `docs/content-viewing.md`: lazy-loaded PhotoSwipe navigation and gestures with Info preserved, native audio/video with range reads and codec failures, DOMPurify SVG blob previews with external references blocked, source-linked search/vector results, and bundle-size verification. PhotoSwipe and DOMPurify are separate lazy chunks; the first-paint increase is about 2.6 kB JS and 0.88 kB CSS. No Office, PSD, RAW or transcoding engine enters the core.

## Complete acceptance environment (PR 20)

- Keep ordinary first-run examples unchanged. One `tests/scenarios/acceptance` workspace has seven purposeful collections: NAS photos shared to the peer, local media, one Markdown note, structured/edge files, CSV readings, drive-sync reading and encrypted RustFS archive. Do not split capabilities into separate demos or duplicate examples to inflate the count.
- `npm run acceptance` opens the fully seeded workspace and its peer. `npm run acceptance -- --first-run` starts empty and completes setup through the real UI. Every launch uses fresh isolated state, never `~/Oikome`, a mock `NodeClient`, committed node snapshots or committed generated media.
- Mounted/sync folders, OAuth stand-ins and real cloud accounts remain distinctly named levels of verification; no fake connected cloud state is shown.
- The launcher prints and opens the main and peer URLs. Generated roots are marked and ignored; only those roots may be reset, and running the same command again opens the existing workspace rather than resetting it.

## PR 21: collection and data-source targets

- A collection is a content container with folders, files and structured tables, not a file or a side effect of configuring a source. In-place/imported folders and database-backed tables keep their different physical implementations; the UI does not pretend every collection is a plain directory.
- Add a clear Add Data Source flow beside Add Files. It defaults to the current/existing collection and its current folder or a chosen table. Creating a collection is always an explicit separate choice.
- Separate source identity from destination. Introduce destination collection/table fields in the API and source registry rather than deriving the Space from `SourceDef.name`. Migrate existing sources with their current name as the destination so behavior and grants survive.
- CSV configuration names a target collection and table, validates schema compatibility and reports conflicts before sync. It must not create a same-named collection implicitly. Upsert keys and cursor commits retain their current transactional guarantees. Multiple sources targeting one destination are refused clearly until supported safely.
- Let a connector declare table semantics and a recommended view (photos, notes, table or series), with schema inference only as a fallback. No rendering branch may depend on a source or collection name.
- Keep ordinary content use separate from later advanced Iceberg/Lance/S3 dataset connection and migration. Internal analysis and storage layouts are not ordinary-user choices; see `docs/data-analysis.md`.
- Validation: existing-source migration, CSV into an existing collection, destination/schema conflicts, grant preservation, scheduled and manual sync, no implicit collection creation, and a purposeful acceptance example shared to the peer.

## PR 22: safe editing

Local UTF-8 text uses the existing CodeMirror viewer with no new editor dependency. Known non-text formats are excluded before actual UTF-8 bytes determine support, without an editable-extension list; this includes Markdown, code, configuration, extensionless text and CSV/TSV/JSON text tables. Opening recomputes the real BLAKE3 content hash and capabilities; saving repeats path, UTF-8, size, permission and version checks. The owner explicitly chooses the original or a copy beside it. New copies use exclusive creation; replacing a source or existing copy needs confirmation, and replacement uses a same-directory temporary file plus rename while preserving BOM, line endings and permissions. Editing is limited to 512 KiB so requests remain within the ordinary HTTP limit. Remote/read-only bindings and connector working tables are not represented as editable sources.

Remembered source-save and overwrite confirmations live in `.oikome/node.json`, separately for identity and normalized-directory scopes. Identity, action, scope, canonical file, directory, content version and permissions form the safety fingerprint; a mismatch asks again. Settings lists and revokes these records. Browser coverage includes repeated saves, outside-edit conflicts, exclusive copy creation, overwrite confirmation and revocation.

## PR 23: content previews and MCP clients

- Add read-only PDF grid thumbnails and a full view using PDF.js's display layer and a dedicated Worker, loaded only when a PDF tile approaches the viewport. Do not embed the full PDF.js Viewer or preload all pages.
- Render a tile's first page without a text layer. In the open viewer render only the current page, fit narrow containers, support page navigation, zoom and text selection, and cap canvas pixel density. Cancel rendering on page changes and destroy the loading task and Worker on close or navigation.
- Serve indexed PDFs through the existing original-file endpoint with range reads and a restrictive CSP. Character maps, standard fonts, WASM and image decoders are on-demand resources, never first-paint requests.
- Share file artwork between grids and folder covers: sanitized SVG thumbnails, paused native video frames near the viewport, and a distinct audio icon. A folder without photos or notes shows real files and its file count rather than “no data”.
- PSD, RAW, Office and extra media codecs remain optional Content/Preview plugins. The Dashboard sandbox remains separate and does not gain Worker or file privileges.
- Add Kimi Code and Codex as explicit MCP Access choices. Default to owner-approved one-click CLI configuration on the same computer, with safe JSON/TOML merging, loopback validation, atomic writes and an explicit update confirmation. Keep standard manual configurations and Other for generic clients; never prescribe a model or provider profile. Apps and OAuth linking are later stages.
- Validation: real multi-page and damaged PDFs, nonblank thumbnail pixels and damage fallback, decoded SVGs, paused video frames, audio icons and mixed-media covers, text selection, page/zoom controls, desktop and phone layout, no engine request before PDF tiles, worker cleanup, production bundle and resource-size measurements. Exercise Kimi Code and Codex configuration discovery, MCP initialize/tools/list/tools/call, a real client-driven read, grant revocation and all languages.

## PR 24: plugin capabilities and Home composition

- Extend one manifest with independent optional `sources`, `cards`, `preview` and `mcpClients`. Installation alone changes no data and adds no Home card. Current collection grants are shared by cards within one package.
- Keep Dashboard cards in the browser sandbox. Source declarations configure only existing node connectors; preview declarations register metadata only and do not execute third-party code. Neither kind inherits Dashboard privileges, and a real third-party host remains separate future work.
- Make Add a Card and collection/grant configuration discoverable. Preserve existing layouts and legacy default cards during migration. Missing packages or collections produce an explicit unavailable state rather than deleting the saved entry.
- Use flomo as the composite example: configure HTML import into a chosen collection, then separately add its optional Recent Notes card with a read grant. Move Kimi Code, Codex and Claude Code adapters to the same manifest mechanism; official MCP packages are installed by default and can be removed persistently, while external ones require installation. Only installed adapters appear in MCP Access; there is no generic HTTP-client choice.
- Operate the independent `oikome/oikome-plugins` official repository. Its CI publishes deterministic versioned ZIPs and an index with complete manifests, sizes and SHA-256 digests. Plugins lists that release index, and the node downloads only an owner-selected version, verifies it and uses the ordinary installation lifecycle. Publisher signing and third-party community distribution remain later work.
- Smart Home remains a later composite example after a real Home Assistant source exists. There is no built-in Smart Home rendering exception and no device control.
- Validation: capability isolation, no install-time side effects, card add/remove/unavailable/recovery behavior, grant boundaries, external MCP manifests, sandbox tests, the flomo composite example, and a real local HTTP registry install with checksum rejection. Spending by Category has a known-total/category-order/open-collection case; Photo Memories selects and decodes a known photo from dated alternatives and opens its collection. In-place previews contribute image/date/text column kinds for card matching, and cards refresh when grants change.

## PR 25: persistent network access

- Add a network-status icon near the top-left node identity, using Anytype only as an interaction reference. It is an entry to the existing network model, not a second network stack.
- The icon and its popover show real state from `network()`: local-only, preparing/open for sharing, reachable, router-refused or error; what the port is open for; and the current peer-to-peer address when available.
- The popover offers Open Network Settings and the currently relevant allowed action. Full UPnP/NAT-PMP, Don't ask again, contact introductions and permission choices remain in Settings → Network.
- Keyboard, mobile and accessibility behavior matches the rest of the navigation. No green online indicator is shown without a real connectivity check.
- Validation: every network state, no false online status, settings deep link, port-permission choices, mobile layout and no duplicate configuration state.

## PR 26: Save To and public sharing

- Treat saving and public sharing as one complete content-delivery item (3.10 and 3.8). A saved collection is detached from later source-file changes and carries its portfolio: order, cover, title and captions. Saved files open through the ordinary viewer even after the source changes.
- This iteration deliberately implements one-time saves only. The owner chooses the destination place, folder and naming rule. Plain files receive unique names and never replace existing files; a new collection keeps original bytes and portfolio metadata. Continuing save rules remain future work.
- Public sharing starts from a saved collection. Query-only is the default and excludes original bytes and snapshot downloads; copying requires explicit permission. Per-visitor request, row, result-size and runtime limits apply, and JPEG/PNG location metadata is removed by default.
- Signed read-only snapshots live under the node's private `.oikome/public/` directory and are served only when copying is allowed. `did:key` and Ethereum `did:pkh` (EIP-4361) authentication, short-lived sessions, revocation and Activity work over the public HTTP API and the existing WebRTC envelope dispatcher. The existing network gate admits strangers only while an allowed public share is active; private methods still require UCAN. Publishing to external storage, IPFS or Arweave and relays remains future work.
- Validation: source changes after saving, file-write ownership, saved-file viewing, portfolio fidelity, public grant revocation, query limits, both signature methods, photo privacy, snapshot integrity, private-method isolation and the real-node browser Save To → publish → visitor → revoke scenario.

## PR 27: internal analysis profiles and core computation

- Start with precise source targets: a collection contains files, tables, views and derived artifacts, while each source names its target file or table and the plugin capability that produced it. Storage location and provenance stay separate; no plugin implicitly creates a collection.
- Replace the misleading `drive-reading` example with `reading-list/books` supplied through the Drive/CSV path, and keep legacy one-source-per-Space registrations compatible during migration.
- Add the internal dataset profile in `docs/data-analysis.md`: direct-query versus derived-analysis actions, field-meaning inference review, source-structure changes and explicit derived-artifact ownership.
- Keep computation in the existing engines and federation layers. DuckDB owns analytical work, SQLite owns transactional work, and the browser only presents returned results.
- Do not expose internal storage layouts as ordinary-user choices. Do not turn file viewing into an implicit analysis or write operation.
- Validation: multiple source targets within one collection, known-schema inference, incorrect-inference correction, source changes, direct versus derived results, engine boundaries, grants and real-node acceptance.

## PR 28: data sources as plugins, analysis tables of files and the interface review

- Every data source is a plugin's: File Import, RustFS and flomo are preinstalled; Sources lists what installed plugins offer. A folder chosen to add becomes a collection of its own by default, named in the dialog with a notice when the name or the folder is taken; folders on a storage are picked by browsing it.
- Data Analysis over a folder collection's CSV, TSV, Parquet and JSON Lines files: a file, or a subfolder of files of one format, becomes an Iceberg analysis table of the collection (queried as `<collection>.<table>`, rows carrying their file), made again when its files change and dropped with them; an Analysis view with totals by month and by category and a role for each column (`docs/data-analysis.md`).
- The rest of the 2026-11-17 interface review: one Add Contact (by ID or invite, with your own ID to copy), a phone tab bar (Home, Collections, Sharing, More), public shares that ask only access, an end date and photo location and show visitors photo previews, and the shares table fitting desktop widths.
- Not yet: Excel sheets, other aggregations than sum and count and freer exploration of analysis tables, a RustFS folder joining an existing collection, and moving sources registered as CSV to files.

## Order after PR 28 (agreed 2026-11-18)

D (finish file sources) → E (backup and restore, then read-only datasets) → C (the plugin sandbox and RAW previews). A (freer analysis through a third-party library in a plugin) and B (tables as knowledge, K3) follow. Media recognition (knowledge K4) was taken out of the plan on 2026-11-19; see Not planned below. The research behind each stage, with sources, is in [`research/next-phases.md`](research/next-phases.md). Each stage's open decisions are listed with it and are settled at the start of that PR.

## PR 29: file sources across places, CSV sources as files, Excel (D)

- **Folders from several places in one collection.** A RustFS folder (and a NAS or drive folder) joins an existing collection by reference, as a local folder does: `InPlaceSource` gains the storage it is on. Local roots keep their paths; each remote root appears as a top-level folder named after itself, numbered on a clash. Reads, the cache, previews, analysis and knowledge resolve the place per root; a remote root that cannot be reached shows its files unavailable while the rest keep working. The RustFS data source, and File Import for a whole folder, offer an existing collection as well as a new one. An added folder is read where it is and is not edited or copied into from Oikome; knowledge waits for one that is away instead of marking its files failed. Removing an added folder from a collection is not offered yet (nor for local folders).
- **CSV sources to files.** A registered CSV source's page offers Convert to File: the file joins a file collection the owner chooses, by reference, its analysis table is made, and the source stops. The old table collection stays until the owner removes it; the dialog says its shares and grants do not move. Done as: into an existing file collection the file appears at its top under its own name (numbered on a clash, the other paths unchanged), or a new collection holds just the file; the collection's analysis tables are made, the file's among them.
- **Excel.** xlsx, xls, xlsb and ods files become analysis tables, one per sheet (`<file>_<sheet>`), read with calamine (MIT); the NOTICE and `cargo deny` record it. Done as: xlsm too; visible worksheets with anything in them (hidden and empty sheets make no table); the first row with anything in it names the columns; formulas give the values last saved; a column of dates is dates, of numbers numbers, any other mix text; a workbook of one such sheet makes one table named after the file; a workbook is read again only when it changes, and one over 64 MB is not read.
- Validation: a collection with a local folder and a RustFS folder (browse, open, search, analyze, one place offline), restart and re-index without moving existing paths, Convert to File with a shared CSV collection, real Excel files (dates, formulas' values, several sheets, an empty sheet), and the acceptance workspace.

## PR 30: backup and restore (E1)

- **Settings › Backup.** Where to (a folder on this computer or another disk, a NAS, RustFS), how many to keep, the last result, its size and Back Up Now and Check Backup. Backups are made when the owner asks (decided 2026-11-19: no schedule in the first version). Progress, speed and the destination are shown as for every transfer.
- **What is in it.** The data folder: registries, settings, collections' tables (Iceberg, SQLite), analysis and knowledge data, the access log. Not in it: what can be rebuilt (indexes, caches), what is downloaded again (extensions, models), the identity seed (it is the recovery key), keychain credentials, and the owner's original files, which Oikome only references; their paths and hashes are recorded so a restore lists any that are missing.
- **How.** rustic_core (Apache-2.0 OR MIT; through dirs it brings option-ext, MPL-2.0, used unmodified and allowed for that crate alone, decided 2026-11-19) behind a small internal interface, writing a restic-format repository through the existing opendal places: encrypted, deduplicated, readable by the standard restic program if ever needed. A staging step makes a consistent copy (`VACUUM INTO` for every SQLite file; the catalog before the Iceberg and Lance files it names; compaction and cleanup paused) and writes a manifest with BLAKE3 hashes and row counts. The repository key is derived from the recovery key; the first version has no passphrase (decided 2026-11-19: restoring needs the recovery key anyway, for the identity).
- **Restore.** From the first run (Restore from Backup: the recovery key and where the backup is), then the owner restarts Oikome to take the restored identity (decided 2026-11-19; automatic once the desktop app exists). Restoring into another folder is left to the command line in the first version (decided 2026-11-19). Every restore goes to a scratch folder first and is checked (repository check, SQLite integrity, each table's current snapshot and each Lance dataset opened, hashes and counts against the manifest) before it is used. On another computer or folder, Iceberg tables are made again from their current snapshots (their absolute paths cannot move; older snapshots are only used to pin queries). Afterwards places and connectors are signed in again.
- **Done so far (node and API).** `oikome-backup` stages the data folder (SQLite through `VACUUM INTO`; Parquet and Lance data files linked, as their formats never rewrite them; everything else copied), writes the manifest (BLAKE3 per file, rows per SQLite table, the data folder's path as the catalog names it, and each in-place collection's folder and indexed files), and keeps it in a restic repository in `oikome-backup` inside the folder the owner chose, through `RemoteStore` (a folder, a NAS over WebDAV, RustFS). Backup, Check Backup and the list run through the API, the jobs in the background with their progress. Restoring (`restore_from_backup` on a new data center, or `oikome backup restore`) downloads and checks the newest backup into `.oikome/backup/pending` with the identity from the recovery key; the next start puts it in place before anything opens the data folder (removing replaced SQLite files' side files) and, when the folder moved, rewrites the paths Iceberg keeps absolute (metadata JSON, manifests and manifest lists, with their lengths) and those in the registries. The restore reports originals no longer where they were and storages to sign in to again.
- Validation: back up during writes and syncs, interrupted backups resumed, restore on the same and on another computer, a damaged repository found by Check Backup, retention, every destination, and an acceptance scenario that restores the acceptance workspace.
- Retention (decided 2026-11-19): one backup a day for the last 7 days, one a week for the last 4 weeks, one a month for the last 12 months; older ones are removed.

## PR 31: read-only datasets, then migration (E2)

- A preinstalled data source plugin connects existing datasets to a collection as read-only tables: Lance datasets by path and on RustFS (latest version), Iceberg tables from an explicit `metadata.json` (pinned to the snapshot seen when connecting, refreshed when the owner asks), Parquet and CSV on RustFS. Only SELECT runs on them; credentials come from the keychain per session as temporary DuckDB secrets; an offline dataset keeps its last-known schema and shows unavailable.
- Migration is a separate, deliberate action: an Iceberg table's current snapshot is copied into an Oikome table (history not kept), a Lance dataset's folder is copied with its versions; the source, its snapshot or version and the content hashes are recorded. Nothing migrates as a side effect of connecting.
- REST catalogs come later. Moving from DuckDB 1.5 (supported until March 2027) to 2.0 is its own step.
- Validation: real datasets of each kind locally and on RustFS (RustFS with `httpfs` is to be tested), offline behavior, snapshot pinning, grants on connected tables, and a migration checked against its source.

## PR 32: plugin sandbox and RAW previews (C)

- **A sandbox for plugin code:** wasmtime (Apache-2.0) with the component model; each call gets its own store with a memory limit and an epoch deadline, no network or environment, and reads exactly one granted file through a host interface. Preview declarations (PR 24) become executable through it.
- **Plugin pages on origins of their own.** Each plugin's page is served from an origin of its own (a name per plugin on this computer; another port when the app is opened by address from another device) instead of an opaque sandbox origin, so module workers and modern libraries run, while it still cannot reach the app's cookies or API and its CSP still forbids the network. Plugins stay apart from each other.
- **Where WebAssembly runs plugin code in the node:** format plugins that turn a file into rows, text or a preview (RAW first; app exports such as health or bank statements later); later, third-party data source connectors, with network access limited by the host to the hosts they declare; later still, functions a plugin adds to SQL and tasks lent to contacts beyond SQL (the open question in architecture §12). Not for AI models (they run natively on the GPU) and not for plugin pages (those are iframes).
- **RAW previews** as an official plugin: most RAW files embed a full-size JPEG, extracted by a small permissively licensed reader (TIFF-based CR2, NEF, ARW, DNG, ORF, PEF; CR3; RAF). Every Rust RAW decoder found is LGPL or GPL, so none enters the core; a full decode, if ever needed, stays inside the plugin.
- Validation: real RAW samples from several makers, resource limits and failure isolation, package size, and a plugin that tries to read another file or reach the network.

## After PR 32

- **A — BI on the node's DuckDB.** All analysis is SQL executed by Oikome's DuckDB behind one SQL gateway (the caller's principal, queries only, the semantic layer's grant and row-policy rewrite, only granted tables listed, time and memory limits, the access log); every way in is a door in front of it. First door: **Perspective as Oikome's own component in the Analysis view** (grid, pivot, filters, aggregates and charts), with an executor in the node that runs Perspective's generated SQL as single read-only queries (its temporary views kept per session, never created in DuckDB) — proven in the 2026-11-19 spike, see `research/next-phases.md` §5. Assistants make views over MCP as Perspective view configurations (JSON) that Oikome renders, so the expressive power of charts made for the owner is Perspective's. Later doors, each only after its own spike: Quack once Oikome and the client are on DuckDB 2.0 (Superset through duckdb-engine), or the PostgreSQL wire protocol for BI products that need it; Superset can then be a bundled companion. Starts with the app-CSP review (WebAssembly compilation, `blob:` workers) and a design for the owner. A and B can be one stage.
- **B — tables as knowledge (K3).** Analysis tables gain units and `measure`, `text` and `place` roles; text columns become `table_row` chunks; assistants get read-only `query_table`.

## Not planned

- **Media recognition (knowledge K4)**: speech to text for audio and video, text in images and scans, and image search. Taken out of the plan on 2026-11-19 and left undecided for later. The models compared and the `media` schema stay in [`research/next-phases.md`](research/next-phases.md) §3 and [`knowledge-schema.md`](knowledge-schema.md), so it can be taken up again without new research.

## Open decisions (2026-11-19)

- **DuckDB 2.0.** Not now (decided 2026-11-19). Oikome keeps no DuckDB files (DuckDB runs in memory; data is in Iceberg, SQLite and Lance), so backups do not depend on it; and duckdb-rs for 2.0 (1.20000.x) needs Arrow 60 while iceberg 0.11, lance 14 and DataFusion 55 are on Arrow 58–59. Upgrade when they reach Arrow 60, before Quack (which needs 2.0 on both sides); 1.5 is supported until March 2027.
- **Perspective built into the Analysis view** rather than shipped as a plugin. Recommended; to confirm with the app-CSP review.

## Later phases

After A and B, the existing architecture order continues with security mode (3.7), desktop (M6), phone/devices (M7), family (M9) and serving over WebDAV (3.13). M5 payments and M10 third-party plugin distribution remain last. Security work remains deliberate and does not delay the content capabilities above unless a concrete threat requires reordering.

## Review gates

Each PR starts with a short review of its API/migration and prototype, then implements backend and UI tests together. A stage is complete only when its real-node browser scenario passes, all nine languages are checked, the production build is verified and the PRD, architecture, prototype and this queue agree. Unverified future capabilities stay marked as planned, never as supported.
