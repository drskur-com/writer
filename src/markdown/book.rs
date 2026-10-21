//! 마크다운 원문 → 책 블록. 미리보기(와 나중의 내보내기)가 시트를 책 모양으로 배치할 때 쓴다.
//!
//! gpui 타입을 쓰지 않는다. 에디터·프리뷰와 같은 파서(markdown-rs)·같은 옵션을 쓴다.
//! 이미지만 있는 문단은 ADR-0007 속성(`{width=…}`)까지 읽어 이미지 블록으로 만든다.

use markdown::mdast::Node;

use super::{ImageLine, image_line, parse_options};

/// 글자 꾸밈.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Marks {
    pub strong: bool,
    pub emphasis: bool,
    pub strike: bool,
    /// 인라인 코드·수식(고정폭).
    pub code: bool,
    pub link: bool,
}

/// 같은 꾸밈의 글자 조각. `\n`은 강제 줄바꿈이다.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inline {
    pub text: String,
    pub marks: Marks,
}

/// 문단 하나의 글자.
pub type Rich = Vec<Inline>;

/// 꾸밈 없는 글자 조각 하나.
pub fn plain(text: impl Into<String>) -> Rich {
    vec![Inline {
        text: text.into(),
        marks: Marks::default(),
    }]
}

/// 꾸밈을 뺀 글자.
pub fn rich_text(rich: &[Inline]) -> String {
    rich.iter().map(|inline| inline.text.as_str()).collect()
}

/// 책 블록.
#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    /// ATX·setext 제목(레벨 1~6).
    Heading {
        level: u8,
        text: Rich,
    },
    Paragraph(Rich),
    /// 원문에 명시한 빈 줄 수(줄바꿈 유지 옵션).
    BlankLines(usize),
    Quote(Vec<Block>),
    List {
        ordered: bool,
        start: u32,
        items: Vec<Vec<Block>>,
    },
    /// 코드·수식 블록(원문 그대로).
    Code(String),
    Image(ImageLine),
    /// 구분선.
    Rule,
    /// 표. 첫 행이 머리 행이다.
    Table(Vec<Vec<Rich>>),
    /// 시트 끝에 모은 각주(번호, 내용).
    Footnotes(Vec<(usize, Vec<Block>)>),
}

/// 시트 원문을 책 블록으로 바꾼다. 각주는 나온 순서로 번호를 매겨 끝에 모은다.
pub fn blocks(text: &str) -> Vec<Block> {
    blocks_with_line_breaks(text, false)
}

/// 저장된 원고를 읽는다. 줄바꿈 유지 시 블록 사이의 저장용 구분 빈 줄은 세지 않는다.
pub fn manuscript_blocks(text: &str, preserve_line_breaks: bool) -> Vec<Block> {
    blocks_with_line_breaks(text, preserve_line_breaks)
}

/// 일반 개행과 블록 사이 빈 줄을 선택적으로 유지한다.
pub fn blocks_with_line_breaks(text: &str, preserve_line_breaks: bool) -> Vec<Block> {
    let groups = crate::text_transform::line_groups(text);
    let source = ungroup(text, &groups);
    let Ok(root) = markdown::to_mdast(&source, &parse_options()) else {
        return Vec::new();
    };
    let mut cx = Walk {
        source: &source,
        preserve_line_breaks,
        groups,
        footnotes: Vec::new(),
        definitions: Vec::new(),
    };
    let mut out = cx.blocks(children(&root));
    if preserve_line_breaks {
        let leading = text.lines().take_while(|line| line.trim().is_empty()).count();
        let trailing = text.lines().rev().take_while(|line| line.trim().is_empty()).count();
        if leading > 0 {
            out.insert(0, Block::BlankLines(leading));
        }
        // 빈 줄만 있는 원문은 중복해서 넣지 않는다.
        if trailing > 0 && !text.trim().is_empty() {
            out.push(Block::BlankLines(trailing));
        }
    }
    // 참조한 순서대로 정의를 붙인다. 참조하지 않은 정의는 뺀다.
    let notes: Vec<(usize, Vec<Block>)> = cx
        .footnotes
        .iter()
        .enumerate()
        .filter_map(|(ix, id)| {
            let (_, body) = cx.definitions.iter().find(|(def, _)| def == id)?;
            Some((ix + 1, body.clone()))
        })
        .collect();
    if !notes.is_empty() {
        out.push(Block::Footnotes(notes));
    }
    out
}

