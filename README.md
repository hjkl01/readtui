# readtui

A terminal-based reader and local library manager for novels and ebooks, written in Rust.

readtui is designed for distraction-free reading in Windows Terminal, WezTerm, Kitty, and ordinary SSH terminals. It prioritizes local files, reliable reading progress, Chinese text, and keyboard-driven navigation.

## Current status

The first development milestone provides a working foundation:

- Browse a local library recursively for `.txt`, `.md`, and `.markdown` files.
- Open a book directly from the command line or from the library screen.
- Decode UTF-8, GB18030/GBK-compatible text, and Big5 fallback.
- Detect common Chinese and English chapter headings.
- Navigate with Vim-style keys, visual-row-aware page keys, and an interactive chapter list.
- Wrap CJK text by terminal cell width and keep common opening/closing punctuation with adjacent text.
- Persist reading position and bookmarks in SQLite.
- Search the currently opened text for a literal phrase.

EPUB, full-text indexing, annotations, statistics, configurable typography, and richer library management are planned, but are not implemented yet.

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

| Key | Action |
|---|---|
| `j` / `k`, `↑` / `↓` | Scroll one visual row |
| `Space` / `PageDown` | Move forward one screen |
| `PageUp` | Move backward one screen |
| `g` / `Home` | Go to the beginning |
| `G` / `End` | Go to the end |
| `n` / `N` | Next / previous detected chapter |
| `t` | Open the table of contents; use `j` / `k` and `Enter` to jump |
| `/` | Search the current book |
| `b` | Add a bookmark at the current position |
| `q` / `Esc` | Return to library or quit |
| `?` | Show key hints |

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
