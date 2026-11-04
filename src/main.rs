mod document;
mod layout;

use anyhow::{Context, Result};
use clap::Parser;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Margin},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
    Terminal,
};
use rusqlite::{params, Connection, OptionalExtension};
use regex::Regex;
use std::{
    fs,
    io::{self, stdout},
    path::{Path, PathBuf},
    time::Duration,
};
use walkdir::WalkDir;
use document::{Chapter, Document};

#[derive(Parser, Debug)]
#[command(name = "readtui", version, about = "终端小说阅读器与本地书库管理工具")]
struct Args {
    /// 直接打开一本小说
    file: Option<PathBuf>,
    /// 指定书库目录（默认 ~/Books）
    #[arg(short, long)]
    library: Option<PathBuf>,
}

#[derive(Clone)]
struct Book {
    path: PathBuf,
    title: String,
}

#[derive(Clone)]
struct Bookmark {
    id: i64,
    line: usize,
    byte_offset: u64,
    wrap_offset: usize,
    note: String,
}

struct Reader {
    book: Book,
    document: Document,
    chapters: Vec<Chapter>,
    top: usize,
    wrap_offset: usize,
}

impl Reader {
    fn line_count(&self) -> usize { self.document.line_count() }

    fn advance(&mut self, width: usize, rows: usize) -> Result<()> {
        let mut remaining = rows.max(1);
        while remaining > 0 && self.top < self.line_count() {
            let text = self.document.read_line(self.top)?;
            let count = layout::wrap_line(&text, width).len();
            let available = count.saturating_sub(self.wrap_offset);
            if remaining < available {
                self.wrap_offset += remaining;
                return Ok(());
            }
            remaining -= available;
            if self.top + 1 >= self.line_count() {
                self.wrap_offset = count.saturating_sub(1);
                return Ok(());
            }
            self.top += 1;
            self.wrap_offset = 0;
        }
        Ok(())
    }

    fn retreat(&mut self, width: usize, rows: usize) -> Result<()> {
        let mut remaining = rows.max(1);
        while remaining > 0 {
            if self.wrap_offset > 0 {
                let step = remaining.min(self.wrap_offset);
                self.wrap_offset -= step;
                remaining -= step;
            } else if self.top > 0 {
                self.top -= 1;
                let text = self.document.read_line(self.top)?;
                self.wrap_offset = layout::wrap_line(&text, width).len().saturating_sub(1);
                remaining -= 1;
            } else { break; }
        }
        Ok(())
    }

    fn jump_to_line(&mut self, line: usize) {
        self.top = line.min(self.line_count().saturating_sub(1));
        self.wrap_offset = 0;
    }
}

#[derive(PartialEq)]
enum Screen {
    Library,
    Reader,
}

struct App {
    screen: Screen,
    books: Vec<Book>,
    selected: usize,
    list_state: ListState,
    reader: Option<Reader>,
    library_dir: PathBuf,
    db: Connection,
    should_quit: bool,
    show_toc: bool,
    toc_selected: usize,
    show_bookmarks: bool,
    bookmarks: Vec<Bookmark>,
    bookmark_selected: usize,
    annotation_mode: bool,
    input: Option<String>,
    status: String,
    viewport_width: usize,
    viewport_height: usize,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let home = dirs::home_dir().context("无法确定用户主目录")?;
    let library_dir = args.library.unwrap_or_else(|| home.join("Books"));
    fs::create_dir_all(&library_dir)?;

    let data_dir = dirs::config_dir()
        .unwrap_or_else(|| home.join(".config"))
        .join("readtui");
    fs::create_dir_all(&data_dir)?;
    let db = Connection::open(data_dir.join("readtui.db"))?;
    init_db(&db)?;

