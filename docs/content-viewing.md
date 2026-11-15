# Content Viewing Plan

> **Status**: PR 18 through PR 22 are merged. PR 23 implements lazy, read-only PDF preview; complex professional formats retain optional plugin and local-opening paths.
>
> **Product rule**: a light core, complex capabilities installed as plugins, and an independent plugin library. Oikome is the owner's content hub and quick viewer, not a replacement for Office, Photoshop, Acrobat or a RAW developer.

## Scope

The core provides the content contract, viewer routing, safe local opening, preview caching and the plugin host. It natively presents common text, structured data, images, browser-decodable audio and video, and search results. Complex formats use an application on the owner's device or an optional content plugin. No heavy format engine enters the default download or first paint.

The first implementation does not include an Office suite, a PSD editor, a PDF editor, a RAW development tool, a transcoder, or a new vector-visualization system. PR 23 adds a separately lazy-loaded, read-only PDF.js preview outside the first-paint baseline. Univer is not a preview dependency. XLSX conversion and other separately distributed capabilities must not be assumed to be open-source because a core engine is.

## Content and presentation

Physical format and presentation are separate decisions. Extension, media type, file signature and a real parser establish what a file is; a parser that cannot understand it must not silently turn it into another kind. A schema or connector contract can recommend a semantic view. Sampling is only a recommendation, never proof. The owner can switch among applicable views and keep that choice. When uncertain, the most faithful raw or tabular view wins.

| Content | Core presentation | External or optional path |
| --- | --- | --- |
| Markdown and plain text | Proper Markdown reading, source view and safe local editing | An editor of the owner's choice |
| Code, JSON and configuration | Lazy-loaded syntax display, line numbers, folding, wrapping and safe local editing | An editor of the owner's choice |
| CSV, TSV, Parquet and JSON Lines | A typed data grid where the source has types, faithful text cells for CSV/TSV, source editing for local UTF-8 CSV/TSV/JSON text, column choice or a record view on phones | A spreadsheet or data tool |
| Collections and rows | A table by default; photos, notes and readings recommended from schema and connector contracts; the owner can override | — |
| JPEG, PNG, GIF and WebP | Thumbnails, full view, zoom, pan, keyboard and touch navigation, metadata | An image editor |
| SVG | A sanitized view with scripts and external references blocked | A vector editor |
| Audio and video | Native playback, range reads, fullscreen and clear codec failures | A player or editor; no bundled transcoder |
| PDF | Lazy PDF.js read-only preview with current-page rendering, pagination, zoom and text selection | The default application for editing, forms and signatures |
| PPT, DOCX, XLSX, PSD and similar professional formats | Metadata, location and Open in Default App | The native application; an optional approximate preview plugin must state its limits |
| Camera RAW | Metadata and local opening without a plugin | An optional RAW Preview plugin extracts embedded JPEG/EXIF or develops a derivative preview |
| Vector indexes and search | Hits, excerpts, source location and the original content | High-dimensional vectors are an AI index, not a human reading layout |

Unsupported, damaged, offline and unrecognized files keep their real name, format, size, modified time and place. The interface never shows an empty viewer as though a preview succeeded. Desktop and phone layouts choose columns and controls for the available space rather than hiding essential content behind an undiscoverable horizontal scroll.

## Calculation boundaries

DuckDB owns analytical Spaces and analytical tasks; SQLite owns transactional Spaces, connector writes, FTS5 and sqlite-vec. DataFusion coordinates cross-Space and remote queries and merges Arrow streams. The node performs data computation; browser code formats and draws returned values.

Two existing exceptions must stay visible to people: in-place file previews use dedicated Arrow readers and at most the first 64 MiB, while an unindexed search reads at most 10,000 rows before matching in Rust. Neither may be described as a complete read or search of the whole source. Small presentation transforms in JavaScript are not substitutes for whole-table aggregation.

## Delivery stages

### PR 18: content contract and viewer foundation

- Return a typed content descriptor: physical format, available views, source, version, capabilities and a human-readable reason when an operation is unavailable.
- Add viewer routing and shared loading, empty, damaged, offline and unsupported states.
- Add Open in Default App through direct operating-system argument lists, never a shell command. Name the computer on which the application will open; do not confuse it with the browser's device.
- Keep the existing light viewers working while the new contract is introduced.
- Test misleading extensions, absent extensions, unsafe paths, unavailable places and unsupported formats.

