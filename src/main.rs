mod document;
mod epub;
mod formats;
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
use std::{
    collections::HashMap,
    fs,
    hash::{Hash, Hasher},
    io::{self, stdout, Read},
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
    /// 将数据库备份到指定文件并退出
    #[arg(long, conflicts_with = "restore")]
    backup: Option<PathBuf>,
    /// 从数据库备份恢复并退出
    #[arg(long, conflicts_with = "backup")]
    restore: Option<PathBuf>,
    /// 输出书库中的重复文件分组并退出
    #[arg(long)]
    duplicates: bool,
    /// 清理数据库中已不存在的书籍记录并退出
    #[arg(long)]
    prune_missing: bool,
    /// 重新关联丢失文件，格式：旧路径=新路径
    #[arg(long)]
    relink: Option<String>,
}

#[derive(Clone)]
struct Book {
    path: PathBuf,
    title: String,
    author: String,
    favorite: bool,
    status: String,
    tags: Vec<String>,
}

#[derive(Clone)]
struct Bookmark {
    id: i64,
    line: usize,
    byte_offset: u64,
    wrap_offset: usize,
    char_offset: usize,
    note: String,
}

#[derive(Clone)]
struct SearchResult {
    path: PathBuf,
    title: String,
    line: usize,
    snippet: String,
}

#[derive(Clone)]
struct Annotation {
    id: i64,
    line: usize,
    byte_offset: u64,
    quote: String,
    note: String,
}

struct Reader {
    book: Book,
    document: Document,
    chapters: Vec<Chapter>,
    top: usize,
    wrap_offset: usize,
    char_offset: usize,
    session_id: i64,
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
    visible_books: Vec<usize>,
    library_filter: usize,
    tag_filter: Option<String>,
    tag_filter_mode: bool,
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
    show_annotations: bool,
    annotations: Vec<Annotation>,
    annotation_selected: usize,
    annotation_mode: bool,
    annotation_edit_mode: bool,
    export_mode: bool,
    directory_import_mode: bool,
    tag_mode: bool,
    metadata_mode: bool,
    goal_mode: bool,
    library_search_mode: bool,
    show_search_results: bool,
    search_results: Vec<SearchResult>,
    search_selected: usize,
    statistics: Option<String>,
    theme_light: bool,
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
    if args.duplicates {
        print_duplicate_books(&library_dir)?;
        return Ok(());
    }

    let data_dir = dirs::config_dir()
        .unwrap_or_else(|| home.join(".config"))
        .join("readtui");
    fs::create_dir_all(&data_dir)?;
    let database_path = data_dir.join("readtui.db");
    if let Some(source) = args.restore {
        if !source.is_file() { anyhow::bail!("备份文件不存在：{}", source.display()); }
        let backup = Connection::open(&source)
            .with_context(|| format!("无法打开备份文件：{}", source.display()))?;
        let integrity: String = backup.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
        if integrity != "ok" { anyhow::bail!("备份数据库完整性检查失败：{integrity}"); }
        drop(backup);
        let temporary = database_path.with_extension("restore.tmp");
        fs::copy(&source, &temporary)?;
        fs::rename(&temporary, &database_path)
            .with_context(|| "无法替换数据库；请先关闭其他 readtui 实例")?;
        println!("数据库已从 {} 恢复", source.display());
        return Ok(());
    }
    let db = Connection::open(&database_path)?;
    init_db(&db)?;
    if let Some(target) = args.backup {
        if target.exists() { anyhow::bail!("备份目标已存在：{}", target.display()); }
        if let Some(parent) = target.parent() {
            if !parent.as_os_str().is_empty() { fs::create_dir_all(parent)?; }
        }
        db.execute("VACUUM INTO ?1", [target.to_string_lossy().as_ref()])
            .with_context(|| format!("备份数据库失败：{}", target.display()))?;
        println!("数据库已备份到 {}", target.display());
        return Ok(());
    }

    if args.prune_missing {
        let removed = prune_missing_books(&db)?;
        println!("已清理 {removed} 条丢失书籍记录。");
        return Ok(());
    }
    if let Some(mapping) = args.relink {
        let (old, new) = mapping.split_once('=')
            .ok_or_else(|| anyhow::anyhow!("--relink 格式应为：旧路径=新路径"))?;
        relink_book_path(&db, Path::new(old), Path::new(new))?;
        return Ok(());
    }

