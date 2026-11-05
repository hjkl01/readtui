# Roadmap

## Phase 1 — Reading foundation

- [x] Rust CLI and cross-platform terminal setup
- [x] Local TXT/Markdown library scan
- [x] UTF-8, BOM-marked UTF-16 LE/BE, and legacy Chinese encoding fallback
- [x] Basic Chinese/English chapter heading detection
- [x] Keyboard navigation and interactive chapter list
- [x] SQLite reading progress and bookmarks
- [x] Literal search in current book
- [x] Terminal-width-aware visual wrapping and visual-row pagination
- [x] Stable byte-offset + source-line + wrapped-row progress, with legacy database migration
- [x] Large plain-text file streaming and byte-offset line index
- [x] Light/dark theme toggle and daily reading goal
- [x] Configurable horizontal and vertical reading margins
- [ ] Paragraph spacing and user-defined typography

## Phase 2 — Library management

- [x] Open/import an external file path interactively
- [x] Batch directory import and library rescan command
- [x] Book title and author editing
- [x] Tags, favorites, and unread/reading/finished status
- [ ] Collections and hierarchical grouping
- [x] Tag-based filtering
- [x] Content-fingerprint duplicate detection
- [x] Missing-file path relinking and stale-record cleanup
- [x] Recent books sorting and favorites-first sorting
- [x] Favorite/status filters and tag filters
- [ ] Multi-column sorting

## Phase 3 — Reading tools

- [x] Bookmark browser, jump, and delete
- [x] Paragraph annotations and excerpts with jump/delete
- [x] Current-book and cross-library literal search
- [x] Annotation editing and Markdown export
- [ ] Chapter-only search
- [ ] Configurable chapter detection rules
- [x] Exact character-column restoration within a source line after reflow

## Phase 4 — Formats

- [x] EPUB 2/3 spine extraction and cached text reading
- [x] EPUB 2 NCX / EPUB 3 nav chapter-label extraction
- [x] Read EPUB title and creator metadata
- [x] Basic Markdown markup cleanup
- [x] Basic HTML and FB2 text extraction
- [ ] Evaluate MOBI/AZW3 and PDF support

## Phase 5 — Statistics and polish

- [x] Reading session duration and total/today/rolling-week/month statistics
- [x] Daily reading goal
- [x] Light/dark theme toggle
- [x] Reading streaks (consecutive calendar days with a reading session)
- [ ] User-defined key bindings and configurable typography
- [x] SQLite database backup and integrity-checked restore
- [x] Orphan book/tag cleanup
- [x] Windows/macOS/Linux release binary workflow on version tags