/// 줄 묶음을 강제 줄바꿈으로 이은 한 문단으로 바꾼다. 줄 수는 그대로다(빈 줄 세기·위치용).
///
/// 펜스 줄은 빈 줄이 되어 앞뒤 블록과 나뉜다. 안의 줄은 블록 문법으로 읽히지 않게 첫머리를 바꾸고
/// (들여쓰기는 줄바꿈 없는 공백, 블록 기호는 `\` 이스케이프) 마지막 글줄 앞까지 끝에 `\`를 붙인다.
/// 안의 빈 줄은 빈 행으로 남는다. 묶음 끝의 빈 줄은 버린다.
fn ungroup(text: &str, groups: &[std::ops::Range<usize>]) -> String {
    if groups.is_empty() {
        return text.to_string();
    }
    let mut lines: Vec<String> = text.split('\n').map(str::to_string).collect();
    for group in groups {
        let fence_end = group.end.min(lines.len());
        // 닫는 줄(여는 줄 이상의 콜론뿐)이 있으면 마지막 줄도 펜스다.
        let colons = |line: &str| line.bytes().take_while(|&b| b == b':').count();
        let last = lines[fence_end - 1].trim_end();
        let closed = fence_end - group.start >= 2
            && last.bytes().all(|b| b == b':')
            && colons(last) >= colons(&lines[group.start]);
        let body = group.start + 1..if closed { fence_end - 1 } else { fence_end };
        let last_text = body.clone().rev().find(|&ix| !lines[ix].trim().is_empty());
        lines[group.start].clear();
        if closed {
            lines[fence_end - 1].clear();
        }
        for ix in body {
            if last_text.is_none_or(|last| ix > last) {
                lines[ix].clear();
                continue;
            }
            let mut line = literal_line(&lines[ix]);
            if Some(ix) != last_text {
                line.push('\\');
            }
            lines[ix] = line;
        }
    }
    lines.join("\n")
}

/// 묶음 안의 줄 하나를 문단 안 글줄로 읽히게 한다. 인라인 꾸밈은 그대로 둔다.
fn literal_line(line: &str) -> String {
    let line = line.strip_suffix('\r').unwrap_or(line);
    // 끝의 강제 줄바꿈 표시는 뺀다(묶음이 줄바꿈을 넣는다).
    let mut line = line.trim_end();
    if line.ends_with('\\') && !line.ends_with("\\\\") {
        line = &line[..line.len() - 1];
    }
    let rest = line.trim_start_matches([' ', '\t']);
    let indent = &line[..line.len() - rest.len()];
    if !indent.is_empty() {
        // 줄바꿈 없는 공백은 들여쓴 코드·목록으로 읽히지 않는다.
        let spaces: String = indent
            .chars()
            .map(|c| if c == '\t' { "\u{a0}".repeat(4) } else { "\u{a0}".to_string() })
            .collect();
        return format!("{spaces}{rest}");
    }
    let mut out = String::with_capacity(line.len() + 1);
    match block_marker(line) {
        Some(ix) => {
            out.push_str(&line[..ix]);
            out.push('\\');
            out.push_str(&line[ix..]);
        }
        None => out.push_str(line),
    }
    out
}