    let books = scan_books(&library_dir);
    let mut app = App {
        screen: Screen::Library,
        books,
        selected: 0,
        list_state: ListState::default(),
        reader: None,
        library_dir,
        db,
        should_quit: false,
        show_toc: false,
        toc_selected: 0,
        show_bookmarks: false,
        bookmarks: Vec::new(),
        bookmark_selected: 0,
        annotation_mode: false,
        input: None,
        status: "按 Enter 阅读，a 添加文件，? 查看快捷键".into(),
        viewport_width: 80,
        viewport_height: 20,
    };

    app.list_state.select(if app.books.is_empty() { None } else { Some(0) });

    if let Some(file) = args.file {
        let path = file.canonicalize().unwrap_or(file);
        app.open_book(path)?;
    }

    let mut terminal = setup_terminal()?;
    let result = run_app(&mut terminal, &mut app);
    restore_terminal(&mut terminal)?;
    result
}

fn has_column(db: &Connection, table: &str, column: &str) -> Result<bool> {
    let mut statement = db.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = statement.query_map([], |row| row.get::<_, String>(1))?;
    for name in columns { if name? == column { return Ok(true); } }
    Ok(false)
}

fn add_column_if_missing(db: &Connection, table: &str, column: &str, definition: &str) -> Result<()> {
    if !has_column(db, table, column)? {
        db.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"))?;
    }
    Ok(())
}

fn init_db(db: &Connection) -> Result<()> {
    db.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA foreign_keys=ON;
         CREATE TABLE IF NOT EXISTS reading_progress (
           path TEXT PRIMARY KEY, line INTEGER NOT NULL DEFAULT 0,
           updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
         );
         CREATE TABLE IF NOT EXISTS bookmarks (
           id INTEGER PRIMARY KEY AUTOINCREMENT, path TEXT NOT NULL,
           line INTEGER NOT NULL, note TEXT NOT NULL DEFAULT '',
           created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
         );
         CREATE TABLE IF NOT EXISTS reading_sessions (
           id INTEGER PRIMARY KEY AUTOINCREMENT, path TEXT NOT NULL,
           started_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
           ended_at TEXT, duration_seconds INTEGER NOT NULL DEFAULT 0
         );
         CREATE TABLE IF NOT EXISTS annotations (
           id INTEGER PRIMARY KEY AUTOINCREMENT, path TEXT NOT NULL,
           line INTEGER NOT NULL, byte_offset INTEGER NOT NULL DEFAULT 0,
           quote TEXT NOT NULL DEFAULT '', note TEXT NOT NULL DEFAULT '',
           color TEXT NOT NULL DEFAULT 'yellow',
           created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
         );
         CREATE TABLE IF NOT EXISTS book_metadata (
           path TEXT PRIMARY KEY, title TEXT NOT NULL DEFAULT '',
           author TEXT NOT NULL DEFAULT '', description TEXT NOT NULL DEFAULT '',
           favorite INTEGER NOT NULL DEFAULT 0,
           status TEXT NOT NULL DEFAULT 'unread',
           updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
         );
         CREATE TABLE IF NOT EXISTS tags (
           id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL UNIQUE
         );
         CREATE TABLE IF NOT EXISTS book_tags (
           path TEXT NOT NULL, tag_id INTEGER NOT NULL,
           PRIMARY KEY(path, tag_id),
           FOREIGN KEY(tag_id) REFERENCES tags(id) ON DELETE CASCADE
         );
         CREATE INDEX IF NOT EXISTS idx_bookmarks_path ON bookmarks(path);
         CREATE INDEX IF NOT EXISTS idx_annotations_path ON annotations(path);
         CREATE INDEX IF NOT EXISTS idx_sessions_path ON reading_sessions(path);",
    )?;
    // Forward-compatible migration from the original line-only schema.
    for (column, definition) in [
        ("byte_offset", "INTEGER NOT NULL DEFAULT 0"),
        ("wrap_offset", "INTEGER NOT NULL DEFAULT 0"),
    ] {
        add_column_if_missing(db, "reading_progress", column, definition)?;
        add_column_if_missing(db, "bookmarks", column, definition)?;
    }
    Ok(())
}

