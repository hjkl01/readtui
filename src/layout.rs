use unicode_width::UnicodeWidthChar;

/// Wrap one source line into terminal-width-aware visual lines.
///
/// CJK opening punctuation is kept with the following character and closing
/// punctuation is kept with the preceding character whenever possible.
pub fn wrap_line(line: &str, max_width: usize) -> Vec<String> {
    let max_width = max_width.max(1);
    if line.is_empty() {
        return vec![String::new()];
    }

    let chars: Vec<char> = line.chars().collect();
    let mut rows = Vec::new();
    let mut current = String::new();
    let mut width = 0usize;
    let mut index = 0usize;

    while index < chars.len() {
        let ch = chars[index];
        let ch_width = ch.width().unwrap_or(0);

        if width + ch_width > max_width && !current.is_empty() {
            // Do not leave closing punctuation at the beginning of a row.
            if is_closing_punctuation(ch) {
                current.push(ch);
                rows.push(std::mem::take(&mut current));
                width = 0;
                index += 1;
                continue;
            }

            rows.push(std::mem::take(&mut current));
            width = 0;
            // Do not leave opening punctuation at the end of a row.
            if is_opening_punctuation(ch) && index + 1 < chars.len() {
                let next = chars[index + 1];
                let next_width = next.width().unwrap_or(0);
                if next_width <= max_width {
                    current.push(ch);
                    current.push(next);
                    width = ch_width + next_width;
                    index += 2;
                    continue;
                }
            }
        }

        current.push(ch);
        width += ch_width;
        index += 1;
    }

    if !current.is_empty() {
        rows.push(current);
    }
    rows
}

fn is_closing_punctuation(ch: char) -> bool {
    matches!(ch, '，' | '。' | '！' | '？' | '；' | '：' | '、' | '）' | '】' | '》' | '」' | '』' | '〉' | '〕' | '”' | '’' | ',' | '.' | '!' | '?' | ';' | ':' | ')')
}

fn is_opening_punctuation(ch: char) -> bool {
    matches!(ch, '（' | '【' | '《' | '「' | '『' | '〈' | '〔' | '“' | '‘' | '(')
}

/// Number of source lines consumed by one visual page.
pub fn page_end(lines: &[String], start: usize, width: usize, height: usize) -> usize {
    if lines.is_empty() {
        return 0;
    }
    let mut rows = 0usize;
    let mut line = start.min(lines.len() - 1);
    while line < lines.len() && rows < height.max(1) {
        rows += wrap_line(&lines[line], width).len();
        line += 1;
    }
    line.max(start.saturating_add(1))
}

/// Find the source-line start of the previous visual page.
pub fn page_start(lines: &[String], end: usize, width: usize, height: usize) -> usize {
    if lines.is_empty() || end == 0 {
        return 0;
    }
    let mut rows = 0usize;
    let mut line = end.min(lines.len());
    while line > 0 && rows < height.max(1) {
        line -= 1;
        rows += wrap_line(&lines[line], width).len();
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_wide_characters_by_terminal_cells() {
        let rows = wrap_line("中文小说阅读", 4);
        assert_eq!(rows, vec!["中文", "小说", "阅读"]);
    }

    #[test]
    fn keeps_closing_punctuation_with_previous_text() {
        let rows = wrap_line("你好，世界。", 4);
        assert_eq!(rows, vec!["你好，", "世界。"]);
    }

    #[test]
    fn page_navigation_uses_visual_rows() {
        let lines = vec!["短行".into(), "很长很长很长的一行".into(), "尾行".into()];
        let end = page_end(&lines, 0, 4, 3);
        assert_eq!(end, 2);
        assert_eq!(page_start(&lines, end, 4, 3), 0);
    }
}
