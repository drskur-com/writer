//! 저장되는 마크다운 ↔ 에디터 본문 변환(ADR-0011).
//!
//! 에디터에서는 줄 하나가 문단이다(Enter = 새 문단, Ulysses·iA Writer와 같은 경험). 파일에는 표준
//! 마크다운으로 저장해 다른 앱에서도 문단이 그대로 보이게 한다.
//!
//! - 에디터의 줄바꿈 하나 = 마크다운의 빈 줄 하나(문단 경계).
//! - 에디터의 빈 줄 k개 = 마크다운의 빈 줄 k+1개. 표준 렌더러는 추가 빈 줄을 버리므로 다른 앱과
//!   내보내기에서는 문단 경계 하나로 보이고, 이 앱으로 다시 열면 빈 줄이 돌아온다.
//! - 줄바꿈 하나로 이어져야 하는 줄(목록 항목끼리, 인용 줄끼리, 표 행끼리, 들여쓴 코드, 줄 끝
//!   강제 줄바꿈, setext 밑줄)은 빈 줄을 넣지 않는다. 코드·수식 펜스와 frontmatter 안은 그대로 둔다.
//! - 다른 앱에서 만든 파일의 문단 안 줄바꿈(`A\nB`, 한 문단)은 읽을 때 공백으로 이어 한 줄로 만든다.
//!
//! 에디터 본문 → 마크다운 → 에디터 본문은 원래대로 돌아온다(테스트로 지킨다).

/// 줄 하나의 종류. 줄 사이에 무엇을 넣을지 정할 때 쓴다.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Blank,
    /// 펜스·frontmatter 안의 줄(여는·닫는 줄 포함). 원문 그대로 둔다.
    Verbatim,
    /// 목록 항목(`-`, `*`, `+`, `1.`, `1)`).
    ListItem,
    Quote,
    TableRow,
    /// 네 칸 이상 들여쓴 줄(들여쓴 코드나 목록 안 문단).
    Indented,
    /// setext 제목 밑줄이나 구분선(`===`, `---`).
    Rule,
    /// ATX 제목.
    Heading,
    /// 문단 줄.
    Text,
}

/// 펜스·frontmatter를 따라가며 줄마다 종류를 정한다.
fn classify(lines: &[&str]) -> Vec<Kind> {
    let mut kinds = Vec::with_capacity(lines.len());
    // 열린 펜스의 닫는 표시. frontmatter는 `---`로 닫는다.
    let mut fence: Option<(char, usize)> = None;
    for (ix, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        if let Some((mark, len)) = fence {
            let closes = indent < 4
                && trimmed.starts_with(&mark.to_string().repeat(len))
                && trimmed.trim_end().chars().all(|c| c == mark);
            if closes {
                fence = None;
            }
            kinds.push(Kind::Verbatim);
            continue;
        }
        if ix == 0 && line.trim_end() == "---" {
            fence = Some(('-', 3));
            kinds.push(Kind::Verbatim);
            continue;
        }
        if indent < 4
            && let Some(mark) = ['`', '~', '$']
                .into_iter()
                .find(|&c| trimmed.starts_with(c))
        {
            let len = trimmed.chars().take_while(|&c| c == mark).count();
            // 수식 펜스는 `$$`만 있는 줄로 연다(`$$x$$` 같은 한 줄 수식과 구분).
            let opens = if mark == '$' {
                len >= 2 && trimmed.trim_end().len() == len
            } else {
                len >= 3
            };
            if opens {
                fence = Some((mark, len));
                kinds.push(Kind::Verbatim);
                continue;
            }
        }
        kinds.push(kind_of(line));
    }
    kinds
}

/// 펜스 밖 줄 하나의 종류.
fn kind_of(line: &str) -> Kind {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Kind::Blank;
    }
    let indent = line.len() - line.trim_start().len();
    if indent >= 4 {
        return Kind::Indented;
    }
    if super::heading_level(line).is_some() {
        return Kind::Heading;
    }
    if trimmed.len() >= 3
        && (trimmed.chars().all(|c| c == '=') || trimmed.chars().all(|c| c == '-'))
    {
        return Kind::Rule;
    }
    if trimmed.starts_with('>') {
        return Kind::Quote;
    }
    if trimmed.starts_with('|') {
        return Kind::TableRow;
    }
    let marker = trimmed.split_once([' ', '\t']).map(|(marker, _)| marker);
    let list = marker.is_some_and(|marker| {
        matches!(marker, "-" | "*" | "+")
            || marker.strip_suffix(['.', ')']).is_some_and(|number| {
                !number.is_empty()
                    && number.len() <= 9
                    && number.bytes().all(|b| b.is_ascii_digit())
            })
    });
    if list { Kind::ListItem } else { Kind::Text }
}