fn scan_books(root: &Path) -> Vec<Book> {
    let mut books = Vec::new();
    for entry in WalkDir::new(root).follow_links(false).into_iter().filter_map(|entry| entry.ok()) {
        if !entry.file_type().is_file() { continue; }
        let path = entry.path();
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("").to_ascii_lowercase();
        if ext == "txt" || ext == "md" || ext == "markdown" {
            let title = path.file_stem().and_then(|s| s.to_str()).unwrap_or("未命名").to_string();
            books.push(Book { path: path.to_path_buf(), title });
        }
    }
    books.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
    books
}

fn decode_text(bytes: &[u8]) -> String {
    if let Some(s) = std::str::from_utf8(bytes).ok() { return s.trim_start_matches('\u{feff}').to_string(); }
    let (decoded, _, had_errors) = encoding_rs::GB18030.decode(bytes);
    if !had_errors { return decoded.into_owned(); }
    let (decoded, _, _) = encoding_rs::BIG5.decode(bytes);
    decoded.into_owned()
}

fn detect_chapters(lines: &[String]) -> Vec<Chapter> {
    let patterns = [
        r"^\s*第[零〇一二三四五六七八九十百千万两0-9０-９]+[章节回卷集部篇].*",
        r"^\s*(序章|楔子|引子|序言|后记|尾声|终章)\s*$",
        r"^\s*Chapter\s+[0-9IVXivx]+.*",
        r"^\s*卷[一二三四五六七八九十0-9]+.*",
    ];
    let regexes: Vec<Regex> = patterns.iter().filter_map(|p| Regex::new(p).ok()).collect();
    let mut chapters = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if !trimmed.is_empty() && regexes.iter().any(|r| r.is_match(trimmed)) {
            chapters.push(Chapter { title: trimmed.to_string(), line: idx });
        }
    }
    if chapters.is_empty() { chapters.push(Chapter { title: "正文".into(), line: 0 }); }
    chapters
}