/// 줄 첫머리가 블록 문법(제목·인용·목록·구분선·setext 밑줄·표 구분 행·펜스·HTML·정의)이면
/// 이스케이프할 기호의 위치.
fn block_marker(line: &str) -> Option<usize> {
    let first = line.chars().next()?;
    let after = |marker: usize| line[marker..].starts_with([' ', '\t']) || line.len() == marker;
    let only = |chars: &[char]| line.chars().all(|c| chars.contains(&c) || c == ' ' || c == '\t');
    let escape = match first {
        '#' => super::heading_level(line).is_some(),
        '>' | '<' | '|' => true,
        '-' | '+' | '*' => after(1) || only(&['-', ':', '|']) || only(&[first]),
        '_' => only(&['_']),
        '=' => only(&['=']),
        ':' => only(&['-', ':', '|']),
        '`' | '~' => line.starts_with(&first.to_string().repeat(3)),
        '$' => line.starts_with("$$"),
        '[' => line.find("]:").is_some_and(|ix| !line[1..ix].contains(']')),
        '0'..='9' => {
            let digits = line.bytes().take_while(u8::is_ascii_digit).count();
            return (digits <= 9
                && line[digits..].starts_with(['.', ')'])
                && after(digits + 1))
                .then_some(digits);
        }
        _ => false,
    };
    escape.then_some(0)
}

struct Walk<'a> {
    source: &'a str,
    preserve_line_breaks: bool,
    /// 줄 묶음의 줄 범위(0부터, 펜스 포함). 빈 줄을 셀 때 묶음 문단을 펜스까지 넓혀 본다.
    groups: Vec<std::ops::Range<usize>>,
    /// 참조한 각주 식별자(순서 = 번호 - 1).
    footnotes: Vec<String>,
    /// 각주 정의(식별자, 내용).
    definitions: Vec<(String, Vec<Block>)>,
}

fn children(node: &Node) -> &[Node] {
    node.children().map(Vec::as_slice).unwrap_or(&[])
}

