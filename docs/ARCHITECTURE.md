# Architecture

## Reading pipeline

The reader keeps the original book path as its identity. Text formats are converted to cache files only when extraction or markup cleanup is required:

- TXT: read directly from the original file.
- EPUB: read the OPF manifest and spine, extract XHTML paragraphs in spine order, use EPUB 2 NCX / EPUB 3 nav labels as chapter headings where available, and cache the extracted text. The OPF title and creator are imported as defaults when no user metadata exists.
- Markdown: remove common heading, emphasis, link, quote, and fenced-code markup while retaining readable text.
- HTML: remove scripts/styles and tags, preserve common block boundaries, and decode common named entities.
- FB2: parse XML and extract title, paragraph, verse, and author blocks.

The cache key includes the source path, file size, and modification time. The original file is never modified.

## Large text files

Document::open performs one sequential buffered scan and records the byte offset of each line. It does not retain the full document in memory. The reader loads only the lines needed for the current viewport. Search and chapter detection also use sequential buffered reads.

Memory use is approximately proportional to the number of lines (one u64 offset per line), plus the current viewport and chapter index. A file containing millions of very short lines still needs a sizable offset index; a disk-backed index is a future optimization.

## Stable progress

The SQLite progress record stores:

- line: source line index, retained for compatibility and quick navigation.
- byte_offset: byte offset of the current source line in the active text stream.
- wrap_offset: visual row offset within that source line (compatibility fallback).
- char_offset: Unicode character index at the start of the current visual row; it is used to remap the location after terminal-width changes.

The byte offset is independent of terminal width. On restore, the reader maps it back to the closest indexed line, then maps the saved character index into the visual rows at the current terminal width. Existing databases are migrated in place.

The initial line-only database is migrated in place by adding missing columns. Existing bookmarks receive the same offset columns.

## Local database

The database is stored under the platform configuration directory in readtui/readtui.db.

Tables:

- reading_progress: last reading location per original book path.
- bookmarks: saved locations.
- annotations: paragraph quote and user note.
- reading_sessions: session start/end and duration.
- book_metadata: title, author, favorite flag, and reading status.
- tags / book_tags: user-defined labels.
- settings: theme and daily reading goal.

Original source files are not copied into SQLite.

## Current limitations

- EPUB NCX/nav labels and OPF title/creator are consumed, but a hierarchical navigation panel, landmarks, cover images, inline images, footnotes, and complex layout are not exposed in the UI.
- HTML conversion is intentionally lightweight and is not a full browser DOM renderer.
- Markdown conversion is text-oriented, not a complete CommonMark renderer.
- FB2 extraction supports common text blocks; images, binary attachments, and metadata editing are not exposed.
- Legacy Chinese decoding is line-oriented. UTF-16 and user-selectable encodings are not yet implemented.
- Cross-library search returns the first match per book and is a sequential scan, not a persistent full-text index.
- Notes are attached to the current source line; selecting arbitrary text and editing existing annotations are future work.
- Daily, rolling-week, monthly, total statistics, daily goals, and consecutive reading-day streaks are implemented; charts and statistics export are not yet implemented.
