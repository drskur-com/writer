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
    let Ok(root) = markdown::to_mdast(text, &parse_options()) else {
        return Vec::new();
    };
    let mut cx = Walk {
        source: text,
        footnotes: Vec::new(),
        definitions: Vec::new(),
    };
    let mut out = cx.blocks(children(&root));
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

struct Walk<'a> {
    source: &'a str,
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
        for node in nodes {
            self.block(node, &mut out);
        }
        out
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
            // 문단 안 줄바꿈은 공백이다(ADR-0011).
            Node::Text(text) => push(&text.value.replace('\n', " "), marks),
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
            Node::Html(_) => {}
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
