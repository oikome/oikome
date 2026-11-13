# Content Viewing Plan

> **Status**: PR 18 and PR 19 are merged. PR 20 implements lazy-loaded image gestures, safe SVG, native media and source-linked retrieval; safe editing has not started.
>
> **Product rule**: a light core, complex capabilities installed as plugins, and an independent plugin library. Oikome is the owner's content hub and quick viewer, not a replacement for Office, Photoshop, Acrobat or a RAW developer.

## Scope

The core provides the content contract, viewer routing, safe local opening, preview caching and the plugin host. It natively presents common text, structured data, images, browser-decodable audio and video, and search results. Complex formats use an application on the owner's device or an optional content plugin. No heavy format engine enters the default download or first paint.

The first implementation does not include an Office suite, a PSD editor, a PDF editor, a RAW development tool, a transcoder, or a new vector-visualization system. PDF may later get a separately lazy-loaded, read-only PDF.js preview; it is not part of the core baseline. Univer is not a preview dependency. XLSX conversion and other separately distributed capabilities must not be assumed to be open-source because a core engine is.

## Content and presentation

Physical format and presentation are separate decisions. Extension, media type, file signature and a real parser establish what a file is; a parser that cannot understand it must not silently turn it into another kind. A schema or connector contract can recommend a semantic view. Sampling is only a recommendation, never proof. The owner can switch among applicable views and keep that choice. When uncertain, the most faithful raw or tabular view wins.

| Content | Core presentation | External or optional path |
| --- | --- | --- |
| Markdown and plain text | Proper Markdown reading, source view; later, safe source editing | An editor of the owner's choice |
| Code, JSON and configuration | Lazy-loaded syntax display, line numbers, folding and wrapping | An editor of the owner's choice |
| CSV, TSV, Parquet and JSON Lines | A typed data grid where the source has types, faithful text cells for CSV/TSV, column choice or a record view on phones | A spreadsheet or data tool |
| Collections and rows | A table by default; photos, notes and readings recommended from schema and connector contracts; the owner can override | — |
| JPEG, PNG, GIF and WebP | Thumbnails, full view, zoom, pan, keyboard and touch navigation, metadata | An image editor |
| SVG | A sanitized view with scripts and external references blocked | A vector editor |
| Audio and video | Native playback, range reads, fullscreen and clear codec failures | A player or editor; no bundled transcoder |
| PDF | Metadata and local opening in the baseline | A separately lazy-loaded read-only preview may be evaluated later |
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
- The PR 19 first-paint baseline was 666.01 kB of JS and 99.70 kB of CSS; PR 20 is 668.35 kB and 100.35 kB. PhotoSwipe (61.94 kB JS, 4.56 kB CSS) and DOMPurify (32.13 kB JS) remain separate lazy chunks. No Office, PSD, RAW or transcoding engine enters the core.

### PR 21: safe editing

Start with Markdown and plain text only, after the viewing stages. The editing interaction is reviewed again before implementation.

- Distinguish editing a source file, editing a collection and editing a new copy. A connector's SQLite working copy is not its CSV source; never let a later sync silently erase an edit described as permanent.
- Return capabilities when opening and check them again when saving. Remote shares, read-only bindings, unsupported formats, offline places and filesystem denial are different reasons.
- Carry the source version or hash. A file changed by another application is a conflict, not an overwrite opportunity.
- Create new files exclusively. Ask before replacing an existing file; never replace silently.
- Write through a temporary file and atomically replace where the place supports it. Preserve encoding, line endings, BOM and permissions where applicable.
- A read-only source offers Save a Copy and Edit, or Cancel. The original stays untouched.
- Remember confirmations separately for the current identity and for a normalized directory, stored on the node rather than only in one browser. Either scope can be revoked in Settings.
- Never suppress warnings for a version conflict, changed permission, different directory, different identity or a different file than the one confirmed.

### Separate track: content plugins and RAW Preview

Content/Preview is a third plugin kind beside Dashboard and Source. It runs on the host in a separate process or controlled WASM sandbox, not in the dashboard iframe. It receives only a single file read grant and its own temporary directory, has no network by default, and has CPU, memory, time and output limits. A parser failure cannot crash the node.

RAW Preview returns embedded JPEG and EXIF where available, or a developed JPEG derivative and whether the result is an embedded or developed preview. It never modifies RAW or XMP. The cache key includes source hash, plugin version and preview parameters; disabling or removing the plugin leaves original photos untouched. Unsupported cameras and absent plugins always retain local opening.

The independent plugin library distributes signed, versioned packages without becoming a runtime data service. A listing names publisher, signature, checksum, supported formats and platforms, plugin API version, download size, licenses, permissions and fidelity limits. Installation is explicit; official plugins use the same public protocol and have no hidden privileges.

## Validation

Every implementation stage updates the PRD, architecture, generated client types, prototype and all nine languages. Browser tests use a real local node and real files. Each stage checks desktop and phone screenshots, keyboard and touch operation, accessibility, content security, source conflicts and the production build. Optional dependencies record their license and attribution, and the build is compared with the previous baseline so a viewer does not silently become a heavy default dependency.