### PR 19: text and structured content

- Render Markdown with react-markdown and remark-gfm. Do not enable uncontrolled raw HTML or remote resources.
- Lazy-load CodeMirror for source and code views. Wrap long lines by default without a separate toggle; place Read/Source or Table/Source switching in a quiet content toolbar, not in the file-action header.
- Validate AG Grid Community with React 19, Arrow data, the existing themes, mobile layouts and bundle output before adopting it. Load only required Community modules.
- Add useful column widths, a sticky header, faithful value formatting, long-text behavior, copying, and column or record choices on phones. Keep Copy beside the view switch in one toolbar: Markdown and source copy the whole file, tables copy the current page as TSV, failures are shown, and Cmd/Ctrl+A then Cmd/Ctrl+C selects and copies source content rather than the whole page.
- Cover Markdown injection, wide tables, long CJK and Latin text, nulls, dates without zones and decimals in all nine languages.
- Keep a damaged JSON file available as source while isolating its unavailable table binding, rather than failing the collection's whole `files` query. Open damaged JSON directly as source without offering a table action that must fail. Count in-place collections on their actual storage page.
- Allow only the viewer libraries' inline styles in CSP; inline scripts remain blocked. The file dialog uses three grid rows normally and four with Info open, so the data grid cannot collapse when details are toggled.
- One Add Files action replaces separate Import Files and View a Folder entries. Keep in Place references selected original files or a whole folder without copying; Copy to Oikome deliberately copies into the current collection or subfolder and refreshes it there. The current collection is the default target, never requiring the owner to go back to the overview. Selected files from several folders can belong to one collection without exposing unselected siblings. An empty analytical collection's first add of either kind turns it into a file collection while keeping its name and whole-Space grants; a non-empty collection is never converted implicitly. The overview asks for a destination collection; creating a new one is an explicit option, never the default. Copies select files before naming an explicitly new collection, suggest an editable name from the first file, preserve original filenames and reject duplicate base names before submission. Copies never replace existing files and are limited to 32 MiB decoded (48 MiB JSON body); ordinary API requests remain limited to 1 MiB. Came from shows the current file's full original or remote-provider path, never the index or cache path. Analysis later produces structured records, chunks and vectors from either references or copies; it does not itself require another copy of the original.

### PR 20: media and retrieval results

- Lazy-load PhotoSwipe 5.4.4 for image navigation, zoom, pan and touch gestures; do not implement a second gesture engine. Gallery changes update the same file address and preserve Info; originals load only for the current and adjacent slides.
- Use native audio/video playback and single-range HTTP reads. Keep local opening for unsupported codecs and show a playback failure rather than a blank viewer.
- Sanitize SVG on demand with DOMPurify 3.4.17 under Apache-2.0, remove scripts, event handlers, foreign content and external references, and serialize to a blob image rather than inserting SVG into the page. Raw SVG responses also carry a restrictive CSP.
- Present search and vector hits as excerpts with real links to their original content, preserving keyboard selection and modified-click behavior, not as a meaningful visualization of an embedding.
- The PR 19 first-paint baseline was 666.01 kB of JS and 99.70 kB of CSS; PR 20 is 668.56 kB and 100.58 kB. PhotoSwipe (61.94 kB JS, 4.56 kB CSS) and DOMPurify (32.13 kB JS) remain separate lazy chunks. No Office, PSD, RAW or transcoding engine enters the core.

### PR 22: safe editing

Local UTF-8 text is editable up to 512 KiB, using the existing CodeMirror chunk with no new editor dependency. Known media, professional-document and binary-table formats are excluded, then real UTF-8 bytes determine editability without an editable-extension allowlist; filenames recommend syntax highlighting and table parsing. This covers Markdown, code, configuration, extensionless text and CSV/TSV/JSON text tables. Copy and Edit form one right-aligned action group.

