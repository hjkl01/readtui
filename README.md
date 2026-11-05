# readtui

A terminal-based reader and local library manager for novels and ebooks, written in Rust.

readtui is designed for distraction-free reading in Windows Terminal, WezTerm, Kitty, and ordinary SSH terminals. It prioritizes local files, reliable reading progress, Chinese text, and keyboard-driven navigation.

## Current status

The first development milestone provides a working foundation:

- Browse local `.txt`, `.md`, `.markdown`, `.epub`, `.html`, `.htm`, and `.fb2` books recursively.
- Read large plain-text files through a byte-offset line index instead of loading the entire book into a `Vec<String>`.
- Convert EPUB spine chapters (including NCX/nav chapter labels), Markdown, HTML, and FB2 into a cached plain-text reading stream.
- Import EPUB title and creator metadata as defaults when opening a book.
- Open a book directly from the command line or from the library screen.
- Decode UTF-8, BOM-marked UTF-16 LE/BE, GB18030/GBK-compatible text, and Big5 fallback.
- Detect common Chinese and English chapter headings.
- Navigate with Vim-style keys, visual-row-aware page keys, and an interactive chapter list.
- Wrap CJK text by terminal cell width and keep common opening/closing punctuation with adjacent text.
- Persist byte offsets, source line, wrapped-row offset, and a source-character anchor (so reading position survives terminal resizing), plus bookmarks, annotations, and reading sessions in SQLite, with automatic migration from the original schema.
- Browse, jump to, and delete bookmarks; save paragraph notes and excerpts.
- Search the current book or search across the local library.
- Manage book title, author, tags, favorites, and reading status; filter by favorite/status and tag.
- Configure reading-area margins; back up and restore the SQLite database.
- Detect duplicate files by content fingerprint, relink moved/missing books, and prune stale database records.
- Track reading time, consecutive reading days, and a daily reading goal; switch between light and dark themes.

The current format pipeline extracts EPUB 2/3 spine content, uses NCX/nav labels as chapter headings where available, and reads OPF title/creator metadata. It converts Markdown, HTML, and FB2 to cached text. A hierarchical EPUB navigation tree, images, footnotes, and advanced semantic formatting are not yet represented.

## Install

A Rust stable toolchain is required.

```bash
cargo install --git https://github.com/hjkl01/readtui
```

Or build from source:

```bash
git clone https://github.com/hjkl01/readtui.git
cd readtui
cargo build --release
```

The executable is `target/release/readtui` (Windows: `target/release/readtui.exe`).

## Usage

Open the default library at `~/Books`:

```bash
readtui
```

Choose a different library directory:

```bash
readtui --library ./novels
```

Open a file directly:

```bash
readtui ./novels/三体.txt
```

Database maintenance:

```bash
readtui --backup ./readtui-backup.db
readtui --restore ./readtui-backup.db
readtui --duplicates --library ./novels
readtui --prune-missing
readtui --relink "/old/path/book.txt=/new/path/book.txt"
```

`--prune-missing` removes stale book metadata, progress, bookmarks, notes, and tag links. `--relink` migrates a book's path references to an existing file.

From the library screen, press `a` and enter a file path to open a file outside the library directory.

## Keys

Library:

| Key | Action |
|---|---|
| `j` / `k`, arrows | Select a book |
| `Enter` | Open selected book |
| `a` | Add/open an external file path (persisted in the library) |
| `A` | Import a directory recursively |
| `r` | Rescan the library |
| `/` | Search across the library |
| `f` | Toggle favorite |
| `s` | Cycle unread / reading / finished |
| `t` | Add a tag |
| `e` | Edit title and author (`title|author`) |
| `x` | Clear tags on selected book |
| `o` | Sort favorites first |
| `R` | Sort by recent reading progress |
| `F` | Cycle all / favorite / reading / unread / finished filters |
| `T` | Filter by tag (empty input clears tag filter) |
| `P` | Configure horizontal and vertical reading margins (`horizontal,vertical`) |
| `G` | Set daily reading goal in minutes |
| `c` | Toggle light/dark theme |
| `S` | Reading statistics (today, rolling 7 days, month, total, reading streak) |
| `q` | Quit |

Reader:

| Key | Action |
|---|---|
| `j` / `k`, arrows | Scroll one visual row |
| `Space` / `PageDown` | Move forward one screen |
| `PageUp` | Move backward one screen |
| `g` / `Home`, `G` / `End` | Beginning / end |
| `n` / `N` | Next / previous detected chapter |
| `t` | Open the table of contents |
| `/` | Search the current book |
| `C` | Search within the current chapter |
| `K` | Configure custom chapter-heading regex rules (`;` separated; empty resets defaults) |
| `b` / `B` | Add bookmark / browse bookmarks |
| `m` / `M` | Add paragraph note / browse notes |
| `e` / `d` in notes | Edit / delete selected note |
| `E` | Export notes and bookmarks to Markdown |
| `c` | Toggle light/dark theme |
| `S` | Reading statistics |
| `q` / `Esc` | Return to library |

## Data locations

- Library: `~/Books` by default.
- Database: `$XDG_CONFIG_HOME/readtui/readtui.db` on Linux, or the platform configuration directory returned by `dirs`.
- Extracted format cache: the platform cache directory under `readtui/formats`.

Original book files are never copied into the database. SQLite stores reading positions, bookmarks, notes, metadata, tags, settings, and session records. EPUB/Markdown/HTML/FB2 extraction is cached separately; cache files can be regenerated from the originals.

## Development

```bash
cargo fmt --all
cargo test
cargo run -- ./path/to/novel.txt
```

See [`docs/`](docs/) for architecture, database, format, and roadmap documents.

## Releases

Push a version tag such as `v0.1.0` to trigger the release workflow. GitHub Actions builds native release binaries for Linux x86_64, Windows x86_64, and macOS, then attaches them to a GitHub Release.

```bash
git tag v0.1.0
git push origin v0.1.0
```

## License

MIT
