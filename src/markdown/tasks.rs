//! 메모 원문에서 할 일(ADR-0024)을 읽는다.
//!
//! 할 일은 GFM 할 일 항목(`- [ ] …`)이다. 정보는 Obsidian Tasks 표기로 같은 줄에 적는다:
//! 예정일 `⏳`, 시작일 `🛫`, 마감일 `📅`, 완료일 `✅`, 만든 날 `➕`, 식별자 `🆔`, 태그 `#…`.
//! 할 일 줄 아래 들여 쓴 줄은 그 할 일의 블록이다. 들여 쓴 할 일 항목은 체크리스트, 나머지는 메모다.
//!
//! 파일 전체를 보므로 코드 펜스와 맨 앞 frontmatter 안의 줄은 할 일로 보지 않는다.

use serde::{Deserialize, Serialize};

use super::task_marker;

/// 할 일 하나와 원문 위치.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Todo {
    /// 할 일 줄(0부터).
    pub line: usize,
    /// 블록(할 일 줄 + 들여 쓴 메모·체크리스트)의 마지막 줄.
    pub end_line: usize,
    /// 할 일 줄 원문. 원문에 다시 쓰기 전에 그 자리에 같은 줄이 있는지 이것으로 확인한다.
    pub source: String,
    pub checked: bool,
    /// 표기(날짜·태그 등)를 뺀 제목.
    pub title: String,
    pub scheduled: Option<String>,
    pub start: Option<String>,
    pub due: Option<String>,
    pub done: Option<String>,
    pub created: Option<String>,
    pub id: Option<String>,
    /// `#`을 뺀 태그.
    pub tags: Vec<String>,
    /// 들여 쓴 메모 줄(들여쓰기를 뺀 원문, 줄바꿈으로 잇는다).
    pub notes: String,
    pub checklist: Vec<ChecklistItem>,
}

/// 할 일 블록 안의 체크리스트 항목.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChecklistItem {
    pub text: String,
    pub checked: bool,
}

/// 할 일의 날짜 칸 하나.
type DateSlot = fn(&mut Todo) -> &mut Option<String>;

/// 날짜를 받는 표기와 그 칸.
const DATE_SIGNIFIERS: [(char, DateSlot); 5] = [
    ('⏳', |todo| &mut todo.scheduled),
    ('🛫', |todo| &mut todo.start),
    ('📅', |todo| &mut todo.due),
    ('✅', |todo| &mut todo.done),
    ('➕', |todo| &mut todo.created),
];

const ID_SIGNIFIER: char = '🆔';

/// 원문의 할 일을 나온 순서로 읽는다.
pub fn todos(text: &str) -> Vec<Todo> {
    let lines: Vec<&str> = text.split('\n').map(|line| line.trim_end_matches('\r')).collect();
    let skipped = skipped_lines(&lines);
    let mut out = Vec::new();
    let mut row = 0;
    while row < lines.len() {
        let line = lines[row];
        let Some(marker) = (!skipped[row]).then(|| task_marker(line)).flatten() else {
            row += 1;
            continue;
        };
        let indent = indent_width(line);
        let end = block_end(&lines, &skipped, row, indent);
        let mut todo = parse_line(&line[marker.cover.end..]);
        todo.line = row;
        todo.end_line = end;
        todo.source = line.to_string();
        todo.checked = marker.checked;
        // 항목 내용이 시작하는 칸(`- ` 다음, `[` 자리). 메모 줄은 이만큼 들여 쓴다.
        let leading = line.len() - line.trim_start_matches([' ', '\t']).len();
        let content_indent = indent + (marker.check - 1 - leading);
        let mut notes = Vec::new();
        for child in &lines[row + 1..=end] {
            match task_marker(child) {
                Some(item) => todo.checklist.push(ChecklistItem {
                    text: strip_trailing_signifiers(child[item.cover.end..].trim()),
                    checked: item.checked,
                }),
                None => notes.push(strip_indent(child, content_indent)),
            }
        }
        while notes.last().is_some_and(|note| note.trim().is_empty()) {
            notes.pop();
        }
        todo.notes = notes.join("\n");
        out.push(todo);
        row = end + 1;
    }
    out
}

