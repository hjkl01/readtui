# Product Design

## Product goal

readtui is a local-first, keyboard-driven terminal reading application. Reading comfort and data reliability take priority over feature count.

## Core principles

1. Local-first: no account, server, telemetry, or network dependency is required for reading.
2. Reader-first: library administration must not interrupt immersive reading.
3. Chinese-first, multilingual: CJK line width, punctuation, encodings, and chapter headings are first-class concerns.
4. Format adapters: file parsing is separated from pagination and UI rendering.
5. Durable state: reading progress, bookmarks, notes, and history survive restarts.
6. Portable: Windows, Linux, and macOS are supported without requiring a graphics protocol.

## Main areas

- Library: recent books, all books, collections, tags, import, scan, and file relocation.
- Reader: typography, pagination, chapter navigation, progress, bookmarks, annotations, and search.
- Explorer: browse the filesystem and import one or many books.
- Search: metadata search and full-text search across the library.
- Statistics: reading time, daily activity, words read, goals, and completion history.
- Settings: themes, key bindings, encoding preferences, chapter rules, and storage paths.

## Reader behavior

The reader operates on a normalized document model rather than rendering raw format data. A document contains metadata, ordered chapters, and paragraphs. A layout engine converts paragraphs into terminal-width-aware visual rows. The UI consumes visual rows and maps their positions back to stable document offsets.

Progress must be stored as a stable location (book identity, chapter identity, and character/paragraph offset), not only as a percentage or terminal row. This allows restoration after terminal resizing or typography changes.

## File ownership

readtui indexes user-owned files in place. It must not silently move, rewrite, or delete original book files. Importing a book creates library metadata; optional copy-to-library behavior must be explicit.

## First milestone scope

The initial implementation supports TXT and Markdown as plain text, local library scanning, basic chapter detection, line-based reading navigation, literal search, SQLite progress, and bookmarks. It is a foundation, not the final feature set.