impl App {
    fn open_book(&mut self, path: PathBuf) -> Result<()> {
        let mut document = Document::open(&path)?;
        let chapters = document.detect_chapters()?;
        let saved: Option<(i64, i64, i64)> = self.db.query_row(
            "SELECT line, byte_offset, wrap_offset FROM reading_progress WHERE path=?1",
            [path.to_string_lossy().as_ref()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).optional()?;
        let (line, byte_offset, wrap_offset) = saved.unwrap_or((0, 0, 0));
        let top = if byte_offset > 0 {
            document.line_for_offset(byte_offset as u64)
        } else {
            (line.max(0) as usize).min(document.line_count().saturating_sub(1))
        };
        let title = path.file_stem().and_then(|s| s.to_str()).unwrap_or("未命名").to_string();
        let book = Book { path: path.clone(), title };
        self.db.execute("INSERT INTO reading_sessions(path) VALUES (?1)", [path.to_string_lossy().as_ref()])?;
        self.reader = Some(Reader { book, document, chapters, top, wrap_offset: wrap_offset.max(0) as usize });
        self.screen = Screen::Reader;
        self.show_toc = false;
        self.show_bookmarks = false;
        Ok(())
    }

    fn save_progress(&self) -> Result<()> {
        if let Some(reader) = &self.reader {
            let offset = reader.document.offset_for_line(reader.top);
            self.db.execute(
                "INSERT INTO reading_progress(path,line,byte_offset,wrap_offset,updated_at)
                 VALUES (?1,?2,?3,?4,CURRENT_TIMESTAMP)
                 ON CONFLICT(path) DO UPDATE SET
                   line=excluded.line, byte_offset=excluded.byte_offset,
                   wrap_offset=excluded.wrap_offset, updated_at=CURRENT_TIMESTAMP",
                params![
                    reader.book.path.to_string_lossy().as_ref(),
                    reader.top as i64, offset as i64, reader.wrap_offset as i64
                ],
            )?;
        }
        Ok(())
    }

    fn add_bookmark(&mut self) -> Result<()> {
        if let Some(reader) = &self.reader {
            let offset = reader.document.offset_for_line(reader.top);
            self.db.execute(
                "INSERT INTO bookmarks(path,line,byte_offset,wrap_offset) VALUES (?1,?2,?3,?4)",
                params![reader.book.path.to_string_lossy().as_ref(), reader.top as i64, offset as i64, reader.wrap_offset as i64],
            )?;
            self.status = "已添加书签".into();
        }
        Ok(())
    }

    fn load_bookmarks(&mut self) -> Result<()> {
        self.bookmarks.clear();
        if let Some(reader) = &self.reader {
            let mut statement = self.db.prepare(
                "SELECT id,line,byte_offset,wrap_offset,note FROM bookmarks WHERE path=?1 ORDER BY created_at DESC,id DESC"
            )?;
            let rows = statement.query_map([reader.book.path.to_string_lossy().as_ref()], |row| {
                Ok(Bookmark {
                    id: row.get(0)?,
                    line: row.get::<_, i64>(1)?.max(0) as usize,
                    byte_offset: row.get::<_, i64>(2)?.max(0) as u64,
                    wrap_offset: row.get::<_, i64>(3)?.max(0) as usize,
                    note: row.get(4)?,
                })
            })?;
            for row in rows { self.bookmarks.push(row?); }
        }
        self.bookmark_selected = 0;
        self.show_bookmarks = true;
        Ok(())
    }

    fn add_annotation(&mut self, note: &str) -> Result<()> {
        if let Some(reader) = &mut self.reader {
            let quote = reader.document.read_line(reader.top)?;
            let offset = reader.document.offset_for_line(reader.top);
            self.db.execute(
                "INSERT INTO annotations(path,line,byte_offset,quote,note) VALUES (?1,?2,?3,?4,?5)",
                params![reader.book.path.to_string_lossy().as_ref(), reader.top as i64, offset as i64, quote, note],
            )?;
            self.status = "笔记已保存".into();
        }
        Ok(())
    }

    fn delete_selected_bookmark(&mut self) -> Result<()> {
        if let Some(bookmark) = self.bookmarks.get(self.bookmark_selected) {
            self.db.execute("DELETE FROM bookmarks WHERE id=?1", [bookmark.id])?;
            self.bookmarks.remove(self.bookmark_selected);
            self.bookmark_selected = self.bookmark_selected.min(self.bookmarks.len().saturating_sub(1));
            self.status = "书签已删除".into();
        }
        Ok(())
    }
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen)?;
    Terminal::new(CrosstermBackend::new(out)).map_err(Into::into)
}

fn restore_terminal<B: ratatui::backend::Backend>(terminal: &mut Terminal<B>) -> Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, app: &mut App) -> Result<()> {
    while !app.should_quit {
        terminal.draw(|frame| draw(frame, app))?;
        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? { handle_key(app, key)?; app.save_progress()?; }
        }
    }
    app.save_progress()?;
    Ok(())
}