    let mut books = scan_books(&library_dir);
    load_registered_books(&db, &mut books)?;
    load_book_metadata(&db, &mut books)?;
    let theme_light = load_setting(&db, "theme").unwrap_or_else(|_| "dark".into()) == "light";
    let mut app = App {
        screen: Screen::Library,
        visible_books: (0..books.len()).collect(),
        books,
        library_filter: 0,
        tag_filter: None,
        tag_filter_mode: false,
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
        show_annotations: false,
        annotations: Vec::new(),
        annotation_selected: 0,
        annotation_mode: false,
        annotation_edit_mode: false,
        export_mode: false,
        directory_import_mode: false,
        tag_mode: false,
        metadata_mode: false,
        goal_mode: false,
        library_search_mode: false,
        show_search_results: false,
        search_results: Vec::new(),
        search_selected: 0,
        statistics: None,
        theme_light,
        input: None,
        status: "按 Enter 阅读，a 添加文件，? 查看快捷键".into(),
        viewport_width: 0,
        viewport_height: 0,
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


fn content_fingerprint(path: &Path) -> Result<(u64, u64)> {
    let mut file = fs::File::open(path)?;
    let size = file.metadata()?.len();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 { break; }
        buffer[..count].hash(&mut hasher);
    }
    Ok((size, hasher.finish()))
}

fn print_duplicate_books(root: &Path) -> Result<()> {
    let mut groups: HashMap<(u64, u64), Vec<PathBuf>> = HashMap::new();
    for entry in WalkDir::new(root).follow_links(false).into_iter().filter_map(|entry| entry.ok()) {
        if !entry.file_type().is_file() { continue; }
        let path = entry.path();
        let ext = path.extension().and_then(|value| value.to_str()).unwrap_or("").to_ascii_lowercase();
        if !matches!(ext.as_str(), "txt" | "md" | "markdown" | "epub" | "html" | "htm" | "fb2") { continue; }
        let fingerprint = content_fingerprint(path)?;
        groups.entry(fingerprint).or_default().push(path.to_path_buf());
    }
    let duplicates: Vec<_> = groups.into_values().filter(|paths| paths.len() > 1).collect();
    if duplicates.is_empty() { println!("没有发现重复书籍。"); }
    for (index, paths) in duplicates.iter().enumerate() {
        println!("重复组 {}（{} 个文件）", index + 1, paths.len());
        for path in paths { println!("  {}", path.display()); }
    }
    println!("共发现 {} 组重复文件。", duplicates.len());
    Ok(())
}

fn prune_missing_books(db: &Connection) -> Result<usize> {
    let tx = db.unchecked_transaction()?;
    let mut statement = tx.prepare("SELECT path FROM library_books")?;
    let paths = statement.query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(statement);
    let missing: Vec<String> = paths.into_iter().filter(|path| !Path::new(path).is_file()).collect();
    for path in &missing {
        for table in ["reading_progress", "bookmarks", "annotations", "book_metadata", "book_tags", "library_books"] {
            tx.execute(&format!("DELETE FROM {table} WHERE path=?1"), [path])?;
        }
    }
    tx.execute("DELETE FROM tags WHERE id NOT IN (SELECT DISTINCT tag_id FROM book_tags)", [])?;
    tx.commit()?;
    Ok(missing.len())
}

fn relink_book_path(db: &Connection, old_path: &Path, new_path: &Path) -> Result<()> {
    if !new_path.is_file() { anyhow::bail!("新文件不存在：{}", new_path.display()); }
    let old = old_path.to_string_lossy();
    let new = new_path.canonicalize().unwrap_or_else(|_| new_path.to_path_buf()).to_string_lossy().to_string();
    let tx = db.unchecked_transaction()?;
    let exists: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM library_books WHERE path=?1)", [&old], |row| row.get(0))?;
    if !exists { anyhow::bail!("数据库中没有旧路径记录：{old}"); }
    let collision: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM library_books WHERE path=?1)", [&new], |row| row.get(0))?;
    if collision { anyhow::bail!("新路径已经在书库中：{new}"); }
    for table in ["reading_progress", "bookmarks", "annotations", "book_metadata", "book_tags", "library_books"] {
        tx.execute(&format!("UPDATE {table} SET path=?1 WHERE path=?2"), params![new, old.as_ref()])?;
    }
    tx.commit()?;
    println!("已将书籍路径重新关联到：{new}");
    Ok(())
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
         CREATE TABLE IF NOT EXISTS settings (
           key TEXT PRIMARY KEY, value TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS library_books (
           path TEXT PRIMARY KEY, added_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
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
        ("char_offset", "INTEGER NOT NULL DEFAULT 0"),
    ] {
        add_column_if_missing(db, "reading_progress", column, definition)?;
        add_column_if_missing(db, "bookmarks", column, definition)?;
    }
    // Recover abandoned sessions left open by a crash or forced shutdown.
    // Keep a 24-hour grace period so another active instance is not interrupted.
    db.execute(
        "UPDATE reading_sessions
         SET ended_at=CURRENT_TIMESTAMP,
             duration_seconds=MIN(MAX(CAST((julianday('now')-julianday(started_at))*86400 AS INTEGER),0),7200)
         WHERE ended_at IS NULL AND started_at < datetime('now','-1 day')",
        [],
    )?;
    Ok(())
}

fn reader_source_path(path: &Path) -> Result<PathBuf> {
    let cache_dir = dirs::cache_dir().unwrap_or_else(|| std::env::temp_dir()).join("readtui/formats");
    match path.extension().and_then(|s| s.to_str()).unwrap_or("").to_ascii_lowercase().as_str() {
        "epub" => epub::extract_to_cache(path, &cache_dir.join("epub")),
        "md" | "markdown" | "html" | "htm" | "fb2" => formats::to_cache(path, &cache_dir),
        _ => Ok(path.to_path_buf()),
    }
}

fn scan_books(root: &Path) -> Vec<Book> {
    let mut books = Vec::new();
    for entry in WalkDir::new(root).follow_links(false).into_iter().filter_map(|entry| entry.ok()) {
        if !entry.file_type().is_file() { continue; }
        let path = entry.path();
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("").to_ascii_lowercase();
        if matches!(ext.as_str(), "txt" | "md" | "markdown" | "epub" | "html" | "htm" | "fb2") {
            let title = path.file_stem().and_then(|s| s.to_str()).unwrap_or("未命名").to_string();
            books.push(Book { path: path.to_path_buf(), title, author: String::new(), favorite: false, status: "unread".into(), tags: Vec::new() });
        }
    }
    books.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
    books
}

fn load_registered_books(db: &Connection, books: &mut Vec<Book>) -> Result<()> {
    let mut statement = db.prepare("SELECT path FROM library_books ORDER BY added_at DESC")?;
    let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
    for row in rows {
        let path = PathBuf::from(row?);
        if !path.is_file() || books.iter().any(|book| book.path == path) { continue; }
        let title = path.file_stem().and_then(|value| value.to_str()).unwrap_or("未命名").to_string();
        books.push(Book { path, title, author: String::new(), favorite: false, status: "unread".into(), tags: Vec::new() });
    }
    books.sort_by(|a,b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
    Ok(())
}

fn load_setting(db: &Connection, key: &str) -> Result<String> {
    Ok(db.query_row("SELECT value FROM settings WHERE key=?1", [key], |row| row.get(0))
        .optional()?.unwrap_or_default())
}

fn load_book_metadata(db: &Connection, books: &mut [Book]) -> Result<()> {
    for book in books {
        let metadata: Option<(String, String, bool, String)> = db.query_row(
            "SELECT title,author,favorite,status FROM book_metadata WHERE path=?1",
            [book.path.to_string_lossy().as_ref()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get::<_, i64>(2)? != 0, row.get(3)?)),
        ).optional()?;
        if let Some((title, author, favorite, status)) = metadata {
            if !title.trim().is_empty() { book.title = title; }
            book.author = author;
            book.favorite = favorite;
            book.status = status;
        }
        let mut statement = db.prepare(
            "SELECT t.name FROM tags t JOIN book_tags bt ON bt.tag_id=t.id WHERE bt.path=?1 ORDER BY t.name"
        )?;
        let rows = statement.query_map([book.path.to_string_lossy().as_ref()], |row| row.get::<_, String>(0))?;
        book.tags = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    }
    Ok(())
}

impl App {
    fn selected_book_index(&self) -> Option<usize> {
        self.visible_books.get(self.selected).copied()
    }