/// 줄 끝 강제 줄바꿈(공백 두 칸 이상 또는 `\`)이 있는지.
fn hard_break(line: &str) -> bool {
    line.ends_with("  ") || line.ends_with('\\')
}

/// 두 줄이 빈 줄 없이 줄바꿈 하나로 이어져야 하는지(빈 줄을 넣으면 뜻이 바뀌는지).
fn tight(prev: (&str, Kind), next: (&str, Kind)) -> bool {
    use Kind::*;
    let (prev_line, prev) = prev;
    let (next_line, next) = next;
    if prev == Verbatim || next == Verbatim {
        return false;
    }
    let indented_item = next_line.starts_with([' ', '\t']) && prev == ListItem;
    hard_break(prev_line)
        || matches!(
            (prev, next),
            (ListItem | Indented, ListItem | Indented)
                | (Quote, Quote)
                | (TableRow, TableRow)
                | (Text, Rule)
        )
        || indented_item
}

/// 빈 줄이 아닌 줄과 그 앞의 빈 줄 수. 펜스 안의 빈 줄은 빈 줄로 치지 않는다(원문 그대로).
struct Line<'a> {
    text: &'a str,
    kind: Kind,
    /// 이 줄 앞(바로 앞 줄 이후)의 빈 줄 수.
    blanks_before: usize,
}

fn lines_with_blanks<'a>(lines: &[&'a str], kinds: &[Kind]) -> (Vec<Line<'a>>, usize) {
    let mut out = Vec::new();
    let mut blanks = 0;
    for (line, &kind) in lines.iter().zip(kinds) {
        if kind == Kind::Blank {
            blanks += 1;
        } else {
            out.push(Line {
                text: line,
                kind,
                blanks_before: blanks,
            });
            blanks = 0;
        }
    }
    (out, blanks)
}

/// 텍스트를 줄로 나눈다. 끝의 줄바꿈 뒤에는 빈 줄이 하나 있는 것으로 친다.
///
/// 문서 처음과 끝의 빈 줄은 변환하지 않고 그대로 옮긴다.
fn split(text: &str) -> Vec<&str> {
    text.split('\n').collect()
}

