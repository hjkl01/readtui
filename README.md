# readtui

A terminal-based reader and local library manager for novels and ebooks, written in Rust.

readtui is designed for distraction-free reading in Windows Terminal, WezTerm, Kitty, and ordinary SSH terminals. It prioritizes local files, reliable reading progress, Chinese text, and keyboard-driven navigation.

## Current status

The first development milestone provides a working foundation:

- Browse local `.txt`, `.md`, `.markdown`, `.epub`, `.html`, `.htm`, and `.fb2` books recursively.
- Read large plain-text files through a byte-offset line index instead of loading the entire book into a `Vec<String>`.
- Convert EPUB spine chapters, Markdown, HTML, and FB2 into a cached plain-text reading stream.
- Open a book directly from the command line or from the library screen.
- Decode UTF-8, GB18030/GBK-compatible text, and Big5 fallback.
- Detect common Chinese and English chapter headings.
- Navigate with Vim-style keys, visual-row-aware page keys, and an interactive chapter list.
- Wrap CJK text by terminal cell width and keep common opening/closing punctuation with adjacent text.
- Persist byte offsets, source line, wrapped-row offset, bookmarks, annotations, and reading sessions in SQLite, with automatic migration from the original schema.
- Browse, jump to, and delete bookmarks; save paragraph notes and excerpts.
- Search the current book or search across the local library.
- Manage book title, author, tags, favorites, and reading status.
- Track reading time and a daily reading goal; switch between light and dark themes.

The current format pipeline extracts EPUB 2/3 spine content and converts Markdown, HTML, and FB2 to cached text. EPUB navigation landmarks and advanced semantic formatting are not yet fully represented.

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

From the library screen, press `a` and enter a file path to open a file outside the library directory.

## Keys

Library:

| Key | Action |
|---|---|
| `j` / `k`, arrows | Select a book |
| `Enter` | Open selected book |
| `a` | Add/open a file path |
| `/` | Search across the library |
| `f` | Toggle favorite |
| `s` | Cycle unread / reading / finished |
| `t` | Add a tag |
| `e` | Edit title and author (`title|author`) |
| `x` | Clear tags on selected book |
| `o` | Sort favorites first |
| `G` | Set daily reading goal in minutes |
| `c` | Toggle light/dark theme |
| `S` | Reading statistics |
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
| `b` / `B` | Add bookmark / browse bookmarks |
| `m` / `M` | Add paragraph note / browse notes |
| `c` | Toggle light/dark theme |
| `S` | Reading statistics |
| `q` / `Esc` | Return to library |

## Data locations

- Library: `~/Books` by default.
- Database: `$XDG_CONFIG_HOME/readtui/readtui.db` on Linux, or the platform configuration directory returned by `dirs`.

Original book files are never copied into the database. SQLite stores reading positions, bookmarks, and session records.

## Development

```bash
cargo fmt --all
cargo test
cargo run -- ./path/to/novel.txt
```

See [`docs/`](docs/) for architecture, database, format, and roadmap documents.

## License

MIT