    fn refresh_visible_books(&mut self) {
        self.visible_books = self.books.iter().enumerate().filter_map(|(index, book)| {
            let visible = match self.library_filter {
                1 => book.favorite,
                2 => book.status == "reading",
                3 => book.status == "unread",
                4 => book.status == "finished",
                _ => true,
            };
            let tag_matches = self.tag_filter.as_ref().is_none_or(|tag| book.tags.iter().any(|value| value.eq_ignore_ascii_case(tag)));
            if visible && tag_matches { Some(index) } else { None }
        }).collect();
        self.selected = self.selected.min(self.visible_books.len().saturating_sub(1));
        self.list_state.select(if self.visible_books.is_empty() { None } else { Some(self.selected) });
    }

    fn import_directory(&mut self, root: &Path) -> Result<usize> {
        if !root.is_dir() { self.status = "目录不存在".into(); return Ok(0); }
        let mut imported = 0usize;
        for entry in WalkDir::new(root).follow_links(false).into_iter().filter_map(|entry| entry.ok()) {
            if !entry.file_type().is_file() { continue; }
            let path = entry.path().canonicalize().unwrap_or_else(|_| entry.path().to_path_buf());
            let ext = path.extension().and_then(|value| value.to_str()).unwrap_or("").to_ascii_lowercase();
            if !matches!(ext.as_str(), "txt" | "md" | "markdown" | "epub" | "html" | "htm" | "fb2") { continue; }
            if self.books.iter().any(|book| book.path == path) { continue; }
            self.db.execute("INSERT OR IGNORE INTO library_books(path) VALUES (?1)", [path.to_string_lossy().as_ref()])?;
            let title = path.file_stem().and_then(|value| value.to_str()).unwrap_or("未命名").to_string();
            self.books.push(Book { path, title, author: String::new(), favorite: false, status: "unread".into(), tags: Vec::new() });
            imported += 1;
        }
        self.books.sort_by(|a,b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
        load_book_metadata(&self.db, &mut self.books)?;
        self.refresh_visible_books();
        self.status = format!("目录导入完成，新增 {imported} 本书");
        Ok(imported)
    }

    fn sort_by_recent(&mut self) -> Result<()> {
        let mut recent: Vec<(String, String)> = Vec::new();
        {
            let mut statement = self.db.prepare("SELECT path,updated_at FROM reading_progress")?;
            let rows = statement.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?;
            for row in rows { recent.push(row?); }
        }
        self.books.sort_by(|a,b| {
            let a_path = a.path.to_string_lossy();
            let b_path = b.path.to_string_lossy();
            let a_time = recent.iter().find(|entry| entry.0.as_str() == a_path.as_ref()).map(|entry| entry.1.as_str()).unwrap_or("");
            let b_time = recent.iter().find(|entry| entry.0.as_str() == b_path.as_ref()).map(|entry| entry.1.as_str()).unwrap_or("");
            b_time.cmp(a_time).then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
        });
        self.selected = 0;
        self.list_state.select(if self.books.is_empty() { None } else { Some(0) });
        self.status = "已按最近阅读时间排序".into();
        Ok(())
    }

    fn rescan_library(&mut self) -> Result<()> {
        let mut books = scan_books(&self.library_dir);
        load_registered_books(&self.db, &mut books)?;
        load_book_metadata(&self.db, &mut books)?;
        self.books = books;
        self.refresh_visible_books();
        self.status = format!("书库扫描完成，共 {} 本书", self.books.len());
        Ok(())
    }

    fn add_book_path(&mut self, path: PathBuf) -> Result<()> {
        let path = path.canonicalize().unwrap_or(path);
        if !path.is_file() {
            self.status = "文件不存在，请输入有效路径".into();
            return Ok(());
        }
        let ext = path.extension().and_then(|value| value.to_str()).unwrap_or("").to_ascii_lowercase();
        if !matches!(ext.as_str(), "txt" | "md" | "markdown" | "epub" | "html" | "htm" | "fb2") {
            self.status = format!("暂不支持该文件格式：.{ext}");
            return Ok(());
        }
        self.db.execute("INSERT OR IGNORE INTO library_books(path) VALUES (?1)", [path.to_string_lossy().as_ref()])?;
        if !self.books.iter().any(|book| book.path == path) {
            let title = path.file_stem().and_then(|value| value.to_str()).unwrap_or("未命名").to_string();
            self.books.push(Book { path: path.clone(), title, author: String::new(), favorite: false, status: "unread".into(), tags: Vec::new() });
            self.books.sort_by(|a,b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
            self.selected = self.books.iter().position(|book| book.path == path).unwrap_or(0);
        }
        self.open_book(path)
    }

    fn open_book(&mut self, path: PathBuf) -> Result<()> {
        let document_path = reader_source_path(&path)?;
        let mut document = Document::open(&document_path)?;
        let chapters = document.detect_chapters()?;
        let saved: Option<(i64, i64, i64, i64)> = self.db.query_row(
            "SELECT line, byte_offset, wrap_offset, char_offset FROM reading_progress WHERE path=?1",
            [path.to_string_lossy().as_ref()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        ).optional()?;
        let (line, byte_offset, wrap_offset, char_offset) = saved.unwrap_or((0, 0, 0, 0));
        let top = if byte_offset > 0 {
            document.line_for_offset(byte_offset as u64)
        } else {
            (line.max(0) as usize).min(document.line_count().saturating_sub(1))
        };
        let mut book = self.books.iter().find(|book| book.path == path).cloned().unwrap_or_else(|| {
            let title = path.file_stem().and_then(|s| s.to_str()).unwrap_or("未命名").to_string();
            Book { path: path.clone(), title, author: String::new(), favorite: false, status: "unread".into(), tags: Vec::new() }
        });
        if path.extension().and_then(|value| value.to_str()).is_some_and(|value| value.eq_ignore_ascii_case("epub"))
            && book.author.is_empty()
        {
            if let Ok((title, author)) = epub::read_metadata(&path) {
                if let Some(title) = title {
                    let filename_title = path.file_stem().and_then(|value| value.to_str()).unwrap_or("");
                    if book.title == filename_title { book.title = title; }
                }
                if let Some(author) = author { book.author = author; }
                if let Some(existing) = self.books.iter_mut().find(|existing| existing.path == path) {
                    existing.title = book.title.clone();
                    existing.author = book.author.clone();
                }
                self.db.execute(
                    "INSERT INTO book_metadata(path,title,author,favorite,status)
                     VALUES (?1,?2,?3,?4,?5)
                     ON CONFLICT(path) DO UPDATE SET title=excluded.title,author=excluded.author,updated_at=CURRENT_TIMESTAMP",
                    params![path.to_string_lossy().as_ref(), book.title, book.author, book.favorite as i64, book.status],
                )?;
            }
        }
        self.db.execute("INSERT INTO reading_sessions(path) VALUES (?1)", [path.to_string_lossy().as_ref()])?;
        let session_id = self.db.last_insert_rowid();
        self.reader = Some(Reader { book, document, chapters, top, wrap_offset: wrap_offset.max(0) as usize, char_offset: char_offset.max(0) as usize, session_id });
        self.screen = Screen::Reader;
        self.show_toc = false;
        self.show_bookmarks = false;
        Ok(())
    }

    fn save_progress(&mut self) -> Result<()> {
        if let Some(reader) = &mut self.reader {
            let offset = reader.document.offset_for_line(reader.top);
            if self.viewport_width > 0 {
                let text = reader.document.read_line(reader.top)?;
                reader.char_offset = char_offset_for_wrap_offset(&text, self.viewport_width, reader.wrap_offset);
            }
            self.db.execute(
                "INSERT INTO reading_progress(path,line,byte_offset,wrap_offset,char_offset,updated_at)
                 VALUES (?1,?2,?3,?4,?5,CURRENT_TIMESTAMP)
                 ON CONFLICT(path) DO UPDATE SET
                   line=excluded.line, byte_offset=excluded.byte_offset,
                   wrap_offset=excluded.wrap_offset, char_offset=excluded.char_offset,
                   updated_at=CURRENT_TIMESTAMP",
                params![
                    reader.book.path.to_string_lossy().as_ref(),
                    reader.top as i64, offset as i64, reader.wrap_offset as i64, reader.char_offset as i64
                ],
            )?;
        }
        Ok(())
    }

    fn add_bookmark(&mut self) -> Result<()> {
        if let Some(reader) = &mut self.reader {
            let offset = reader.document.offset_for_line(reader.top);
            let text = reader.document.read_line(reader.top)?;
            reader.char_offset = char_offset_for_wrap_offset(&text, self.viewport_width, reader.wrap_offset);
            self.db.execute(
                "INSERT INTO bookmarks(path,line,byte_offset,wrap_offset,char_offset) VALUES (?1,?2,?3,?4,?5)",
                params![
                    reader.book.path.to_string_lossy().as_ref(), reader.top as i64, offset as i64,
                    reader.wrap_offset as i64, reader.char_offset as i64
                ],
            )?;
            self.status = "已添加书签".into();
        }
        Ok(())
    }

    fn load_bookmarks(&mut self) -> Result<()> {
        self.bookmarks.clear();
        if let Some(reader) = &self.reader {
            let mut statement = self.db.prepare(
                "SELECT id,line,byte_offset,wrap_offset,char_offset,note FROM bookmarks WHERE path=?1 ORDER BY created_at DESC,id DESC"
            )?;
            let rows = statement.query_map([reader.book.path.to_string_lossy().as_ref()], |row| {
                Ok(Bookmark {
                    id: row.get(0)?,
                    line: row.get::<_, i64>(1)?.max(0) as usize,
                    byte_offset: row.get::<_, i64>(2)?.max(0) as u64,
                    wrap_offset: row.get::<_, i64>(3)?.max(0) as usize,
                    char_offset: row.get::<_, i64>(4)?.max(0) as usize,
                    note: row.get(5)?,
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

    fn load_annotations(&mut self) -> Result<()> {
        self.annotations.clear();
        if let Some(reader) = &self.reader {
            let mut statement = self.db.prepare(
                "SELECT id,line,byte_offset,quote,note FROM annotations WHERE path=?1 ORDER BY created_at DESC,id DESC"
            )?;
            let rows = statement.query_map([reader.book.path.to_string_lossy().as_ref()], |row| {
                Ok(Annotation {
                    id: row.get(0)?,
                    line: row.get::<_, i64>(1)?.max(0) as usize,
                    byte_offset: row.get::<_, i64>(2)?.max(0) as u64,
                    quote: row.get(3)?,
                    note: row.get(4)?,
                })
            })?;
            for row in rows { self.annotations.push(row?); }
        }
        self.annotation_selected = 0;
        self.show_annotations = true;
        Ok(())
    }

    fn edit_selected_annotation(&mut self, note: &str) -> Result<()> {
        if let Some(annotation) = self.annotations.get(self.annotation_selected) {
            self.db.execute("UPDATE annotations SET note=?1 WHERE id=?2", params![note, annotation.id])?;
            self.annotations[self.annotation_selected].note = note.to_string();
            self.status = "笔记已更新".into();
        }
        Ok(())
    }

    fn export_book_notes(&mut self, target: &str) -> Result<()> {
        let Some(reader) = &self.reader else { return Ok(()); };
        let path = reader.book.path.to_string_lossy().to_string();
        let title = reader.book.title.clone();
        let mut output = format!("# {title}\n\n来源：{path}\n\n");
        {
            let mut statement = self.db.prepare("SELECT line,quote,note,created_at FROM annotations WHERE path=?1 ORDER BY line")?;
            let rows = statement.query_map([&path], |row| Ok((
                row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?
            )))?;
            output.push_str("## 笔记与摘录\n\n");
            for row in rows {
                let (line, quote, note, created) = row?;
                output.push_str(&format!("### 第 {} 行 · {}\n\n> {}\n\n{}\n\n", line + 1, created, quote, note));
            }
        }
        {
            let mut statement = self.db.prepare("SELECT line,note,created_at FROM bookmarks WHERE path=?1 ORDER BY line")?;
            let rows = statement.query_map([&path], |row| Ok((
                row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?
            )))?;
            output.push_str("## 书签\n\n");
            for row in rows {
                let (line, note, created) = row?;
                output.push_str(&format!("- 第 {} 行 · {} {}\n", line + 1, created, note));
            }
        }
        fs::write(target, output)?;
        self.status = format!("笔记和书签已导出到 {target}");
        Ok(())
    }

    fn delete_selected_annotation(&mut self) -> Result<()> {
        if let Some(annotation) = self.annotations.get(self.annotation_selected) {
            self.db.execute("DELETE FROM annotations WHERE id=?1", [annotation.id])?;
            self.annotations.remove(self.annotation_selected);
            self.annotation_selected = self.annotation_selected.min(self.annotations.len().saturating_sub(1));
            self.status = "笔记已删除".into();
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

    fn close_reader(&mut self) -> Result<()> {
        self.save_progress()?;
        if let Some(reader) = &self.reader {
            self.db.execute(
                "UPDATE reading_sessions SET ended_at=CURRENT_TIMESTAMP,
                 duration_seconds=CAST((julianday('now')-julianday(started_at))*86400 AS INTEGER)
                 WHERE id=?1 AND ended_at IS NULL",
                [reader.session_id],
            )?;
        }
        self.reader = None;
        self.screen = Screen::Library;
        Ok(())
    }

    fn show_statistics(&mut self) -> Result<()> {
        let (total, sessions, today, week, month): (i64, i64, i64, i64, i64) = self.db.query_row(
            "SELECT
             CAST(COALESCE(SUM(CASE WHEN ended_at IS NULL THEN (julianday('now')-julianday(started_at))*86400 ELSE duration_seconds END),0) AS INTEGER),
             COUNT(*),
             CAST(COALESCE(SUM(CASE WHEN date(started_at,'localtime')=date('now','localtime') THEN
               CASE WHEN ended_at IS NULL THEN (julianday('now')-julianday(started_at))*86400 ELSE duration_seconds END ELSE 0 END),0) AS INTEGER),
             CAST(COALESCE(SUM(CASE WHEN date(started_at,'localtime')>=date('now','localtime','-6 days') THEN
               CASE WHEN ended_at IS NULL THEN (julianday('now')-julianday(started_at))*86400 ELSE duration_seconds END ELSE 0 END),0) AS INTEGER),
             CAST(COALESCE(SUM(CASE WHEN date(started_at,'localtime')>=date('now','localtime','start of month') THEN
               CASE WHEN ended_at IS NULL THEN (julianday('now')-julianday(started_at))*86400 ELSE duration_seconds END ELSE 0 END),0) AS INTEGER)
             FROM reading_sessions", [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))
        )?;
        let goal = load_setting(&self.db, "daily_goal_minutes")?.parse::<i64>().unwrap_or(30);
        let streak: i64 = self.db.query_row(
            "WITH RECURSIVE
             reading_days(day) AS (
               SELECT DISTINCT date(started_at,'localtime')
               FROM reading_sessions
               WHERE duration_seconds > 0 OR ended_at IS NULL
             ),
             anchor(day) AS (
               SELECT MAX(day) FROM reading_days
               WHERE day >= date('now','localtime','-1 day')
             ),
             streak(day,n) AS (
               SELECT day,1 FROM anchor WHERE day IS NOT NULL
               UNION ALL
               SELECT date(streak.day,'-1 day'),streak.n+1
               FROM streak JOIN reading_days
                 ON reading_days.day=date(streak.day,'-1 day')
             )
             SELECT COALESCE(MAX(n),0) FROM streak",
            [], |row| row.get(0)
        )?;
        let hours = total / 3600;
        let minutes = (total % 3600) / 60;
        self.statistics = Some(format!(
            "累计阅读：{hours} 小时 {minutes} 分钟\n阅读会话：{sessions} 次\n今日：{} 分钟 / 目标 {goal} 分钟\n近 7 天：{} 分钟\n本月：{} 分钟\n\n连续阅读：{streak} 天\n按 Esc 关闭",
            today / 60, week / 60, month / 60
        ));
        Ok(())
    }

    fn toggle_theme(&mut self) -> Result<()> {
        self.theme_light = !self.theme_light;
        let theme = if self.theme_light { "light" } else { "dark" };
        self.db.execute("INSERT INTO settings(key,value) VALUES ('theme',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [theme])?;
        self.status = format!("已切换{}主题", if self.theme_light { "浅色" } else { "深色" });
        Ok(())
    }

    fn set_daily_goal(&mut self, value: &str) -> Result<()> {
        let minutes = value.trim().parse::<u32>().ok().filter(|v| (1..=1440).contains(v));
        if let Some(minutes) = minutes {
            self.db.execute("INSERT INTO settings(key,value) VALUES ('daily_goal_minutes',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [minutes.to_string()])?;
            self.status = format!("每日阅读目标设为 {minutes} 分钟");
        } else {
            self.status = "目标必须是 1 到 1440 之间的分钟数".into();
        }
        Ok(())
    }

    fn toggle_favorite(&mut self) -> Result<()> {
        if let Some(index) = self.selected_book_index() {
            let book = &mut self.books[index];
            book.favorite = !book.favorite;
            self.db.execute(
                "INSERT INTO book_metadata(path,title,author,favorite,status) VALUES (?1,?2,?3,?4,?5)
                 ON CONFLICT(path) DO UPDATE SET favorite=excluded.favorite,updated_at=CURRENT_TIMESTAMP",
                params![book.path.to_string_lossy().as_ref(), book.title, book.author, book.favorite as i64, book.status],
            )?;
            self.status = if book.favorite { "已加入收藏".into() } else { "已取消收藏".into() };
        }
        self.refresh_visible_books();
        Ok(())
    }

    fn cycle_book_status(&mut self) -> Result<()> {
        if let Some(index) = self.selected_book_index() {
            let book = &mut self.books[index];
            book.status = match book.status.as_str() {
                "unread" => "reading",
                "reading" => "finished",
                _ => "unread",
            }.into();
            self.db.execute(
                "INSERT INTO book_metadata(path,title,author,favorite,status) VALUES (?1,?2,?3,?4,?5)
                 ON CONFLICT(path) DO UPDATE SET status=excluded.status,updated_at=CURRENT_TIMESTAMP",
                params![book.path.to_string_lossy().as_ref(), book.title, book.author, book.favorite as i64, book.status],
            )?;
            self.status = format!("阅读状态：{}", match book.status.as_str() { "reading" => "阅读中", "finished" => "已读完", _ => "未读" });
        }
        self.refresh_visible_books();
        Ok(())
    }

    fn add_tag_to_selected(&mut self, tag: &str) -> Result<()> {
        let tag = tag.trim();
        if tag.is_empty() { return Ok(()); }
        if let Some(index) = self.selected_book_index() {
            let book = &mut self.books[index];
            self.db.execute("INSERT OR IGNORE INTO tags(name) VALUES (?1)", [tag])?;
            let tag_id: i64 = self.db.query_row("SELECT id FROM tags WHERE name=?1", [tag], |row| row.get(0))?;
            self.db.execute("INSERT OR IGNORE INTO book_tags(path,tag_id) VALUES (?1,?2)", params![book.path.to_string_lossy().as_ref(), tag_id])?;
            if !book.tags.iter().any(|existing| existing == tag) { book.tags.push(tag.to_string()); }
            self.status = format!("已添加标签：{tag}");
        }
        Ok(())
    }

    fn clear_selected_tags(&mut self) -> Result<()> {
        if let Some(index) = self.selected_book_index() {
            let book = &mut self.books[index];
            self.db.execute("DELETE FROM book_tags WHERE path=?1", [book.path.to_string_lossy().as_ref()])?;
            book.tags.clear();
            self.status = "已清除该书全部标签".into();
        }
        Ok(())
    }

    fn edit_selected_metadata(&mut self, value: &str) -> Result<()> {
        if let Some(index) = self.selected_book_index() {
            let book = &mut self.books[index];
            let mut parts = value.splitn(2, '|');
            let title = parts.next().unwrap_or("").trim();
            let author = parts.next().unwrap_or("").trim();
            if !title.is_empty() { book.title = title.to_string(); }
            if !author.is_empty() { book.author = author.to_string(); }
            self.db.execute(
                "INSERT INTO book_metadata(path,title,author,favorite,status) VALUES (?1,?2,?3,?4,?5)
                 ON CONFLICT(path) DO UPDATE SET title=excluded.title,author=excluded.author,updated_at=CURRENT_TIMESTAMP",
                params![book.path.to_string_lossy().as_ref(), book.title, book.author, book.favorite as i64, book.status],
            )?;
            self.status = "书籍元数据已更新（格式：书名|作者）".into();
        }
        Ok(())
    }

    fn search_library(&mut self, query: &str) -> Result<()> {
        self.search_results.clear();
        let books = self.books.clone();
        for book in books {
            if self.search_results.len() >= 200 { break; }
            let source = match reader_source_path(&book.path) { Ok(path) => path, Err(_) => continue };
            let mut document = match Document::open(&source) { Ok(doc) => doc, Err(_) => continue };
            if let Some(line) = document.search(query, 0, false)? {
                let snippet = document.read_line(line)?;
                self.search_results.push(SearchResult { path: book.path, title: book.title, line, snippet });
            }
        }
        self.search_selected = 0;
        self.show_search_results = true;
        self.status = format!("跨书搜索：找到 {} 本匹配书籍", self.search_results.len());
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
    if let Some(reader) = &app.reader {
        app.db.execute(
            "UPDATE reading_sessions SET ended_at=CURRENT_TIMESTAMP,
             duration_seconds=CAST((julianday('now')-julianday(started_at))*86400 AS INTEGER)
             WHERE id=?1 AND ended_at IS NULL",
            [reader.session_id],
        )?;
    }
    Ok(())
}

fn draw(frame: &mut ratatui::Frame, app: &mut App) {
    let area = frame.area();
    let chunks = Layout::default().direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(5), Constraint::Length(2)]).split(area);
    let title = Paragraph::new(Line::from(vec![
        Span::styled(" READTUI ", Style::default().fg(if app.theme_light { Color::Blue } else { Color::Cyan }).add_modifier(Modifier::BOLD)),
        Span::raw("本地小说阅读器"),
    ])).block(Block::default().borders(Borders::ALL));
    frame.render_widget(title, chunks[0]);

    if app.screen == Screen::Library {
        let items: Vec<ListItem> = app.visible_books.iter().filter_map(|index| app.books.get(*index)).map(|book| {
            let path = book.path.strip_prefix(&app.library_dir).unwrap_or(&book.path);
            ListItem::new(format!("{}{} [{}] {}  {}   {}", if book.favorite { "★ " } else { "" }, book.title, match book.status.as_str() { "reading" => "阅读中", "finished" => "已读完", _ => "未读" }, if book.author.is_empty() { String::new() } else { format!("· {} ", book.author) }, book.tags.join(","), path.display())).style(Style::default().fg(if book.tags.is_empty() { Color::White } else { Color::Cyan }))
        }).collect();
        let filter_label = match app.library_filter { 1 => "收藏", 2 => "阅读中", 3 => "未读", 4 => "已读完", _ => "全部" };
        let tag_label = app.tag_filter.as_deref().map(|tag| format!(" / 标签:{tag}")).unwrap_or_default();
        let list = List::new(items).block(Block::default().title(format!("书库 · {} 本 / {filter_label}{tag_label}", app.visible_books.len())).borders(Borders::ALL))
            .highlight_style(Style::default().bg(if app.theme_light { Color::Gray } else { Color::DarkGray }).fg(if app.theme_light { Color::Black } else { Color::White }).add_modifier(Modifier::BOLD))
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
            if reader.char_offset > 0 {
                reader.wrap_offset = wrap_offset_for_char_offset(&current_line, text_width, reader.char_offset);
            } else {
                reader.wrap_offset = reader.wrap_offset.min(wrapped_rows.saturating_sub(1));
            }
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
        let content = Paragraph::new(Text::from(visible))
            .style(Style::default().fg(if app.theme_light { Color::Black } else { Color::White }))
            .wrap(Wrap { trim: false });
        frame.render_widget(content, body);
        let progress = if reader.document.byte_len == 0 { 0 } else {
            (reader.document.offset_for_line(reader.top).saturating_mul(100) / reader.document.byte_len) as usize
        };
        let chapter = reader.chapters.iter().rev().find(|c| c.line <= reader.top).map(|c| c.title.as_str()).unwrap_or("正文");
        let footer_text = format!("{}  ·  {}  ·  {}%  ·  第 {}/{} 行  ·  {}", reader.book.title, chapter, progress, reader.top + 1, line_count, app.status);
        let footer = Paragraph::new(footer_text).style(Style::default().fg(Color::Gray));
        frame.render_widget(footer, chunks[2]);
        if app.show_annotations {
            let popup = centered_rect(80, 75, area);
            frame.render_widget(Clear, popup);
            let entries: Vec<ListItem> = app.annotations.iter().map(|a| {
                let quote = a.quote.chars().take(42).collect::<String>();
                ListItem::new(format!("第 {} 行  {}  — {}", a.line + 1, quote, a.note))
            }).collect();
            let list = List::new(entries)
                .block(Block::default().title("笔记摘录 · Enter 跳转 · e 编辑 · d 删除 · Esc 关闭").borders(Borders::ALL))
                .highlight_style(Style::default().bg(Color::DarkGray).fg(Color::White).add_modifier(Modifier::BOLD))
                .highlight_symbol("▶ ");
            let mut state = ListState::default();
            state.select(if app.annotations.is_empty() { None } else { Some(app.annotation_selected.min(app.annotations.len()-1)) });
            frame.render_stateful_widget(list, popup, &mut state);
        }
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
    if app.show_search_results {
        let popup = centered_rect(85, 80, area);
        frame.render_widget(Clear, popup);
        let entries: Vec<ListItem> = app.search_results.iter().map(|result| {
            let snippet = result.snippet.chars().take(55).collect::<String>();
            ListItem::new(format!("{} · 第 {} 行 · {}", result.title, result.line + 1, snippet))
        }).collect();
        let list = List::new(entries)
            .block(Block::default().title("跨书搜索 · Enter 打开并定位 · Esc 关闭").borders(Borders::ALL))
            .highlight_style(Style::default().bg(Color::DarkGray).fg(Color::White).add_modifier(Modifier::BOLD))
            .highlight_symbol("▶ ");
        let mut state = ListState::default();
        state.select(if app.search_results.is_empty() { None } else { Some(app.search_selected.min(app.search_results.len()-1)) });
        frame.render_stateful_widget(list, popup, &mut state);
    }
    if let Some(stats) = &app.statistics {
        let popup = centered_rect(60, 35, area);
        frame.render_widget(Clear, popup);
        frame.render_widget(Paragraph::new(stats.as_str()).block(Block::default().title("阅读统计").borders(Borders::ALL)), popup);
    }
    if let Some(input) = &app.input {
        let popup = centered_rect(60, 20, area);
        frame.render_widget(Clear, popup);
        let title = if app.directory_import_mode { "输入要导入的目录路径" } else if app.export_mode { "导出笔记/书签的 Markdown 文件路径" } else if app.annotation_edit_mode { "编辑笔记内容" } else if app.goal_mode { "每日阅读目标（分钟）" } else if app.library_search_mode { "跨书搜索关键词" } else if app.annotation_mode { "为当前段落添加笔记" } else if app.tag_filter_mode { "按标签筛选（留空清除）" } else if app.tag_mode { "输入标签名称" } else if app.metadata_mode { "编辑书名|作者（作者可留空）" } else if app.screen == Screen::Reader { "搜索当前书籍" } else { "输入文件路径" };
        frame.render_widget(Paragraph::new(input.as_str()).block(Block::default().title(title).borders(Borders::ALL)), popup);
    }
}


fn char_offset_for_wrap_offset(text: &str, width: usize, wrap_offset: usize) -> usize {
    layout::wrap_line(text, width).iter().take(wrap_offset).map(|row| row.chars().count()).sum()
}

fn wrap_offset_for_char_offset(text: &str, width: usize, char_offset: usize) -> usize {
    let rows = layout::wrap_line(text, width);
    let mut consumed = 0usize;
    for (index, row) in rows.iter().enumerate() {
        let row_end = consumed + row.chars().count();
        if char_offset < row_end { return index; }
        consumed = row_end;
    }
    rows.len().saturating_sub(1)
}

fn centered_rect(width: u16, height: u16, area: ratatui::layout::Rect) -> ratatui::layout::Rect {
    let vert = Layout::default().direction(Direction::Vertical).constraints([Constraint::Percentage((100-height)/2), Constraint::Percentage(height), Constraint::Percentage((100-height)/2)]).split(area);
    Layout::default().direction(Direction::Horizontal).constraints([Constraint::Percentage((100-width)/2), Constraint::Percentage(width), Constraint::Percentage((100-width)/2)]).split(vert[1])[1]
}

fn handle_key(app: &mut App, key: KeyEvent) -> Result<()> {
    if app.statistics.is_some() {
        if matches!(key.code, KeyCode::Esc | KeyCode::Char('S')) { app.statistics = None; }
        return Ok(());
    }
    if let Some(input) = &mut app.input {
        match key.code {
            KeyCode::Esc => {
                app.input = None;
                app.annotation_mode = false;
                app.annotation_edit_mode = false;
                app.export_mode = false;
                app.directory_import_mode = false;
                app.tag_mode = false;
                app.metadata_mode = false;
                app.goal_mode = false;
                app.tag_filter_mode = false;
                app.library_search_mode = false;
            },
            KeyCode::Enter => {
                let value = input.clone(); app.input = None;
                if app.directory_import_mode {
                    app.directory_import_mode = false;
                    app.import_directory(Path::new(&value))?;
                } else if app.export_mode {
                    app.export_mode = false;
                    app.export_book_notes(&value)?;
                } else if app.annotation_edit_mode {
                    app.annotation_edit_mode = false;
                    app.edit_selected_annotation(&value)?;
                } else if app.goal_mode {
                    app.goal_mode = false;
                    app.set_daily_goal(&value)?;
                } else if app.library_search_mode {
                    app.library_search_mode = false;
                    app.search_library(&value)?;
                } else if app.annotation_mode {
                    app.annotation_mode = false;
                    if !value.trim().is_empty() { app.add_annotation(value.trim())?; }
                } else if app.tag_filter_mode {
                    app.tag_filter_mode = false;
                    app.tag_filter = if value.trim().is_empty() { None } else { Some(value.trim().to_string()) };
                    app.refresh_visible_books();
                    app.status = "已更新标签筛选（输入空值清除）".into();
                } else if app.tag_mode {
                    app.tag_mode = false;
                    app.add_tag_to_selected(&value)?;
                } else if app.metadata_mode {
                    app.metadata_mode = false;
                    app.edit_selected_metadata(&value)?;
                } else if app.screen == Screen::Library {
                    let path = PathBuf::from(value);
                    app.add_book_path(path)?;
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
    if app.show_search_results {
        match key.code {
            KeyCode::Esc => app.show_search_results = false,
            KeyCode::Down | KeyCode::Char('j') => {
                if !app.search_results.is_empty() { app.search_selected = (app.search_selected + 1).min(app.search_results.len()-1); }
            }
            KeyCode::Up | KeyCode::Char('k') => app.search_selected = app.search_selected.saturating_sub(1),
            KeyCode::Enter => {
                if let Some(result) = app.search_results.get(app.search_selected).cloned() {
                    app.open_book(result.path)?;
                    if let Some(reader) = &mut app.reader { reader.jump_to_line(result.line); }
                }
                app.show_search_results = false;
            }
            _ => {}
        }
        return Ok(());
    }
    if app.screen == Screen::Library {
        match key.code {
            KeyCode::Char('q') => app.should_quit = true,
            KeyCode::Down | KeyCode::Char('j') => { if !app.visible_books.is_empty() { app.selected = (app.selected + 1).min(app.visible_books.len()-1); } },
            KeyCode::Up | KeyCode::Char('k') => { app.selected = app.selected.saturating_sub(1); },
            KeyCode::Enter => { if let Some(index) = app.selected_book_index() { app.open_book(app.books[index].path.clone())?; } },
            KeyCode::Char('a') => app.input = Some(String::new()),
            KeyCode::Char('A') => { app.directory_import_mode = true; app.input = Some(String::new()); },
            KeyCode::Char('r') => app.rescan_library()?,
            KeyCode::Char('/') => { app.library_search_mode = true; app.input = Some(String::new()); },
            KeyCode::Char('f') => app.toggle_favorite()?,
            KeyCode::Char('s') => app.cycle_book_status()?,
            KeyCode::Char('t') => { app.tag_mode = true; app.input = Some(String::new()); },
            KeyCode::Char('e') => { app.metadata_mode = true; app.input = Some(String::new()); },
            KeyCode::Char('x') => app.clear_selected_tags()?,
            KeyCode::Char('o') => app.books.sort_by(|a,b| b.favorite.cmp(&a.favorite).then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))),
            KeyCode::Char('R') => app.sort_by_recent()?,
            KeyCode::Char('F') => {
                app.library_filter = (app.library_filter + 1) % 5;
                app.refresh_visible_books();
                app.status = "筛选：全部 / 收藏 / 阅读中 / 未读 / 已读完".into();
            },
            KeyCode::Char('T') => { app.tag_filter_mode = true; app.input = Some(String::new()); },
            KeyCode::Char('S') => app.show_statistics()?,
            KeyCode::Char('G') => { app.goal_mode = true; app.input = Some(String::new()); },
            KeyCode::Char('c') => app.toggle_theme()?,
            KeyCode::Char('?') => app.status = "↑/k 上移  ↓/j 下移  Enter 阅读  a 打开文件  A 导入目录  r 重扫书库  / 跨书搜索  F 状态筛选  T 标签筛选  f 收藏  s 状态  t 标签  e 编辑元数据  x 清空标签  o 收藏优先  G 每日目标  c 主题  S 统计  q 退出".into(),
            _ => {}
        }
        app.list_state.select(if app.visible_books.is_empty() { None } else { Some(app.selected) });
    } else {
        if app.show_annotations {
            match key.code {
                KeyCode::Esc | KeyCode::Char('M') => app.show_annotations = false,
                KeyCode::Down | KeyCode::Char('j') => {
                    if !app.annotations.is_empty() { app.annotation_selected = (app.annotation_selected + 1).min(app.annotations.len()-1); }
                }
                KeyCode::Up | KeyCode::Char('k') => app.annotation_selected = app.annotation_selected.saturating_sub(1),
                KeyCode::Char('d') => app.delete_selected_annotation()?,
                KeyCode::Char('e') => {
                    if let Some(annotation) = app.annotations.get(app.annotation_selected) {
                        app.annotation_edit_mode = true;
                        app.input = Some(annotation.note.clone());
                    }
                }
                KeyCode::Enter => {
                    if let Some(annotation) = app.annotations.get(app.annotation_selected) {
                        if let Some(reader) = &mut app.reader {
                            reader.jump_to_line(if annotation.byte_offset > 0 { reader.document.line_for_offset(annotation.byte_offset) } else { annotation.line });
                        }
                    }
                    app.show_annotations = false;
                }
                _ => {}
            }
            return Ok(());
        }
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
                            reader.char_offset = bookmark.char_offset;
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
            KeyCode::Esc | KeyCode::Char('q') => { app.close_reader()?; },
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
            KeyCode::Char('S') => app.show_statistics()?,
            KeyCode::Char('c') => app.toggle_theme()?,
            KeyCode::Char('t') => {
                if let Some(reader) = &app.reader {
                    app.toc_selected = reader.chapters.iter().rposition(|chapter| chapter.line <= reader.top).unwrap_or(0);
                }
                app.show_toc = true;
            },
            KeyCode::Char('b') => app.add_bookmark()?,
            KeyCode::Char('B') => app.load_bookmarks()?,
            KeyCode::Char('M') => app.load_annotations()?,
            KeyCode::Char('E') => { app.export_mode = true; app.input = Some(String::new()); },
            KeyCode::Char('m') => { app.annotation_mode = true; app.input = Some(String::new()); },
            KeyCode::Char('/') => app.input = Some(String::new()),
            KeyCode::Char('n') => if let Some(r) = &mut app.reader {
                if let Some(next) = r.chapters.iter().find(|c| c.line > r.top) { r.top = next.line; r.wrap_offset = 0; }
            },
            KeyCode::Char('N') => if let Some(r) = &mut app.reader {
                if let Some(prev) = r.chapters.iter().rev().find(|c| c.line < r.top) { r.top = prev.line; r.wrap_offset = 0; }
            },
            KeyCode::Char('?') => app.status = "j/k 滚动  Space/PageDown 下一页  PageUp 上一页  t 目录  n/N 章节  b 添加书签  B 书签列表  m 添加笔记  M 笔记列表  E 导出笔记  / 搜索  q 返回".into(),
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
        let path = std::env::temp_dir().join(format!("readtui-chapters-{}.txt", std::process::id()));
        std::fs::write(&path, "序章\n正文\n第一百零二章 风起").unwrap();
        let mut document = Document::open(&path).unwrap();
        let chapters = document.detect_chapters().unwrap();
        assert_eq!(chapters.len(), 2);
        assert_eq!(chapters[0].title, "序章");
        assert_eq!(chapters[1].line, 2);
        drop(document);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn decodes_utf8_text() {
        assert_eq!(document::decode_line("中文小说".as_bytes()), "中文小说");
    }

    #[test]
    fn migrates_legacy_progress_and_bookmark_tables() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE reading_progress(path TEXT PRIMARY KEY,line INTEGER NOT NULL DEFAULT 0,updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP);
             CREATE TABLE bookmarks(id INTEGER PRIMARY KEY AUTOINCREMENT,path TEXT NOT NULL,line INTEGER NOT NULL,note TEXT NOT NULL DEFAULT '',created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP);"
        ).unwrap();
        init_db(&db).unwrap();
        assert!(has_column(&db, "reading_progress", "byte_offset").unwrap());
        assert!(has_column(&db, "reading_progress", "wrap_offset").unwrap());
        assert!(has_column(&db, "reading_progress", "char_offset").unwrap());
        assert!(has_column(&db, "bookmarks", "byte_offset").unwrap());
        assert!(has_column(&db, "bookmarks", "wrap_offset").unwrap());
        assert!(has_column(&db, "bookmarks", "char_offset").unwrap());
    }

    #[test]
    fn character_anchor_survives_terminal_reflow() {
        let text = "第一行第二行第三行第四行";
        let original_row = 1;
        let char_offset = char_offset_for_wrap_offset(text, 4, original_row);
        assert!(char_offset > 0);
        assert_eq!(wrap_offset_for_char_offset(text, 4, char_offset), original_row);
        let resized_row = wrap_offset_for_char_offset(text, 6, char_offset);
        let resized_rows = layout::wrap_line(text, 6);
        assert!(resized_row < resized_rows.len());
        assert_eq!(
            char_offset_for_wrap_offset(text, 6, resized_row),
            resized_rows.iter().take(resized_row).map(|row| row.chars().count()).sum()
        );
    }
}
