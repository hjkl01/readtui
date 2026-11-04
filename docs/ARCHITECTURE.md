# Architecture

## Layers

```text
Terminal UI (Ratatui + Crossterm)
        |
Application state and commands
        |
Reader engine ---- Library services ---- Search/statistics services
        |                    |
Format adapters       SQLite repositories
        |
Filesystem
```

## Module boundaries

- `ui`: rendering and terminal interaction only.
- `reader`: document navigation, layout, pagination, and progress locations.
- `formats`: format-specific parsing into a common document model.
- `library`: import, scan, metadata, collections, and file identity.
- `services`: bookmarks, annotations, search, history, and statistics.
- `storage`: SQLite schema and repositories.
- `config`: user preferences and key bindings.

The current prototype is intentionally compact in `src/main.rs`; modules should be extracted as each domain becomes stable. Do not move SQL, parsing, and rendering into a single long-lived application state type.

## Performance requirements

- Avoid loading very large plain-text files into memory in the production reader. Build a streaming/indexed text source with byte offsets.
- Do not rebuild the chapter index on every render.
- Persist progress on meaningful navigation events and on exit, with debouncing for high-frequency input.
- Keep expensive scanning and indexing off the render loop.
- Reflow visible content when terminal dimensions or typography settings change.

## Terminal compatibility

Crossterm is used as the cross-platform event and terminal backend. Layout must account for full-width CJK characters. Mouse input and terminal image protocols are optional enhancements; core reading must work in a basic UTF-8 terminal.
