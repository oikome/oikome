# Product Delivery Plan

> **Status**: consolidated 2026-11-13. This is the single implementation queue for the current app work. It preserves the agreed content-viewing stages and adds the data-source, plugin and network work identified during the PR 19 review. Code, the PRD and the prototype must be updated together as each stage lands.

## Rules

- Adding content targets an existing collection by default. Creating a collection is always a separate, explicit choice.
- A source and its destination are different things. A connector must not silently become a collection merely because it was configured.
- Rendering follows a declared data contract. Source names, collection names and hard-coded Smart Home exceptions are not presentation rules.
- Installing a plugin registers capabilities; it does not implicitly start a source, grant data or put a card on Home. Each capability is configured explicitly.
- Home shows real node state only. Prototype data belongs to `docs/prototype/`; generated examples are identified as examples and are not represented as user activity.
- Existing media and safe-editing commitments keep their places. New work must not silently displace them.

## Current behavior, verified in code

- `add_source` currently derives one new collection name from the source and calls `session.add_source` with that same name. `SourceDef.name` is also documented as the Space name. This is the one-to-one model that needs migration.
- Built-in connectors are CSV, flomo export/MCP and the legacy photo/note folder connectors. There is no Home Assistant connector or Smart Home source plugin in the node. Smart Home appears in the PRD and prototype only.
- Dashboard plugins already have a manifest, a data-shape contract, per-collection grants and a sandboxed Home card. Source and Content/Preview capabilities are not yet parts of that plugin manifest.
- Home has Arrange and Add a Card. With no saved arrangement it computes a default from real collections and installed plugins. The arrangement is persisted in `.oikome/node.json` through `set_home_cards`; it is not fake state.
- Deleting a collection hides cards that no longer resolve. It does not erase the saved card id or permanently ban that card type, but stale entries need an explicit removed/restore experience. Any compatible card can be added again through Arrange → Add a Card.
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

- Keep ordinary first-run examples unchanged. One `tests/scenarios/acceptance` workspace covers content, local files, mounted NAS and drive-sync-folder test doubles, real RustFS, and a real second node with an accepted share; do not split these into separate demos that people must switch between.
- `npm run acceptance` opens the fully seeded workspace and its peer. `npm run acceptance -- --first-run` starts empty and completes setup through the real UI. Every launch uses fresh isolated state, never `~/Oikome`, a mock `NodeClient`, committed node snapshots or committed generated media.
- Mounted/sync folders, OAuth stand-ins and real cloud accounts remain distinctly named levels of verification; no fake connected cloud state is shown.
- The launcher prints and opens the main and peer URLs. Generated roots are marked and ignored; only those roots may be reset, and running the same command again opens the existing workspace rather than resetting it.

## PR 21: safe editing

Keep the agreed scope: Markdown and plain text first, explicit source versus copy behavior, read-only detection, exclusive creation, overwrite confirmation, per-identity/directory confirmation preferences, source-version conflicts and atomic replacement where supported. The editing interaction is reviewed again before implementation.

## PR 22: data-source targets and semantic contracts

- Add a clear Add Data Source flow beside Add Files, with the same existing-collection-first target rule. Do not hide connector configuration inside file import.
- Separate source identity from destination. Introduce destination collection/table fields in the API and source registry rather than deriving the Space from `SourceDef.name`. Migrate existing sources with their current name as the destination so behavior and grants survive.
- CSV configuration names a target collection and table, validates schema compatibility and reports conflicts before sync. It must not create a same-named collection implicitly. Upsert keys and cursor commits retain their current transactional guarantees.
- Let a connector declare table semantics and a recommended view (photos, notes, table or series), with the existing schema inference only as a fallback. No rendering branch may depend on a source or collection name.
- Define whether a connector may write several tables before implementing it; do not imply that capability in the UI until the backend supports it.
- Validation: existing-source migration, CSV into an existing collection, destination/schema conflicts, grant preservation, scheduled and manual sync, and no implicit collection creation.

## PR 23: plugin capabilities and Home composition

- Extend the plugin manifest with independent optional capabilities such as `sources`, `cards` and `preview`. A plugin may provide several, each with its own configuration, grants and lifecycle. Installation alone changes no data and adds no Home card.
- Keep Dashboard cards in the browser sandbox. Source plugins run on the node under the connector permission model; Content/Preview plugins use their separate host sandbox. Do not give one plugin kind another kind's runtime privileges.
- Make Add a Card and each card's collection/grant configuration discoverable. Preserve existing layouts during migration. A deleted collection produces an explicit unavailable/restore state rather than silently losing the card definition forever.
- Use flomo as the composite example: the flomo capability configures import into a chosen collection, and an optional card is separately added to Home with its own read grant.
- Smart Home is a later composite example, only after a real Home Assistant source exists. Its source declares sensor semantics; a separate card renders rooms and readings. There is no built-in Smart Home rendering exception and no device control.
- Validation: capability isolation, no install-time side effects, card add/remove/restore, deleted-collection behavior, grant boundaries, sandbox tests and composite-plugin examples.

## PR 24: persistent network access

- Add a network-status icon near the top-left node identity, using Anytype only as an interaction reference. It is an entry to the existing network model, not a second network stack.
- The icon and its popover show real state from `network()`: local-only, preparing/open for sharing, reachable, router-refused or error; what the port is open for; and the current peer-to-peer address when available.
- The popover offers Open Network Settings and the currently relevant allowed action. Full UPnP/NAT-PMP, Don't ask again, contact introductions and permission choices remain in Settings → Network.
- Keyboard, mobile and accessibility behavior matches the rest of the navigation. No green online indicator is shown without a real connectivity check.
- Validation: every network state, no false online status, settings deep link, port-permission choices, mobile layout and no duplicate configuration state.

## Later phases

The existing architecture order remains after these app stages: 3.8 public shares, 3.7 security, 3.10 Save To, M6 desktop, M7 phone/devices, M9 family and 3.13 WebDAV. M5 payments and M10 third-party plugin distribution remain last. RAW and other complex content capabilities are distributed as optional plugins through the independent plugin library, not added to the light core.

## Review gates

Each PR starts with a short review of its API/migration and prototype, then implements backend and UI tests together. A stage is complete only when its real-node browser scenario passes, all nine languages are checked, the production build is verified and the PRD, architecture, prototype and this queue agree. Unverified future capabilities stay marked as planned, never as supported.
