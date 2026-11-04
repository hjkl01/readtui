use anyhow::{Context, Result};
use encoding_rs::{BIG5, GB18030};
use regex::Regex;
use std::{
    fs::File,
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone)]
pub struct Chapter {
    pub title: String,
    pub line: usize,
    pub byte_offset: u64,
}

pub struct Document {
    pub path: PathBuf,
    file: File,
    pub line_offsets: Vec<u64>,
    pub byte_len: u64,
}

impl Document {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let file = File::open(&path).with_context(|| format!("无法打开文件：{}", path.display()))?;
        let byte_len = file.metadata()?.len();
        let mut offsets = vec![0u64];
        let mut reader = BufReader::new(file.try_clone()?);
        let mut buffer = Vec::with_capacity(8192);
        let mut position = 0u64;

        loop {
            buffer.clear();
            let read = reader.read_until(b'\n', &mut buffer)?;
            if read == 0 { break; }
            position += read as u64;
            if position < byte_len {
                offsets.push(position);
            }
        }

        if byte_len == 0 {
            offsets.clear();
        }

        Ok(Self { path, file, line_offsets: offsets, byte_len })
    }

    pub fn line_count(&self) -> usize {
        self.line_offsets.len()
    }

    pub fn offset_for_line(&self, line: usize) -> u64 {
        self.line_offsets.get(line).copied().unwrap_or(self.byte_len)
    }

    pub fn line_for_offset(&self, offset: u64) -> usize {
        match self.line_offsets.binary_search(&offset) {
            Ok(line) => line,
            Err(next) => next.saturating_sub(1).min(self.line_offsets.len().saturating_sub(1)),
        }
    }

    pub fn seek_offset(&mut self, offset: u64) -> Result<()> {
        self.file.seek(SeekFrom::Start(offset.min(self.byte_len)))?;
        Ok(())
    }

    pub fn read_line(&mut self, line: usize) -> Result<String> {
        let Some(offset) = self.line_offsets.get(line).copied() else {
            return Ok(String::new());
        };
        self.file.seek(SeekFrom::Start(offset))?;
        let mut reader = BufReader::new(&mut self.file);
        let mut bytes = Vec::with_capacity(1024);
        reader.read_until(b'\n', &mut bytes)?;
        if bytes.last() == Some(&b'\n') { bytes.pop(); }
        if bytes.last() == Some(&b'\r') { bytes.pop(); }
        Ok(decode_line(&bytes))
    }

    pub fn detect_chapters(&mut self) -> Result<Vec<Chapter>> {
        let patterns = [
            r"^\s*第[零〇一二三四五六七八九十百千万两0-9０-９]+[章节回卷集部篇].*",
            r"^\s*(序章|楔子|引子|序言|后记|尾声|终章)\s*$",
            r"^\s*Chapter\s+[0-9IVXivx]+.*",
            r"^\s*卷[一二三四五六七八九十0-9]+.*",
        ];
        let regexes: Vec<Regex> = patterns.iter().filter_map(|p| Regex::new(p).ok()).collect();
        let mut chapters = Vec::new();
        for line in 0..self.line_count() {
            let text = self.read_line(line)?;
            let title = text.trim();
            if !title.is_empty() && regexes.iter().any(|regex| regex.is_match(title)) {
                chapters.push(Chapter {
                    title: title.to_string(),
                    line,
                    byte_offset: self.offset_for_line(line),
                });
            }
        }
        if chapters.is_empty() && self.line_count() > 0 {
            chapters.push(Chapter { title: "正文".into(), line: 0, byte_offset: 0 });
        }
        Ok(chapters)
    }

    pub fn search(&mut self, query: &str, from_line: usize, case_sensitive: bool) -> Result<Option<usize>> {
        if query.is_empty() { return Ok(None); }
        for line in from_line..self.line_count() {
            let text = self.read_line(line)?;
            let found = if case_sensitive {
                text.contains(query)
            } else {
                text.to_lowercase().contains(&query.to_lowercase())
            };
            if found { return Ok(Some(line)); }
        }
        Ok(None)
    }
}

pub fn decode_line(bytes: &[u8]) -> String {
    if let Ok(text) = std::str::from_utf8(bytes) {
        return text.trim_start_matches('\u{feff}').to_string();
    }
    let (decoded, _, errors) = GB18030.decode(bytes);
    if !errors { return decoded.into_owned(); }
    BIG5.decode(bytes).0.into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_offsets_round_trip() {
        let path = std::env::temp_dir().join(format!("readtui-index-{}.txt", std::process::id()));
        std::fs::write(&path, "第一行\n第二行\n第三行").unwrap();
        let mut doc = Document::open(&path).unwrap();
        assert_eq!(doc.line_count(), 3);
        assert_eq!(doc.read_line(1).unwrap(), "第二行");
        assert_eq!(doc.line_for_offset(doc.offset_for_line(2)), 2);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn handles_empty_files() {
        let path = std::env::temp_dir().join(format!("readtui-empty-{}.txt", std::process::id()));
        std::fs::write(&path, "").unwrap();
        let doc = Document::open(&path).unwrap();
        assert_eq!(doc.line_count(), 0);
        std::fs::remove_file(path).unwrap();
    }
}