/// 할 일로 보지 않을 줄: 코드 펜스 안과 맨 앞 frontmatter.
fn skipped_lines(lines: &[&str]) -> Vec<bool> {
    let mut skipped = vec![false; lines.len()];
    let mut row = 0;
    if lines.first().is_some_and(|line| line.trim_end() == "---")
        && let Some(close) = lines[1..].iter().position(|line| line.trim_end() == "---")
    {
        for flag in &mut skipped[..=close + 1] {
            *flag = true;
        }
        row = close + 2;
    }
    let mut fence: Option<(char, usize)> = None;
    while row < lines.len() {
        let body = lines[row].trim_start();
        let fence_char = body.chars().next().filter(|c| *c == '`' || *c == '~');
        let run = fence_char.map_or(0, |c| body.chars().take_while(|x| *x == c).count());
        match fence {
            Some((c, len)) => {
                skipped[row] = true;
                if fence_char == Some(c) && run >= len && body[run..].trim().is_empty() {
                    fence = None;
                }
            }
            None if run >= 3 => {
                skipped[row] = true;
                fence = fence_char.map(|c| (c, run));
            }
            None => {}
        }
        row += 1;
    }
    skipped
}

/// `row`번 할 일 줄의 블록 끝: 할 일 줄보다 깊이 들여 쓴 줄(사이의 빈 줄 포함)까지.
fn block_end(lines: &[&str], skipped: &[bool], row: usize, indent: usize) -> usize {
    let mut end = row;
    for (next, line) in lines.iter().enumerate().skip(row + 1) {
        if line.trim().is_empty() {
            continue;
        }
        if skipped[next] || indent_width(line) <= indent {
            break;
        }
        end = next;
    }
    end
}

/// 줄 앞 공백 폭(탭은 4칸).
fn indent_width(line: &str) -> usize {
    line.chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .map(|c| if c == '\t' { 4 } else { 1 })
        .sum()
}

/// 앞 공백을 `width`칸까지 뺀다.
fn strip_indent(line: &str, width: usize) -> String {
    let mut taken = 0;
    let mut start = 0;
    for (ix, c) in line.char_indices() {
        if taken >= width || !(c == ' ' || c == '\t') {
            break;
        }
        taken += if c == '\t' { 4 } else { 1 };
        start = ix + c.len_utf8();
    }
    line[start..].to_string()
}

/// 할 일 줄의 기호 뒤 글에서 제목과 표기를 읽는다.
fn parse_line(rest: &str) -> Todo {
    let mut todo = Todo::default();
    let mut title: Vec<&str> = Vec::new();
    let mut words = rest.split_whitespace().peekable();
    while let Some(word) = words.next() {
        let signifier = word.trim_end_matches('\u{fe0f}');
        let mut chars = signifier.chars();
        let first = chars.next();
        let alone = chars.next().is_none();
        if alone && let Some(slot) = DATE_SIGNIFIERS.iter().find(|(c, _)| Some(*c) == first) {
            if let Some(date) = words.next_if(|next| is_date(next)) {
                *slot.1(&mut todo) = Some(date.to_string());
                continue;
            }
        } else if alone && first == Some(ID_SIGNIFIER) {
            if let Some(id) = words.next() {
                todo.id = Some(id.to_string());
                continue;
            }
        } else if let Some(tag) = tag(word) {
            todo.tags.push(tag.to_string());
            continue;
        }
        title.push(word);
    }
    todo.title = title.join(" ");
    todo
}

/// 체크리스트 항목 글. 날짜 표기는 남기지 않는다(Things의 체크리스트는 글만 있다).
fn strip_trailing_signifiers(text: &str) -> String {
    parse_line(text).title
}