fn draw(frame: &mut ratatui::Frame, app: &mut App) {
    let area = frame.area();
    let chunks = Layout::default().direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(5), Constraint::Length(2)]).split(area);
    let title = Paragraph::new(Line::from(vec![
        Span::styled(" READTUI ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw("本地小说阅读器"),
    ])).block(Block::default().borders(Borders::ALL));
    frame.render_widget(title, chunks[0]);

    if app.screen == Screen::Library {
        let items: Vec<ListItem> = app.books.iter().map(|book| {
            let path = book.path.strip_prefix(&app.library_dir).unwrap_or(&book.path);
            ListItem::new(format!("{}   {}", book.title, path.display()))
        }).collect();
        let list = List::new(items).block(Block::default().title(format!("书库 · {} 本", app.books.len())).borders(Borders::ALL))
            .highlight_style(Style::default().bg(Color::DarkGray).fg(Color::White).add_modifier(Modifier::BOLD))
            .highlight_symbol("▶ ");
        frame.render_stateful_widget(list, chunks[1], &mut app.list_state);
    } else if let Some(reader) = &mut app.reader {
        let body = Layout::default().direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(100)]).split(chunks[1])[0].inner(Margin { vertical: 1, horizontal: 3 });
        let height = body.height.max(1) as usize;
        let text_width = body.width.max(1) as usize;
        app.viewport_width = text_width;
        app.viewport_height = height;
        let line_count = reader.line_count();
        if let Ok(current_line) = reader.document.read_line(reader.top) {
            let wrapped_rows = layout::wrap_line(&current_line, text_width).len();
            reader.wrap_offset = reader.wrap_offset.min(wrapped_rows.saturating_sub(1));
        }
        let mut visible = Vec::with_capacity(height);
        for index in reader.top..line_count {
            let text = reader.document.read_line(index).unwrap_or_else(|_| "[读取失败]".into());
            let wrapped = layout::wrap_line(&text, text_width);
            let rows = if index == reader.top { wrapped.into_iter().skip(reader.wrap_offset).collect::<Vec<_>>() } else { wrapped };
            for row in rows {
                visible.push(Line::from(Span::raw(row)));
                if visible.len() >= height { break; }
            }
            if visible.len() >= height { break; }
        }
        let content = Paragraph::new(Text::from(visible)).wrap(Wrap { trim: false });
        frame.render_widget(content, body);
        let progress = if reader.document.byte_len == 0 { 0 } else {
            (reader.document.offset_for_line(reader.top).saturating_mul(100) / reader.document.byte_len) as usize
        };
        let chapter = reader.chapters.iter().rev().find(|c| c.line <= reader.top).map(|c| c.title.as_str()).unwrap_or("正文");
        let footer_text = format!("{}  ·  {}  ·  {}%  ·  第 {}/{} 行  ·  {}", reader.book.title, chapter, progress, reader.top + 1, line_count, app.status);
        let footer = Paragraph::new(footer_text).style(Style::default().fg(Color::Gray));
        frame.render_widget(footer, chunks[2]);
        if app.show_bookmarks {
            let popup = centered_rect(75, 70, area);
            frame.render_widget(Clear, popup);
            let entries: Vec<ListItem> = app.bookmarks.iter().map(|b| {
                let note = if b.note.is_empty() { String::new() } else { format!(" · {}", b.note) };
                ListItem::new(format!("第 {:>7} 行{}", b.line + 1, note))
            }).collect();
            let list = List::new(entries)
                .block(Block::default().title("书签 · Enter 跳转 · d 删除 · Esc 关闭").borders(Borders::ALL))
                .highlight_style(Style::default().bg(Color::DarkGray).fg(Color::White).add_modifier(Modifier::BOLD))
                .highlight_symbol("▶ ");
            let mut state = ListState::default();
            state.select(if app.bookmarks.is_empty() { None } else { Some(app.bookmark_selected.min(app.bookmarks.len()-1)) });
            frame.render_stateful_widget(list, popup, &mut state);
        }
        if app.show_toc {
            let popup = centered_rect(70, 70, area);
            frame.render_widget(Clear, popup);
            let entries: Vec<ListItem> = reader.chapters.iter().map(|c| ListItem::new(format!("{:>7}  {}", c.line + 1, c.title))).collect();
            let toc = List::new(entries)
                .block(Block::default().title("章节目录 · Enter 跳转 · Esc 关闭").borders(Borders::ALL))
                .highlight_style(Style::default().bg(Color::DarkGray).fg(Color::White).add_modifier(Modifier::BOLD))
                .highlight_symbol("▶ ");
            let mut toc_state = ListState::default();
            toc_state.select(if reader.chapters.is_empty() { None } else { Some(app.toc_selected.min(reader.chapters.len() - 1)) });
            frame.render_stateful_widget(toc, popup, &mut toc_state);
        }
    }

    if app.screen == Screen::Library {
        let footer = Paragraph::new(app.status.clone()).block(Block::default().borders(Borders::TOP))
            .style(Style::default().fg(Color::Yellow));
        frame.render_widget(footer, chunks[2]);
    }
    if let Some(input) = &app.input {
        let popup = centered_rect(60, 20, area);
        frame.render_widget(Clear, popup);
        let title = if app.annotation_mode { "为当前段落添加笔记" } else if app.screen == Screen::Reader { "搜索当前书籍" } else { "输入文件路径" };
        frame.render_widget(Paragraph::new(input.as_str()).block(Block::default().title(title).borders(Borders::ALL)), popup);
    }
}

