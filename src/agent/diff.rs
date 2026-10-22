//! 승인 카드에 보일 변경 내용(diff).
//!
//! 줄(시트에서는 문단) 단위로 비교하고, 바뀐 줄 안에서는 바뀐 글자를 강조한다. 바뀌지 않은 줄은
//! 바뀐 곳 앞뒤 한 줄만 짧게 보이고, 그 사이는 생략 표시(`Gap`)로 줄인다.

use std::ops::Range;

use similar::{ChangeTag, InlineChangeMode, InlineChangeOptions, TextDiff};

/// 바뀌지 않은 줄을 보일 때 이보다 길면 자른다(좁은 패널).
const MAX_CONTEXT_CHARS: usize = 60;

/// 바뀐 곳 앞뒤로 보일 바뀌지 않은 줄 수.
const CONTEXT_LINES: usize = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    /// 바뀌지 않은 줄.
    Context,
    Removed,
    Added,
    /// 생략한 바뀌지 않은 줄들.
    Gap,
}

/// diff 한 줄.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: LineKind,
    /// 줄 끝 줄바꿈을 뺀 글.
    pub text: String,
    /// 강조할 범위(`text`의 바이트 범위). 바뀐 줄 안에서 실제로 바뀐 글자다.
    pub emphasis: Vec<Range<usize>>,
}

/// `before`에서 `after`로의 변경. 같으면 빈 목록.
pub fn diff(before: &str, after: &str) -> Vec<DiffLine> {
    let diff = TextDiff::from_lines(before, after);
    let mut options = InlineChangeOptions::default();
    options.mode(InlineChangeMode::Chars).semantic_cleanup(true);
    let mut lines = Vec::new();
    for (ix, group) in diff.grouped_ops(CONTEXT_LINES).iter().enumerate() {
        if ix > 0 {
            lines.push(gap());
        }
        for op in group {
            for change in diff.iter_inline_changes_with_options(op, options) {
                let kind = match change.tag() {
                    ChangeTag::Equal => LineKind::Context,
                    ChangeTag::Delete => LineKind::Removed,
                    ChangeTag::Insert => LineKind::Added,
                };
                let mut text = String::new();
                let mut emphasis = Vec::new();
                for (emphasized, part) in change.iter_strings_lossy() {
                    let start = text.len();
                    text.push_str(&part);
                    if emphasized {
                        emphasis.push(start..text.len());
                    }
                }
                trim_newline(&mut text, &mut emphasis);
                if kind == LineKind::Context {
                    // 문단 사이 빈 줄은 보일 것이 없다.
                    if text.trim().is_empty() {
                        continue;
                    }
                    shorten(&mut text);
                    emphasis.clear();
                }
                lines.push(DiffLine {
                    kind,
                    text,
                    emphasis,
                });
            }
        }
    }
    lines
}

/// 새 파일의 diff: 모든 줄이 더해진 줄이다.
pub fn added(text: &str) -> Vec<DiffLine> {
    diff("", text)
}

fn gap() -> DiffLine {
    DiffLine {
        kind: LineKind::Gap,
        text: String::new(),
        emphasis: Vec::new(),
    }
}

fn trim_newline(text: &mut String, emphasis: &mut Vec<Range<usize>>) {
    while text.ends_with('\n') || text.ends_with('\r') {
        text.pop();
    }
    let len = text.len();
    emphasis.retain_mut(|range| {
        range.end = range.end.min(len);
        range.start < range.end
    });
}

fn shorten(text: &mut String) {
    if let Some((cut, _)) = text.char_indices().nth(MAX_CONTEXT_CHARS) {
        text.truncate(cut);
        text.push('…');
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(lines: &[DiffLine]) -> Vec<(LineKind, &str)> {
        lines
            .iter()
            .map(|line| (line.kind, line.text.as_str()))
            .collect()
    }

    #[test]
    fn shows_changed_paragraph_with_emphasis_and_context() {
        let before = "첫 문단.\n\n그는 문을 열엇다.\n\n끝 문단.\n";
        let after = "첫 문단.\n\n그는 문을 열었다.\n\n끝 문단.\n";
        let lines = diff(before, after);
        assert_eq!(
            kinds(&lines),
            [
                (LineKind::Removed, "그는 문을 열엇다."),
                (LineKind::Added, "그는 문을 열었다."),
            ]
        );
        // 바뀐 글자만 강조한다.
        let added = &lines[1];
        let emphasized: Vec<&str> = added
            .emphasis
            .iter()
            .map(|r| &added.text[r.clone()])
            .collect();
        assert_eq!(emphasized, ["었"]);
    }

    #[test]
    fn collapses_far_apart_changes() {
        let before = "a\nb\nc\nd\ne\nf\ng\n";
        let after = "A\nb\nc\nd\ne\nf\nG\n";
        assert_eq!(
            kinds(&diff(before, after)),
            [
                (LineKind::Removed, "a"),
                (LineKind::Added, "A"),
                (LineKind::Context, "b"),
                (LineKind::Gap, ""),
                (LineKind::Context, "f"),
                (LineKind::Removed, "g"),
                (LineKind::Added, "G"),
            ]
        );
    }

    #[test]
    fn new_file_is_all_added_and_long_context_is_cut() {
        assert_eq!(
            kinds(&added("# 제목\n\n본문\n")),
            [
                (LineKind::Added, "# 제목"),
                (LineKind::Added, ""),
                (LineKind::Added, "본문"),
            ]
        );
        let long = "가".repeat(100);
        let lines = diff(&format!("{long}\nx\n"), &format!("{long}\ny\n"));
        assert_eq!(lines[0].text.chars().count(), MAX_CONTEXT_CHARS + 1);
        assert!(lines[0].text.ends_with('…'));
        assert!(diff("same\n", "same\n").is_empty());
    }
}