impl Walk<'_> {
    fn blocks(&mut self, nodes: &[Node]) -> Vec<Block> {
        let mut out = Vec::new();
        let mut previous_end = None;
        for node in nodes {
            let start = out.len();
            self.block(node, &mut out);
            let lines = self.line_span(node);
            if self.preserve_line_breaks
                && out.len() > start
                && let (Some(end), Some((first, _))) = (previous_end, lines)
            {
                // 블록 사이 첫 빈 줄은 저장 형식의 문단 경계이고, 나머지가 원문의 빈 줄이다.
                let blank_lines = first.saturating_sub(end + 2);
                if blank_lines > 0 {
                    out.insert(start, Block::BlankLines(blank_lines));
                }
            }
            previous_end = lines.map(|(_, last)| last);
        }
        out
    }

    /// 노드의 (첫 줄, 끝 줄)(1부터). 줄 묶음 문단은 펜스 줄까지 넓힌다.
    fn line_span(&self, node: &Node) -> Option<(usize, usize)> {
        let position = node.position()?;
        let (start, end) = (position.start.line, position.end.line);
        Some(
            self.groups
                .iter()
                .find(|group| group.contains(&(start - 1)))
                .map_or((start, end), |group| (group.start + 1, group.end.max(end))),
        )
    }

    fn block(&mut self, node: &Node, out: &mut Vec<Block>) {
        match node {
            Node::Heading(heading) => {
                let text = self.inlines(&heading.children);
                out.push(Block::Heading {
                    level: heading.depth,
                    text,
                });
            }
            Node::Paragraph(paragraph) => {
                // 이미지만 있는 문단(속성 포함)은 이미지 블록.
                let source = paragraph
                    .position
                    .as_ref()
                    .and_then(|pos| self.source.get(pos.start.offset..pos.end.offset));
                if let Some(image) = source.and_then(|line| image_line(line.trim())) {
                    out.push(Block::Image(image));
                    return;
                }
                let text = self.inlines(&paragraph.children);
                if !rich_text(&text).trim().is_empty() {
                    out.push(Block::Paragraph(text));
                }
            }
            Node::Blockquote(quote) => {
                let inner = self.blocks(&quote.children);
                out.push(Block::Quote(inner));
            }
            Node::List(list) => {
                let items = list
                    .children
                    .iter()
                    .map(|item| self.blocks(children(item)))
                    .collect();
                out.push(Block::List {
                    ordered: list.ordered,
                    start: list.start.unwrap_or(1),
                    items,
                });
            }
            Node::Code(code) => out.push(Block::Code(code.value.clone())),
            Node::Math(math) => out.push(Block::Code(math.value.clone())),
            Node::ThematicBreak(_) => out.push(Block::Rule),
            Node::Table(table) => {
                let rows = table
                    .children
                    .iter()
                    .map(|row| {
                        children(row)
                            .iter()
                            .map(|cell| self.inlines(children(cell)))
                            .collect()
                    })
                    .collect();
                out.push(Block::Table(rows));
            }
            Node::FootnoteDefinition(definition) => {
                let body = self.blocks(&definition.children);
                self.definitions.push((definition.identifier.clone(), body));
            }
            // frontmatter·HTML·링크 정의는 책에 보이지 않는다.
            Node::Yaml(_) | Node::Toml(_) | Node::Html(_) | Node::Definition(_) => {}
            other => {
                // 모르는 블록은 안의 블록을 그대로 꺼낸다.
                let inner = self.blocks(children(other));
                out.extend(inner);
            }
        }
    }

    fn inlines(&mut self, nodes: &[Node]) -> Rich {
        let mut out = Vec::new();
        for node in nodes {
            self.inline(node, Marks::default(), &mut out);
        }
        out
    }

    fn inline(&mut self, node: &Node, marks: Marks, out: &mut Rich) {
        let mut push = |text: &str, marks: Marks| {
            if text.is_empty() {
                return;
            }
            match out.last_mut() {
                Some(last) if last.marks == marks => last.text.push_str(text),
                _ => out.push(Inline {
                    text: text.to_string(),
                    marks,
                }),
            }
        };
        match node {
            // 기본은 문단 안 줄바꿈을 공백으로 바꾸고, 옵션을 켜면 유지한다.
            Node::Text(text) => {
                let normalized = text.value.replace("\r\n", "\n").replace('\r', "\n");
                if self.preserve_line_breaks {
                    push(&normalized, marks);
                } else {
                    push(&normalized.replace('\n', " "), marks);
                }
            }
            Node::Break(_) => push("\n", marks),
            Node::InlineCode(code) => push(
                &code.value,
                Marks {
                    code: true,
                    ..marks
                },
            ),
            Node::InlineMath(math) => push(
                &math.value,
                Marks {
                    code: true,
                    ..marks
                },
            ),
            Node::Image(image) => push(&image.alt, marks),
            Node::FootnoteReference(reference) => {
                let number = match self
                    .footnotes
                    .iter()
                    .position(|id| *id == reference.identifier)
                {
                    Some(ix) => ix + 1,
                    None => {
                        self.footnotes.push(reference.identifier.clone());
                        self.footnotes.len()
                    }
                };
                push(&superscript(number), marks);
            }
            // `<br>`은 강제 줄바꿈이다(제목에 두 줄을 넣을 때 쓴다). 다른 HTML은 버린다.
            Node::Html(html) => {
                if is_break(&html.value) {
                    push("\n", marks);
                }
            }
            other => {
                let marks = match other {
                    Node::Strong(_) => Marks {
                        strong: true,
                        ..marks
                    },
                    Node::Emphasis(_) => Marks {
                        emphasis: true,
                        ..marks
                    },
                    Node::Delete(_) => Marks {
                        strike: true,
                        ..marks
                    },
                    Node::Link(_) | Node::LinkReference(_) => Marks {
                        link: true,
                        ..marks
                    },
                    _ => marks,
                };
                for child in children(other) {
                    self.inline(child, marks, out);
                }
            }
        }
    }
}

/// 인라인 HTML이 줄바꿈 태그인지(`<br>`, `<br/>`, `<br />`).
fn is_break(html: &str) -> bool {
    matches!(
        html.trim().to_ascii_lowercase().as_str(),
        "<br>" | "<br/>" | "<br />"
    )
}

