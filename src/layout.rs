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
            // Keep closing punctuation with the preceding character when possible.
            if is_closing_punctuation(ch) {
                if let Some(previous) = current.pop() {
                    let previous_width = previous.width().unwrap_or(0);
                    if !current.is_empty() {
                        rows.push(std::mem::take(&mut current));
                    }
                    current.push(previous);
                    current.push(ch);
                    width = previous_width + ch_width;
                    index += 1;
                    continue;
                }
            }

            rows.push(std::mem::take(&mut current));
            width = 0;
            // Keep an opening punctuation mark with the following character.
            if is_opening_punctuation(ch) && index + 1 < chars.len() {
                let next = chars[index + 1];
                let next_width = next.width().unwrap_or(0);
                if ch_width + next_width <= max_width {
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
        assert_eq!(rows, vec!["你", "好，", "世", "界。"]);
    }

    #[test]
    fn cursor_navigation_uses_visual_rows() {
        let lines = vec!["短行".into(), "很长很长很长的一行".into(), "尾行".into()];
        let (mut line, mut offset) = (0, 0);
        advance(&lines, &mut line, &mut offset, 4, 3);
        assert_eq!((line, offset), (1, 2));
        retreat(&lines, &mut line, &mut offset, 4, 3);
        assert_eq!((line, offset), (0, 0));
    }
}

/// Map a character offset within a source line to its wrapped visual row.
pub fn wrap_offset_for_char_offset(line: &str, width: usize, char_offset: usize) -> usize {
    let rows = wrap_line(line, width);
    let mut consumed = 0usize;
    for (index, row) in rows.iter().enumerate() {
        let next = consumed + row.chars().count();
        if char_offset < next {
            return index;
        }
        if char_offset == next {
            return (index + 1).min(rows.len().saturating_sub(1));
        }
        consumed = next;
    }
    rows.len().saturating_sub(1)
}

#[cfg(test)]
mod position_tests {
    use super::*;

    #[test]
    fn restores_character_position_after_width_changes() {
        assert_eq!(wrap_offset_for_char_offset("中文小说阅读", 4, 4), 2);
        assert_eq!(wrap_offset_for_char_offset("中文小说阅读", 2, 4), 4);
        assert_eq!(wrap_offset_for_char_offset("abcde", 3, 4), 1);
    }
}

/// Advance a cursor expressed as (source line, wrapped row within that line).
pub fn advance(lines: &[String], line: &mut usize, offset: &mut usize, width: usize, rows: usize) {
    if lines.is_empty() { *line = 0; *offset = 0; return; }
    let mut remaining = rows.max(1);
    while remaining > 0 && *line < lines.len() {
        let count = wrap_line(&lines[*line], width).len();
        let available = count.saturating_sub(*offset);
        if remaining < available {
            *offset += remaining;
            return;
        }
        remaining -= available;
        if *line + 1 >= lines.len() {
            *offset = count.saturating_sub(1);
            return;
        }
        *line += 1;
        *offset = 0;
    }
}

/// Retreat a cursor expressed as (source line, wrapped row within that line).
pub fn retreat(lines: &[String], line: &mut usize, offset: &mut usize, width: usize, rows: usize) {
    if lines.is_empty() { *line = 0; *offset = 0; return; }
    let mut remaining = rows.max(1);
    while remaining > 0 {
        if *offset > 0 {
            let step = remaining.min(*offset);
            *offset -= step;
            remaining -= step;
        } else if *line > 0 {
            *line -= 1;
            let count = wrap_line(&lines[*line], width).len();
            *offset = count.saturating_sub(1);
            remaining -= 1;
        } else {
            break;
        }
    }
}