/// `#태그`의 태그. 숫자만인 것(`#1`)과 `#` 하나는 태그가 아니다.
fn tag(word: &str) -> Option<&str> {
    let tag = word.strip_prefix('#')?;
    let valid = !tag.is_empty()
        && !tag.chars().all(|c| c.is_ascii_digit())
        && tag
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '/'));
    valid.then_some(tag)
}

/// `YYYY-MM-DD`인지.
fn is_date(word: &str) -> bool {
    let bytes = word.as_bytes();
    bytes.len() == 10
        && bytes.iter().enumerate().all(|(ix, byte)| match ix {
            4 | 7 => *byte == b'-',
            _ => byte.is_ascii_digit(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_title_dates_tags_notes_and_checklist() {
        let text = "# 회의\n\n- [ ] 출판사 미팅 ⏳ 2026-10-25 📅 2026-10-30 #출판 #계약/검토\n  원고 3장, 표지 시안\n\n  지난 피드백 확인\n  - [ ] 원고 출력\n  - [x] 표지 시안 받기\n\n다음 문단\n";
        let todos = todos(text);
        assert_eq!(todos.len(), 1);
        let todo = &todos[0];
        assert_eq!(todo.title, "출판사 미팅");
        assert_eq!((todo.line, todo.end_line), (2, 7));
        assert!(!todo.checked);
        assert_eq!(todo.scheduled.as_deref(), Some("2026-10-25"));
        assert_eq!(todo.due.as_deref(), Some("2026-10-30"));
        assert_eq!(todo.tags, ["출판", "계약/검토"]);
        assert_eq!(todo.notes, "원고 3장, 표지 시안\n\n지난 피드백 확인");
        assert_eq!(
            todo.checklist,
            [
                ChecklistItem {
                    text: "원고 출력".into(),
                    checked: false
                },
                ChecklistItem {
                    text: "표지 시안 받기".into(),
                    checked: true
                },
            ]
        );
        assert!(todo.source.starts_with("- [ ] 출판사 미팅"));
    }

    #[test]
    fn done_date_id_and_variation_selector() {
        let todos = todos("- [x] 끝냄 ✅\u{fe0f} 2026-10-22 🆔 abc123\n");
        assert!(todos[0].checked);
        assert_eq!(todos[0].done.as_deref(), Some("2026-10-22"));
        assert_eq!(todos[0].id.as_deref(), Some("abc123"));
        assert_eq!(todos[0].title, "끝냄");
    }

    #[test]
    fn keeps_unknown_text_and_bad_dates_in_the_title() {
        let todos = todos("- [ ] 📅 내일 #1 # 쓰기\n");
        assert_eq!(todos[0].title, "📅 내일 #1 # 쓰기");
        assert_eq!(todos[0].due, None);
        assert!(todos[0].tags.is_empty());
    }

    #[test]
    fn skips_code_fences_and_frontmatter() {
        let text = "---\ntitle: - [ ] 아님\n---\n```\n- [ ] 코드\n```\n~~~~\n- [ ] 또 코드\n~~~~\n- [ ] 진짜\n";
        let todos = todos(text);
        assert_eq!(todos.len(), 1);
        assert_eq!(todos[0].title, "진짜");
        assert_eq!(todos[0].line, 9);
    }

    #[test]
    fn sibling_and_nested_under_plain_items() {
        // 일반 항목 아래 할 일은 할 일이다. 할 일 아래 할 일은 체크리스트다.
        let text = "- 회의\n  - [ ] 할 일 하나\n    - [ ] 하위\n- [ ] 할 일 둘\n";
        let todos = todos(text);
        let titles: Vec<_> = todos.iter().map(|todo| todo.title.as_str()).collect();
        assert_eq!(titles, ["할 일 하나", "할 일 둘"]);
        assert_eq!(todos[0].checklist.len(), 1);
        assert_eq!((todos[1].line, todos[1].end_line), (3, 3));
    }
}