/// 각주 번호를 위 첨자 숫자로.
pub fn superscript(number: usize) -> String {
    const DIGITS: [char; 10] = ['⁰', '¹', '²', '³', '⁴', '⁵', '⁶', '⁷', '⁸', '⁹'];
    number
        .to_string()
        .chars()
        .map(|c| DIGITS[c.to_digit(10).unwrap_or(0) as usize])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markdown::{ImageAlign, ImageOptions};

    fn inline(text: &str, marks: Marks) -> Inline {
        Inline {
            text: text.into(),
            marks,
        }
    }

    #[test]
    fn line_groups_are_one_paragraph_with_line_breaks() {
        let saved = crate::text_transform::to_markdown(
            "앞\n:::\n# 제목 아님\n- 목록 아님\n**굵게**\n\n  들여쓰기\n1. 번호 아님\n---\n:::\n뒤",
        );
        let parsed = blocks(&saved);
        assert_eq!(parsed.len(), 3, "{parsed:?}");
        assert_eq!(parsed[0], Block::Paragraph(plain("앞")));
        assert_eq!(parsed[2], Block::Paragraph(plain("뒤")));
        let Block::Paragraph(group) = &parsed[1] else {
            panic!("{parsed:?}");
        };
        assert_eq!(
            rich_text(group),
            "# 제목 아님\n- 목록 아님\n굵게\n\n\u{a0}\u{a0}들여쓰기\n1. 번호 아님\n---"
        );
        assert!(group.iter().any(|inline| inline.text == "굵게" && inline.marks.strong));
        // 각주 참조, 묶음 끝의 빈 줄, 닫지 않은 묶음.
        let parsed = blocks(":::\n하나[^1]\n둘\n\n:::\n\n[^1]: 주석\n\n::: 시\n[셋]: 정의 아님");
        assert_eq!(parsed[0], Block::Paragraph(plain("하나¹\n둘")));
        assert_eq!(parsed[1], Block::Paragraph(plain("[셋]: 정의 아님")));
        assert!(matches!(parsed[2], Block::Footnotes(_)), "{parsed:?}");
    }

    #[test]
    fn line_groups_count_blank_lines_from_their_fences() {
        let saved = crate::text_transform::to_markdown("앞\n\n:::\nA\nB\n:::\n뒤");
        assert_eq!(
            manuscript_blocks(&saved, true),
            vec![
                Block::Paragraph(plain("앞")),
                Block::BlankLines(1),
                Block::Paragraph(plain("A\nB")),
                Block::Paragraph(plain("뒤")),
            ]
        );
    }

    #[test]
    fn manuscript_line_breaks_keep_paragraphs_and_blank_lines() {
        // 에디터의 각 줄은 문단으로 남는다.
        let saved = crate::text_transform::to_markdown("첫 문장\n둘째 문장");
        let expected = vec![
            Block::Paragraph(plain("첫 문장")),
            Block::Paragraph(plain("둘째 문장")),
        ];
        assert_eq!(manuscript_blocks(&saved, true), expected);
        assert_eq!(manuscript_blocks(&saved, false), expected);
        // 에디터의 빈 줄은 빈 줄 블록이 된다.
        for count in 1..=3 {
            let editor = format!("첫 문장{}둘째 문장", "\n".repeat(count + 1));
            let saved = crate::text_transform::to_markdown(&editor);
            assert_eq!(
                manuscript_blocks(&saved, true),
                vec![
                    Block::Paragraph(plain("첫 문장")),
                    Block::BlankLines(count),
                    Block::Paragraph(plain("둘째 문장")),
                ],
            );
        }
        // 이미지 줄은 이미지 블록으로 남는다.
        let saved = crate::text_transform::to_markdown("앞 문장\n![](a.png)\n뒤 문장");
        let blocks = manuscript_blocks(&saved, true);
        assert_eq!(blocks.len(), 3, "{blocks:?}");
        assert!(matches!(blocks[1], Block::Image(_)), "{blocks:?}");
        // 다른 앱에서 만든 문단 안 개행은 줄바꿈으로 유지한다.
        assert_eq!(
            manuscript_blocks("긴 문단의\n둘째 줄", true),
            vec![Block::Paragraph(plain("긴 문단의\n둘째 줄"))],
        );
    }

    #[test]
    fn headings_paragraphs_and_marks() {
        let blocks = blocks("## 제1장\n\n첫 **굵게** 문장\n이어짐\n\n둘째  \n줄\n");
        assert_eq!(
            blocks,
            vec![
                Block::Heading {
                    level: 2,
                    text: plain("제1장"),
                },
                Block::Paragraph(vec![
                    inline("첫 ", Marks::default()),
                    inline(
                        "굵게",
                        Marks {
                            strong: true,
                            ..Default::default()
                        }
                    ),
                    inline(" 문장 이어짐", Marks::default()),
                ]),
                Block::Paragraph(plain("둘째\n줄")),
            ]
        );
    }

    #[test]
    fn br_in_heading_is_a_line_break() {
        assert_eq!(
            blocks("# 제목<br>부제\n")[0],
            Block::Heading {
                level: 1,
                text: plain("제목\n부제"),
            }
        );
        // `<br/>`, `<br />`도 같고 다른 HTML은 버린다.
        for tag in ["<br/>", "<br />"] {
            assert_eq!(
                blocks(&format!("## 제목{tag}부제\n"))[0],
                Block::Heading {
                    level: 2,
                    text: plain("제목\n부제"),
                }
            );
        }
        assert_eq!(
            blocks("# 제목<em>부제\n")[0],
            Block::Heading {
                level: 1,
                text: plain("제목부제"),
            }
        );
    }

    #[test]
    fn image_paragraph_reads_attributes() {
        let blocks = blocks("![지도](../Extras/a.png){width=320px .align-left}\n");
        assert_eq!(
            blocks,
            vec![Block::Image(ImageLine {
                alt: "지도".into(),
                destination: "../Extras/a.png".into(),
                options: ImageOptions {
                    width: Some(320),
                    align: ImageAlign::Left,
                    ..Default::default()
                },
            })]
        );
    }

    #[test]
    fn lists_quotes_code_rules_and_tables() {
        let text = "> 인용\n\n3. 셋\n4. 넷\n\n```\nlet a = 1;\n```\n\n---\n\n| a | b |\n|---|---|\n| 1 | 2 |\n";
        let blocks = blocks(text);
        assert_eq!(
            blocks[0],
            Block::Quote(vec![Block::Paragraph(plain("인용"))])
        );
        assert_eq!(
            blocks[1],
            Block::List {
                ordered: true,
                start: 3,
                items: vec![
                    vec![Block::Paragraph(plain("셋"))],
                    vec![Block::Paragraph(plain("넷"))],
                ],
            }
        );
        assert_eq!(blocks[2], Block::Code("let a = 1;".into()));
        assert_eq!(blocks[3], Block::Rule);
        assert_eq!(
            blocks[4],
            Block::Table(vec![
                vec![plain("a"), plain("b")],
                vec![plain("1"), plain("2")],
            ])
        );
    }

    #[test]
    fn footnotes_are_numbered_and_collected() {
        let blocks = blocks("본문[^b] 그리고[^a].\n\n[^a]: 에이\n[^b]: 비\n");
        assert_eq!(blocks[0], Block::Paragraph(plain("본문¹ 그리고².")));
        assert_eq!(
            blocks[1],
            Block::Footnotes(vec![
                (1, vec![Block::Paragraph(plain("비"))]),
                (2, vec![Block::Paragraph(plain("에이"))]),
            ])
        );
    }

    #[test]
    fn frontmatter_is_skipped() {
        assert_eq!(
            blocks("---\ntitle: a\n---\n\n본문\n"),
            vec![Block::Paragraph(plain("본문"))]
        );
    }
}