fn wrap_cjk_line(line: &str, max_width: usize) -> Vec<String> {
    layout::wrap_line(line, max_width)
}

fn centered_rect(width: u16, height: u16, area: ratatui::layout::Rect) -> ratatui::layout::Rect {
    let vert = Layout::default().direction(Direction::Vertical).constraints([Constraint::Percentage((100-height)/2), Constraint::Percentage(height), Constraint::Percentage((100-height)/2)]).split(area);
    Layout::default().direction(Direction::Horizontal).constraints([Constraint::Percentage((100-width)/2), Constraint::Percentage(width), Constraint::Percentage((100-width)/2)]).split(vert[1])[1]
}

fn handle_key(app: &mut App, key: KeyEvent) -> Result<()> {
    if let Some(input) = &mut app.input {
        match key.code {
            KeyCode::Esc => app.input = None,
            KeyCode::Enter => {
                let value = input.clone(); app.input = None;
                if app.annotation_mode {
                    app.annotation_mode = false;
                    if !value.trim().is_empty() { app.add_annotation(value.trim())?; }
                } else if app.screen == Screen::Library {
                    let path = PathBuf::from(value);
                    if path.is_file() { app.open_book(path)?; }
                    else { app.status = "文件不存在，请输入有效路径".into(); }
                } else if let Some(reader) = &mut app.reader {
                    if let Some(pos) = reader.document.search(&value, reader.top.saturating_add(1), false)? {
                        reader.jump_to_line(pos);
                        app.status = format!("找到匹配内容：第 {} 行", pos + 1);
                    } else {
                        app.status = "没有找到匹配内容".into();
                    }
                }
            }
            KeyCode::Backspace => { input.pop(); }
            KeyCode::Char(c) => input.push(c),
            _ => {}
        }
        return Ok(());
    }
    if app.screen == Screen::Library {
        match key.code {
            KeyCode::Char('q') => app.should_quit = true,
            KeyCode::Down | KeyCode::Char('j') => { if !app.books.is_empty() { app.selected = (app.selected + 1).min(app.books.len()-1); } },
            KeyCode::Up | KeyCode::Char('k') => { app.selected = app.selected.saturating_sub(1); },
            KeyCode::Enter => { if let Some(book) = app.books.get(app.selected) { app.open_book(book.path.clone())?; } },
            KeyCode::Char('a') => app.input = Some(String::new()),
            KeyCode::Char('?') => app.status = "↑/k 上移  ↓/j 下移  Enter 阅读  a 添加路径  q 退出".into(),
            _ => {}
        }
        app.list_state.select(if app.books.is_empty() { None } else { Some(app.selected) });
    } else {
        if app.show_bookmarks {
            match key.code {
                KeyCode::Esc | KeyCode::Char('B') => app.show_bookmarks = false,
                KeyCode::Down | KeyCode::Char('j') => {
                    if !app.bookmarks.is_empty() { app.bookmark_selected = (app.bookmark_selected + 1).min(app.bookmarks.len()-1); }
                }
                KeyCode::Up | KeyCode::Char('k') => app.bookmark_selected = app.bookmark_selected.saturating_sub(1),
                KeyCode::Char('d') => app.delete_selected_bookmark()?,
                KeyCode::Enter => {
                    if let Some(bookmark) = app.bookmarks.get(app.bookmark_selected) {
                        if let Some(reader) = &mut app.reader {
                            reader.jump_to_line(if bookmark.byte_offset > 0 { reader.document.line_for_offset(bookmark.byte_offset) } else { bookmark.line });
                            reader.wrap_offset = bookmark.wrap_offset;
                        }
                    }
                    app.show_bookmarks = false;
                }
                _ => {}
            }
            return Ok(());
        }
        if app.show_toc {
            let chapter_count = app.reader.as_ref().map(|r| r.chapters.len()).unwrap_or(0);
            match key.code {
                KeyCode::Esc | KeyCode::Char('t') => app.show_toc = false,
                KeyCode::Down | KeyCode::Char('j') => {
                    if chapter_count > 0 { app.toc_selected = (app.toc_selected + 1).min(chapter_count - 1); }
                }
                KeyCode::Up | KeyCode::Char('k') => app.toc_selected = app.toc_selected.saturating_sub(1),
                KeyCode::Enter => {
                    if let Some(reader) = &mut app.reader {
                        if let Some(chapter) = reader.chapters.get(app.toc_selected) {
                            reader.top = chapter.line; reader.wrap_offset = 0;
                            app.status = format!("已跳转到：{}", chapter.title);
                        }
                    }
                    app.show_toc = false;
                }
                KeyCode::Home => app.toc_selected = 0,
                KeyCode::End => app.toc_selected = chapter_count.saturating_sub(1),
                _ => {}
            }
            return Ok(());
        }
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => { app.save_progress()?; app.screen = Screen::Library; app.reader = None; },
            KeyCode::Down | KeyCode::Char('j') => if let Some(r) = &mut app.reader {
                r.advance(app.viewport_width, 1)?;
            },
            KeyCode::Up | KeyCode::Char('k') => if let Some(r) = &mut app.reader {
                r.retreat(app.viewport_width, 1)?;
            },
            KeyCode::PageDown | KeyCode::Char(' ') => if let Some(r) = &mut app.reader {
                r.advance(app.viewport_width, app.viewport_height)?;
            },
            KeyCode::PageUp => if let Some(r) = &mut app.reader {
                r.retreat(app.viewport_width, app.viewport_height)?;
            },
            KeyCode::Home | KeyCode::Char('g') => if let Some(r) = &mut app.reader { r.top = 0; r.wrap_offset = 0; },
            KeyCode::End | KeyCode::Char('G') => if let Some(r) = &mut app.reader { r.jump_to_line(r.line_count().saturating_sub(1)); },
            KeyCode::Char('t') => {
                if let Some(reader) = &app.reader {
                    app.toc_selected = reader.chapters.iter().rposition(|chapter| chapter.line <= reader.top).unwrap_or(0);
                }
                app.show_toc = true;
            },
            KeyCode::Char('b') => app.add_bookmark()?,
            KeyCode::Char('B') => app.load_bookmarks()?,
            KeyCode::Char('m') => { app.annotation_mode = true; app.input = Some(String::new()); },
            KeyCode::Char('/') => app.input = Some(String::new()),
            KeyCode::Char('n') => if let Some(r) = &mut app.reader {
                if let Some(next) = r.chapters.iter().find(|c| c.line > r.top) { r.top = next.line; r.wrap_offset = 0; }
            },
            KeyCode::Char('N') => if let Some(r) = &mut app.reader {
                if let Some(prev) = r.chapters.iter().rev().find(|c| c.line < r.top) { r.top = prev.line; r.wrap_offset = 0; }
            },
            KeyCode::Char('?') => app.status = "j/k 滚动  Space/PageDown 下一页  PageUp 上一页  t 目录  n/N 章节  b 添加书签  B 书签列表  m 笔记  / 搜索  q 返回".into(),
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_chinese_chapters() {
        let lines = vec!["序章".to_string(), "正文".to_string(), "第一百零二章 风起".to_string()];
        let chapters = detect_chapters(&lines);
        assert_eq!(chapters.len(), 2);
        assert_eq!(chapters[0].title, "序章");
        assert_eq!(chapters[1].line, 2);
    }

    #[test]
    fn decodes_utf8_text() {
        assert_eq!(decode_text("中文小说".as_bytes()), "中文小说");
    }
}