- Distinguish editing a source file, editing a collection and editing a new copy. A connector's SQLite working copy is not its CSV source; remote and read-only bindings are not presented as writable.
- Return capabilities when opening and check them again when saving. Unsupported formats, offline files, filesystem denial and read-only originals have different outcomes; a writable folder still permits Save a Copy.
- Carry the real BLAKE3 content hash. A file changed by another application is a conflict with a reload action, not an overwrite opportunity.
- Create new files exclusively in the source file's directory. Replacing an existing copy requires its target hash and confirmation; path traversal, links and unindexed sibling targets are refused.
- Write replacements through a same-directory temporary file and atomic rename. Preserve UTF-8 BOM, CRLF/CR line endings and existing permissions.
- Remember source-save and overwrite confirmations separately for the current identity and normalized directory in `.oikome/node.json`. Records include the action, scope, identity, canonical file, directory, version and permissions; Settings lists and revokes them.
- Never suppress warnings for a version conflict, changed permission, different directory, different identity or a different file than the one confirmed. A successful save advances only its own remembered version.
- The PR 22 first-paint baseline is 674.01 kB of JS and 101.57 kB of CSS; the separate TextEditor chunk is 3.62 kB. No editor, parser or other runtime dependency was added.

### PR 23: lazy PDF preview

PDF uses PDF.js 6.5.438's display layer and a dedicated Worker, not the full viewer. A file grid shows the first page when its tile approaches the viewport, without a text layer and with pixel density capped at 1.5. The open viewer renders only the current page, at a width that fits its container, with pixel density capped at 2, pagination, zoom and a selectable text layer. Changing pages cancels the old render; closing or changing the file destroys the loading task and Worker. `disableAutoFetch` prevents eager whole-document retrieval, while character maps, standard fonts, WASM and image decoders are addressed as on-demand resources.

- Evaluation measured 463.2 KiB for the minified display layer and 1242.1 KiB for the Worker (about 504 KiB gzip combined). The npm package's 37.6 MiB unpacked size is not the browser payload. CMaps, standard fonts, WASM and image decoders occupy separate on-demand resource directories.
- A smoke run with PDF.js's 992.5 KiB, 14-page sample rendered its first page in 58–75 ms over three runs on the development machine. The real app's small three-page acceptance document opened in about 712 ms, including node and content loading. These are not low-end-device or complex-document benchmarks; memory and large-document behavior remain explicit performance gates.
- Production keeps PDF.js out of the first paint and collections without PDF tiles. The current separate chunks are 454.91 kB for `PdfView` and 1271.92 kB for the Worker; the first-paint bundle is 677.22 kB JS and 103.44 kB CSS after unifying file artwork. Browser tests assert no PDF engine request before reaching PDF tiles, then check real thumbnail pixels and the full viewer.
- PDFs remain read-only and open through the existing indexed original-file endpoint with range support. Raw PDF responses have a restrictive CSP. Damage retains the file-kind icon in a grid and shows a useful failure with the existing Open in Default App action in the viewer.

One shared file-art component routes grid and collection-cover previews. SVG uses the same sanitization and blob-image path as its full viewer. Near-viewport video uses a paused native element with metadata loading to show a real frame, never autoplay or a transcoder; audio uses a distinct audio icon rather than a fabricated waveform. In-place folder covers still prefer photos then notes, but otherwise show up to three real files and a file count instead of incorrectly reporting no data.

PSD, RAW, Office and extra media codecs are not bundled into this stage. Approximate previews are acceptable in explicitly optional plugins, but their fidelity limits and resource costs must be stated.

### Separate track: content plugins and RAW Preview

Content/Preview is a third plugin kind beside Dashboard and Source. It runs on the host in a separate process or controlled WASM sandbox, not in the dashboard iframe. It receives only a single file read grant and its own temporary directory, has no network by default, and has CPU, memory, time and output limits. A parser failure cannot crash the node.

RAW Preview returns embedded JPEG and EXIF where available, or a developed JPEG derivative and whether the result is an embedded or developed preview. It never modifies RAW or XMP. The cache key includes source hash, plugin version and preview parameters; disabling or removing the plugin leaves original photos untouched. Unsupported cameras and absent plugins always retain local opening.

The independent plugin library distributes signed, versioned packages without becoming a runtime data service. A listing names publisher, signature, checksum, supported formats and platforms, plugin API version, download size, licenses, permissions and fidelity limits. Installation is explicit; official plugins use the same public protocol and have no hidden privileges.

## Validation

Every implementation stage updates the PRD, architecture, generated client types, prototype and all nine languages. Browser tests use a real local node and real files. Each stage checks desktop and phone screenshots, keyboard and touch operation, accessibility, content security, source conflicts and the production build. Optional dependencies record their license and attribution, and the build is compared with the previous baseline so a viewer does not silently become a heavy default dependency.
