use anyhow::{Context, Result};
use regex::Regex;
use roxmltree::Document;
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

fn html_to_text(source: &str) -> String {
    let scripts = Regex::new(r"(?is)<script[^>]*>.*?</script>|<style[^>]*>.*?</style>").unwrap();
    let blocks = Regex::new(r"(?i)<\s*/?\s*(br|p|div|h[1-6]|li|blockquote|tr|section|article|hr)[^>]*>").unwrap();
    let tags = Regex::new(r"(?s)<[^>]+>").unwrap();
    let clean = scripts.replace_all(source, "");
    let clean = blocks.replace_all(&clean, "\n");
    let clean = tags.replace_all(&clean, "");
    clean.replace("&nbsp;", " ").replace("&amp;", "&").replace("&lt;", "<")
        .replace("&gt;", ">").replace("&quot;", "\"").lines()
        .map(str::trim).filter(|line| !line.is_empty()).collect::<Vec<_>>().join("\n")
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
        "md" | "markdown" => "markdown", "html" | "htm" => "html", "fb2" => "fb2",
        _ => return Ok(source.to_path_buf()),
    };
    let target = cache_path(source, cache_dir, kind)?;
    if target.exists() { return Ok(target); }
    let raw = fs::read_to_string(source).with_context(|| format!("无法读取 {} 文本", ext.to_uppercase()))?;
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
