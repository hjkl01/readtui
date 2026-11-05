use anyhow::{anyhow, Context, Result};
use roxmltree::{Document, Node};
use std::{
    collections::{hash_map::DefaultHasher, HashMap},
    fs::{self, File},
    hash::{Hash, Hasher},
    io::Read,
    path::{Component, Path, PathBuf},
};
use zip::ZipArchive;

fn entry_text(archive: &mut ZipArchive<File>, name: &str) -> Result<String> {
    let mut file = archive.by_name(name).with_context(|| format!("EPUB 缺少文件：{name}"))?;
    let mut text = String::new();
    file.read_to_string(&mut text).with_context(|| format!("无法读取 EPUB 条目：{name}"))?;
    Ok(text)
}

fn normalized_member(base_file: &str, href: &str) -> String {
    let href = href.split('#').next().unwrap_or(href).split('?').next().unwrap_or(href);
    let decoded = percent_decode(href);
    let joined = Path::new(base_file).parent().unwrap_or(Path::new("")).join(decoded);
    let mut parts = Vec::new();
    for component in joined.components() {
        match component {
            Component::Normal(value) => parts.push(value.to_string_lossy().to_string()),
            Component::ParentDir => { parts.pop(); }
            Component::CurDir | Component::RootDir | Component::Prefix(_) => {}
        }
    }
    parts.join("/")
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(hex) = u8::from_str_radix(&value[i + 1..i + 3], 16) {
                out.push(hex);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn is_block(name: &str) -> bool {
    matches!(name, "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "li" | "blockquote" | "pre" | "td" | "th")
}

fn node_text(node: Node<'_, '_>) -> String {
    node.descendants().filter(|child| child.is_text()).filter_map(|child| child.text())
        .collect::<Vec<_>>().join("")
        .split_whitespace().collect::<Vec<_>>().join(" ")
}

fn xhtml_to_text(source: &str) -> Result<String> {
    let parsed = Document::parse(source).map_err(|e| anyhow!("EPUB XHTML 解析失败：{e}"))?;
    let mut lines = Vec::new();
    for node in parsed.descendants().filter(|node| node.is_element() && is_block(node.tag_name().name())) {
        if node.ancestors().skip(1).any(|parent| parent.is_element() && is_block(parent.tag_name().name())) {
            continue;
        }
        let text = node_text(node);
        if !text.is_empty() { lines.push(text); }
    }
    if lines.is_empty() {
        for node in parsed.descendants().filter(|node| node.is_text()) {
            let text = node.text().unwrap_or("").split_whitespace().collect::<Vec<_>>().join(" ");
            if !text.is_empty() { lines.push(text); }
        }
    }
    Ok(lines.join("\n"))
}


pub fn read_metadata(path: &Path) -> Result<(Option<String>, Option<String>)> {
    let file = File::open(path)?;
    let mut archive = ZipArchive::new(file).context("不是有效的 EPUB/ZIP 文件")?;
    let container = entry_text(&mut archive, "META-INF/container.xml")?;
    let container_xml = Document::parse(&container).context("EPUB container.xml 格式错误")?;
    let opf_path = container_xml.descendants()
        .find(|node| node.has_tag_name("rootfile"))
        .and_then(|node| node.attribute("full-path"))
        .ok_or_else(|| anyhow!("EPUB 未声明 package 文档"))?;
    let opf = entry_text(&mut archive, opf_path)?;
    let package = Document::parse(&opf).context("EPUB OPF 格式错误")?;
    let metadata = package.descendants().find(|node| node.has_tag_name("metadata"));
    let Some(metadata) = metadata else { return Ok((None, None)); };
    let title = metadata.descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "title")
        .map(node_text).filter(|value| !value.is_empty());
    let creator = metadata.descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "creator")
        .map(node_text).filter(|value| !value.is_empty());
    Ok((title, creator))
}

pub fn extract_to_cache(path: &Path, cache_dir: &Path) -> Result<PathBuf> {
    fs::create_dir_all(cache_dir)?;
    let metadata = fs::metadata(path)?;
    let modified = metadata.modified().ok();
    let mut hasher = DefaultHasher::new();
    path.to_string_lossy().hash(&mut hasher);
    metadata.len().hash(&mut hasher);
    if let Some(time) = modified { time.hash(&mut hasher); }
    let cache_path = cache_dir.join(format!("{:016x}.txt", hasher.finish()));
    if cache_path.exists() { return Ok(cache_path); }

    let file = File::open(path)?;
    let mut archive = ZipArchive::new(file).context("不是有效的 EPUB/ZIP 文件")?;
    let container = entry_text(&mut archive, "META-INF/container.xml")?;
    let container_xml = Document::parse(&container).context("EPUB container.xml 格式错误")?;
    let opf_path = container_xml.descendants()
        .find(|node| node.has_tag_name("rootfile"))
        .and_then(|node| node.attribute("full-path"))
        .ok_or_else(|| anyhow!("EPUB 未声明 package 文档"))?
        .to_string();

    let opf = entry_text(&mut archive, &opf_path)?;
    let package = Document::parse(&opf).context("EPUB OPF 格式错误")?;
    let mut manifest = HashMap::new();
    for item in package.descendants().filter(|node| node.has_tag_name("item")) {
        if let (Some(id), Some(href), Some(media)) =
            (item.attribute("id"), item.attribute("href"), item.attribute("media-type"))
        {
            let properties = item.attribute("properties").unwrap_or("").to_string();
            manifest.insert(id.to_string(), (href.to_string(), media.to_string(), properties));
        }
    }

    // EPUB 3 navigation documents use <nav epub:type="toc">; EPUB 2 uses NCX.
    // Normalize archive-relative hrefs so navigation labels can be matched to spine items.
    let mut navigation = HashMap::<String, String>::new();
    for (href, media, properties) in manifest.values() {
        if properties.split_whitespace().any(|value| value == "nav") {
            let member = normalized_member(&opf_path, href);
            if let Ok(source) = entry_text(&mut archive, &member) {
                if let Ok(nav) = Document::parse(&source) {
                    for node in nav.descendants().filter(|node| node.is_element() && node.tag_name().name() == "nav") {
                        let is_toc = node.attributes().any(|attribute| attribute.value().contains("toc"));
                        if !is_toc { continue; }
                        for link in node.descendants().filter(|child| child.is_element() && child.tag_name().name() == "a") {
                            if let Some(target) = link.attribute("href") {
                                let title = node_text(link);
                                if !title.is_empty() {
                                    navigation.insert(normalized_member(&member, target), title);
                                }
                            }
                        }
                    }
                }
            }
        }
        if media == "application/x-dtbncx+xml" || href.to_ascii_lowercase().ends_with(".ncx") {
            let spine_toc = package.descendants().find(|node| node.has_tag_name("spine"))
                .and_then(|node| node.attribute("toc"));
            if spine_toc.is_some() {
                let member = normalized_member(&opf_path, href);
                if let Ok(source) = entry_text(&mut archive, &member) {
                    if let Ok(ncx) = Document::parse(&source) {
                        for point in ncx.descendants().filter(|node| node.is_element() && node.tag_name().name() == "navPoint") {
                            let target = point.descendants().find(|node| node.is_element() && node.tag_name().name() == "content")
                                .and_then(|node| node.attribute("src"));
                            let title = point.descendants().find(|node| node.is_element() && node.tag_name().name() == "navLabel")
                                .map(node_text).unwrap_or_default();
                            if let (Some(target), false) = (target, title.is_empty()) {
                                navigation.insert(normalized_member(&member, target), title);
                            }
                        }
                    }
                }
            }
        }
    }

    let mut chapters = Vec::new();
    for itemref in package.descendants().filter(|node| node.has_tag_name("itemref")) {
        let Some(idref) = itemref.attribute("idref") else { continue; };
        let Some((href, media, _)) = manifest.get(idref) else { continue; };
        if !media.contains("html") && !href.ends_with(".xhtml") && !href.ends_with(".html") { continue; }
        let member = normalized_member(&opf_path, href);
        let source = entry_text(&mut archive, &member)
            .with_context(|| format!("无法读取 EPUB 章节：{member}"))?;
        let chapter = xhtml_to_text(&source)?;
        if !chapter.trim().is_empty() {
            let mut content = chapter.trim().to_string();
            if let Some(title) = navigation.get(&member) {
                if content.lines().next().map(str::trim) == Some(title.as_str()) {
                    content = content.lines().skip(1).collect::<Vec<_>>().join("\n");
                }
                content = format!("第{}章 {}\n{}", chapters.len() + 1, title, content);
            }
            chapters.push(content);
        }
    }
    if chapters.is_empty() { return Err(anyhow!("EPUB 中没有找到可读取的正文章节")); }
    let content = chapters.join("\n\n");
    fs::write(&cache_path, content)?;
    Ok(cache_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_title_and_creator_metadata() {
        let source = r#"<package xmlns:dc="http://purl.org/dc/elements/1.1/"><metadata><dc:title>示例书名</dc:title><dc:creator>示例作者</dc:creator></metadata></package>"#;
        let parsed = Document::parse(source).unwrap();
        let metadata = parsed.descendants().find(|node| node.has_tag_name("metadata")).unwrap();
        let title = metadata.descendants().find(|node| node.tag_name().name() == "title").map(node_text);
        let creator = metadata.descendants().find(|node| node.tag_name().name() == "creator").map(node_text);
        assert_eq!(title.as_deref(), Some("示例书名"));
        assert_eq!(creator.as_deref(), Some("示例作者"));
    }

    #[test]
    fn strips_xhtml_into_readable_paragraphs() {
        let html = r#"<html xmlns="http://www.w3.org/1999/xhtml"><body><h1>第一章</h1><p>你好 <em>世界</em>。</p><p>第二段</p></body></html>"#;
        let text = xhtml_to_text(html).unwrap();
        assert!(text.contains("第一章"));
        assert!(text.contains("你好 世界。"));
        assert!(text.contains("第二段"));
    }

    #[test]
    fn resolves_parent_paths_inside_archive() {
        assert_eq!(normalized_member("OPS/package.opf", "Text/ch1.xhtml"), "OPS/Text/ch1.xhtml");
        assert_eq!(normalized_member("OPS/package.opf", "../Images/a.png"), "Images/a.png");
    }
}
