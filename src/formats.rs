use anyhow::{Context, Result};
use regex::Regex;
use roxmltree::Document;
use scraper::{Html, Selector};
use mobi::{headers::Encryption, Mobi};
use std::{collections::hash_map::DefaultHasher, fs, hash::{Hash, Hasher}, path::{Path, PathBuf}};

fn cache_path(source: &Path, cache_dir: &Path, kind: &str) -> Result<PathBuf> {
    fs::create_dir_all(cache_dir)?;
    let metadata = fs::metadata(source)?;
    let mut hasher = DefaultHasher::new();
    source.to_string_lossy().hash(&mut hasher);
    metadata.len().hash(&mut hasher);
    metadata.modified().ok().hash(&mut hasher);
    Ok(cache_dir.join(format!("{:016x}.{kind}.txt", hasher.finish())))
}

pub(crate) fn html_to_text(source: &str) -> String {
    // EPUB/MOBI markup is often HTML-like rather than well-formed XML.
    // HTML5 parsing tolerates unquoted attributes and legacy markup.
    let scripts = Regex::new(r"(?is)<script\b[^>]*>.*?</script\s*>|<style\b[^>]*>.*?</style\s*>").unwrap();
    let clean = scripts.replace_all(source, "");
    let document = Html::parse_document(&clean);
    let blocks = Selector::parse("p, h1, h2, h3, h4, h5, h6, li, blockquote, pre, td, th").unwrap();
    let mut lines = document.select(&blocks).filter_map(|element| {
        let text = element.text().collect::<Vec<_>>().join("");
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        (!text.is_empty()).then_some(text)
    }).collect::<Vec<_>>();

    if lines.is_empty() {
        let body = Selector::parse("body").unwrap();
        if let Some(element) = document.select(&body).next() {
            let text = element.text().collect::<Vec<_>>().join("");
            let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
            if !text.is_empty() { lines.push(text); }
        }
    }
    lines.join("\n")
}

pub fn mobi_metadata(source: &Path) -> Result<(Option<String>, Option<String>)> {
    let book = Mobi::from_path(source).context("MOBI 文件头或元数据解析失败")?;
    if book.encryption() != Encryption::No {
        anyhow::bail!("该 MOBI 文件受 DRM 加密，readtui 不支持解密");
    }
    let title = book.title().trim().to_string();
    let author = book.author().map(|value| value.trim().to_string()).filter(|value| !value.is_empty());
    Ok(((!title.is_empty()).then_some(title), author))
}

fn mobi_to_cache(source: &Path, cache_dir: &Path) -> Result<PathBuf> {
    let target = cache_path(source, cache_dir, "mobi")?;
    if target.exists() { return Ok(target); }
    let book = Mobi::from_path(source).context("MOBI 文件头或元数据解析失败")?;
    if book.encryption() != Encryption::No {
        anyhow::bail!("该 MOBI 文件受 DRM 加密，readtui 不支持解密");
    }
    let markup = book.content_as_string().context("MOBI 正文解压或字符解码失败")?;
    let text = html_to_text(&markup);
    if text.trim().is_empty() {
        anyhow::bail!("MOBI 文件中没有可读取的正文内容");
    }
    fs::write(&target, text)?;
    Ok(target)
}

fn markdown_to_text(source: &str) -> String {
    let link = Regex::new(r"\[([^\]]+)\]\([^)]+\)").unwrap();
    let heading = Regex::new(r"^\s{0,3}#{1,6}\s+").unwrap();
    let quote = Regex::new(r"^\s*>\s?").unwrap();
    let emphasis = Regex::new(r"[*_]([^*_]+)[*_]").unwrap();
    let mut output = Vec::new();
    let mut in_code = false;
    let fence = char::from(96).to_string().repeat(3);
    for raw in source.lines() {
        let trimmed = raw.trim_start();
        if trimmed.starts_with("~~~") || trimmed.starts_with(&fence) { in_code = !in_code; continue; }
        if in_code { output.push(raw.to_string()); continue; }
        let mut line = link.replace_all(raw, "$1").into_owned();
        line = heading.replace(&line, "").into_owned();
        line = quote.replace(&line, "").into_owned();
        line = line.replace("**", "").replace("__", "").replace("~~", "").replace(char::from(96), "");
        line = emphasis.replace_all(&line, "$1").into_owned();
        output.push(line);
    }
    output.join("\n")
}

fn fb2_to_text(source: &str) -> Result<String> {
    let xml = Document::parse(source).context("FB2 XML 格式错误")?;
    let mut lines = Vec::new();
    for node in xml.descendants().filter(|node| node.is_element()) {
        if !matches!(node.tag_name().name(), "title" | "subtitle" | "p" | "v" | "text-author" | "date") { continue; }
        let text = node.descendants().filter_map(|child| child.text()).collect::<Vec<_>>().join("");
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if !text.is_empty() { lines.push(text); }
    }
    if lines.is_empty() { anyhow::bail!("FB2 文件中没有正文内容"); }
    Ok(lines.join("\n"))
}

pub fn to_cache(source: &Path, cache_dir: &Path) -> Result<PathBuf> {
    let ext = source.extension().and_then(|value| value.to_str()).unwrap_or("").to_ascii_lowercase();
    let kind = match ext.as_str() {
        "md" | "markdown" => "markdown", "html" | "htm" => "html", "fb2" => "fb2", "mobi" => "mobi",
        _ => return Ok(source.to_path_buf()),
    };
    let target = cache_path(source, cache_dir, kind)?;
    if target.exists() { return Ok(target); }
    if kind == "mobi" { return mobi_to_cache(source, cache_dir); }\n    let raw = fs::read_to_string(source).with_context(|| format!("无法读取 {} 文本", ext.to_uppercase()))?;
    let text = match kind {
        "markdown" => markdown_to_text(&raw), "html" => html_to_text(&raw), "fb2" => fb2_to_text(&raw)?,
        _ => raw,
    };
    fs::write(&target, text)?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn markdown_markup_is_removed() {
        let text = markdown_to_text("# 标题\n\n**加粗** [链接](https://example.com)");
        assert!(text.contains("标题")); assert!(text.contains("加粗 链接")); assert!(!text.contains("https://"));
    }
    #[test]
    fn html5_parser_accepts_unquoted_attributes_and_doctype() {
        let text = html_to_text(r#"<!doctype html><html><head><meta http-equiv=Content-Type content=text/html></head><body><p>第一段</p><p>第二段</p></body></html>"#);
        assert_eq!(text, "第一段\n第二段");
    }

    #[test]
    fn html_paragraphs_are_extracted() {
        let text = html_to_text("<h1>标题</h1><p>正文 &amp; 内容</p><script>bad()</script>");
        assert!(text.contains("标题")); assert!(text.contains("正文 & 内容")); assert!(!text.contains("bad"));
    }
    #[test]
    fn fb2_paragraphs_are_extracted() {
        let text = fb2_to_text(r#"<FictionBook><body><section><title><p>第一章</p></title><p>正文内容</p></section></body></FictionBook>"#).unwrap();
        assert!(text.contains("第一章")); assert!(text.contains("正文内容"));
    }
}