/// 줄을 잇는다. 문서 처음의 빈 줄은 그대로 옮기고, 줄 사이에는 `gap`이 정한 줄바꿈을 넣는다.
fn join(out: &mut String, lines: &[Line<'_>], gap: impl Fn(&Line<'_>, &Line<'_>) -> String) {
    for (ix, line) in lines.iter().enumerate() {
        if ix == 0 {
            out.push_str(&"\n".repeat(line.blanks_before));
        } else {
            out.push_str(&gap(&lines[ix - 1], line));
        }
        out.push_str(line.text);
    }
}

/// 에디터 본문 → 저장할 마크다운.
pub fn to_markdown(editor: &str) -> String {
    let lines = split(editor);
    let kinds = classify(&lines);
    let (lines, trailing_blanks) = lines_with_blanks(&lines, &kinds);
    if lines.is_empty() {
        return editor.to_string();
    }
    let mut out = String::with_capacity(editor.len() + lines.len());
    join(&mut out, &lines, |prev, next| {
        if prev.kind == Kind::Verbatim && next.kind == Kind::Verbatim {
            // 펜스 안의 빈 줄은 원문 그대로.
            return "\n".repeat(next.blanks_before + 1);
        }
        let blanks =
            if next.blanks_before == 0 && tight((prev.text, prev.kind), (next.text, next.kind)) {
                0
            } else {
                next.blanks_before + 1
            };
        "\n".repeat(blanks + 1)
    });
    out.push_str(&"\n".repeat(trailing_blanks));
    out
}

/// 저장된 마크다운 → 에디터 본문.
pub fn to_editor(markdown: &str) -> String {
    let lines = split(markdown);
    let kinds = classify(&lines);
    let (lines, trailing_blanks) = lines_with_blanks(&lines, &kinds);
    if lines.is_empty() {
        return markdown.to_string();
    }
    let mut out = String::with_capacity(markdown.len());
    for (ix, line) in lines.iter().enumerate() {
        if ix == 0 {
            out.push_str(&"\n".repeat(line.blanks_before));
            out.push_str(line.text);
            continue;
        }
        let prev = &lines[ix - 1];
        if prev.kind == Kind::Verbatim && line.kind == Kind::Verbatim {
            out.push_str(&"\n".repeat(line.blanks_before + 1));
        } else if line.blanks_before > 0 {
            // 빈 줄 하나는 문단 경계, 나머지는 에디터의 빈 줄.
            out.push_str(&"\n".repeat(line.blanks_before));
        } else if !tight((prev.text, prev.kind), (line.text, line.kind))
            && line.kind == Kind::Text
            && matches!(prev.kind, Kind::Text | Kind::ListItem | Kind::Quote)
        {
            // 문단 안 줄바꿈(다른 앱에서 만든 파일): 한 줄로 잇는다.
            out.push(' ');
            out.push_str(line.text.trim_start());
            continue;
        } else {
            out.push('\n');
        }
        out.push_str(line.text);
    }
    out.push_str(&"\n".repeat(trailing_blanks));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 에디터 본문이 저장 후 다시 열었을 때 그대로인지.
    #[track_caller]
    fn round_trips(editor: &str) {
        let markdown = to_markdown(editor);
        assert_eq!(to_editor(&markdown), editor, "markdown: {markdown:?}");
    }

    #[test]
    fn lines_are_paragraphs_and_blank_lines_survive() {
        assert_eq!(to_markdown("A\nB"), "A\n\nB");
        assert_eq!(to_markdown("A\n\nB"), "A\n\n\nB");
        assert_eq!(to_markdown("A\n\n\nB\n"), "A\n\n\n\nB\n");
        assert_eq!(to_editor("A\n\nB"), "A\nB");
        assert_eq!(to_editor("A\n\n\n\nB"), "A\n\n\nB");
        for editor in [
            "",
            "A",
            "A\n",
            "\n\nA\nB\n\n",
            "# 제목\n문단\n\n## 장\n문단",
        ] {
            round_trips(editor);
        }
    }

    #[test]
    fn soft_breaks_from_other_apps_join_into_one_paragraph() {
        assert_eq!(
            to_editor("긴 문단의\n둘째 줄\n\n다음 문단"),
            "긴 문단의 둘째 줄\n다음 문단"
        );
        // 강제 줄바꿈은 이어 붙이지 않고 그대로 둔다.
        assert_eq!(to_editor("시의 첫 행  \n둘째 행"), "시의 첫 행  \n둘째 행");
        round_trips("시의 첫 행  \n둘째 행\n다음 연");
    }

    #[test]
    fn lists_quotes_and_tables_stay_tight() {
        let editor = "- 하나\n- 둘\n  - 셋\n1. 첫째\n2. 둘째\n> 인용\n> 이어짐\n| a | b |\n|---|---|\n| 1 | 2 |";
        let markdown = to_markdown(editor);
        assert!(markdown.contains("- 하나\n- 둘\n  - 셋"), "{markdown}");
        assert!(markdown.contains("> 인용\n> 이어짐"), "{markdown}");
        assert!(
            markdown.contains("| a | b |\n|---|---|\n| 1 | 2 |"),
            "{markdown}"
        );
        round_trips(editor);
        round_trips("문단\n- 목록\n다음 문단");
        round_trips("제목\n===\n문단");
    }

    #[test]
    fn fences_and_frontmatter_are_kept_verbatim() {
        let editor = "---\ntitle: a\n---\n문단\n```rust\nfn a() {}\n\n\nfn b() {}\n```\n$$\nx\n\ny\n$$\n문단";
        let markdown = to_markdown(editor);
        assert!(
            markdown.starts_with("---\ntitle: a\n---\n\n문단"),
            "{markdown}"
        );
        assert!(
            markdown.contains("```rust\nfn a() {}\n\n\nfn b() {}\n```"),
            "{markdown}"
        );
        assert!(markdown.contains("$$\nx\n\ny\n$$"), "{markdown}");
        round_trips(editor);
        // 닫히지 않은 펜스는 끝까지 그대로.
        round_trips("문단\n```\n코드\n\n코드");
        // 한 줄 수식은 펜스가 아니다.
        round_trips("$$x$$\n문단");
    }

    /// 여러 종류의 줄을 네 줄까지 모든 순서로 섞어도 원래대로 돌아온다.
    #[test]
    fn every_short_mix_of_lines_round_trips() {
        const SAMPLES: [&str; 14] = [
            "문단",
            "",
            "- 항목",
            "  - 하위",
            "> 인용",
            "| 표 |",
            "    코드",
            "===",
            "# 제목",
            "강제 줄바꿈  ",
            "```",
            "x",
            "---",
            "1. 번호",
        ];
        let mut stack = vec![Vec::<&str>::new()];
        while let Some(lines) = stack.pop() {
            let editor = lines.join("\n");
            round_trips(&editor);
            round_trips(&format!("{editor}\n"));
            if lines.len() < 4 {
                for sample in SAMPLES {
                    let mut next = lines.clone();
                    next.push(sample);
                    stack.push(next);
                }
            }
        }
    }

    #[test]
    fn standard_renderers_see_separate_paragraphs() {
        let html = markdown::to_html(&to_markdown("A\n\nB\nC"));
        assert_eq!(html, "<p>A</p>\n<p>B</p>\n<p>C</p>");
    }
}
