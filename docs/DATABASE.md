# Database Design

SQLite is the local metadata store. Original novel content remains in user-selected files.

## Initial schema

- `reading_progress`: latest stable reading position for each file (prototype currently stores source line).
- `bookmarks`: saved locations and optional notes.
- `reading_sessions`: session start/end and duration.

## Planned schema

- `books`: stable book identity, title, author, description, language, cover reference, and timestamps.
- `book_files`: one or more file paths per book, format, size, modification time, and content hash.
- `chapters`: chapter order, title, source offset, and source length.
- `reading_progress`: book ID, chapter ID, paragraph/character offset, percentage cache, and updated time.
- `reading_sessions`: start/end timestamps, duration, and words read.
- `bookmarks`: stable location, label, and creation time.
- `annotations`: selected text, note, location, and timestamps.
- `categories`, `tags`, `book_tags`: user-defined organization.
- `reading_goals`: daily/weekly targets.
- `search_index`: FTS5 virtual table, with a Chinese-aware tokenizer strategy to be evaluated.

## Rules

- Use migrations for every schema change.
- Never store the entire novel body in SQLite by default.
- Preserve metadata and notes if a file is moved or temporarily unavailable.
- Use content hashes to detect duplicate imports, but do not hash huge files synchronously on the UI thread.
