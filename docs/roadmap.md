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
- Network configuration exists under Settings → Network. There is no persistent network-status entry in the top-level navigation.

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
- Use flomo as the composite example: configure HTML import into a chosen collection, then separately add its optional Recent Notes card with a read grant. Move Kimi Code, Codex and Claude Code adapters to the same manifest mechanism; official MCP packages need no installation, while external ones do.
- Operate the independent `oikome/oikome-plugins` official repository. Its CI publishes deterministic versioned ZIPs and an index with complete manifests, sizes and SHA-256 digests. Plugins lists that release index, and the node downloads only an owner-selected version, verifies it and uses the ordinary installation lifecycle. The external-only official `note-count` card proves this path. Publisher signing and third-party community distribution remain later work.
- Smart Home remains a later composite example after a real Home Assistant source exists. There is no built-in Smart Home rendering exception and no device control.
- Validation: capability isolation, no install-time side effects, card add/remove/unavailable/recovery behavior, grant boundaries, external MCP manifests, sandbox tests, the flomo composite example, and a real local HTTP registry install with checksum rejection.

## PR 25: persistent network access

- Add a network-status icon near the top-left node identity, using Anytype only as an interaction reference. It is an entry to the existing network model, not a second network stack.
- The icon and its popover show real state from `network()`: local-only, preparing/open for sharing, reachable, router-refused or error; what the port is open for; and the current peer-to-peer address when available.
- The popover offers Open Network Settings and the currently relevant allowed action. Full UPnP/NAT-PMP, Don't ask again, contact introductions and permission choices remain in Settings → Network.
- Keyboard, mobile and accessibility behavior matches the rest of the navigation. No green online indicator is shown without a real connectivity check.
- Validation: every network state, no false online status, settings deep link, port-permission choices, mobile layout and no duplicate configuration state.

## Later phases

After safe editing, land internal analysis profiles and direct-query versus derived-analysis actions, then read-only Iceberg/Lance/S3 dataset interoperability and deliberately evaluated migrations, then media metadata and optional semantic indexes. These stages follow `docs/data-analysis.md` and never expose storage formats as ordinary-user choices.

The existing architecture order then continues: 3.8 public shares, 3.7 security, 3.10 Save To, M6 desktop, M7 phone/devices, M9 family and 3.13 WebDAV. M5 payments and M10 third-party plugin distribution remain last. RAW and other complex content capabilities are distributed as optional plugins through the independent plugin library, not added to the light core.

## Review gates

Each PR starts with a short review of its API/migration and prototype, then implements backend and UI tests together. A stage is complete only when its real-node browser scenario passes, all nine languages are checked, the production build is verified and the PRD, architecture, prototype and this queue agree. Unverified future capabilities stay marked as planned, never as supported.
