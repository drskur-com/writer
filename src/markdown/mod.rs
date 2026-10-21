//! 마크다운 원문 → 스타일 스팬. 에디터 마크업 표시에 쓴다.
//!
//! gpui 타입을 쓰지 않는다. 스팬 → `HighlightStyle` 변환은 UI 쪽(`app/editor.rs`)에서 한다.
//! 프리뷰(`TextView::markdown`)와 해석이 어긋나지 않도록 같은 파서(markdown-rs)·같은 옵션을 쓴다.

mod book;

pub use book::{Block, Inline, Marks, Rich, manuscript_blocks, plain, rich_text};

use std::ops::Range;

use markdown::ParseOptions;
use markdown::mdast::Node;

/// 스팬 종류.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpanKind {
    /// 제목 전체(레벨 1~6).
    Heading(u8),
    Strong,
    Emphasis,
    Strikethrough,
    /// 인라인 코드·수식(백틱 포함).
    InlineCode,
    /// 링크 글자 부분(`[` `](…)` 제외).
    Link,
    /// 각주 참조의 이름(`[^3]`의 `3`). `[^`·`]`는 기호다.
    FootnoteRef,
    /// 인용 블록 전체.
    Quote,
    /// 코드·수식 블록 전체.
    CodeBlock,
    /// 구문 기호: `#`, `**`, `_`, `` ` ``, `[`, `](…)`, `>`, `-`, 펜스, 줄 묶음 펜스, 구분선, frontmatter, 이미지,
    /// 각주 `[^`·`]`·정의 머리.
    Marker,
}

/// 원문의 UTF-8 바이트 범위에 붙는 스타일.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub range: Range<usize>,
    pub kind: SpanKind,
}

/// 프리뷰(`TextView::markdown`)와 같은 파서 옵션.
fn parse_options() -> ParseOptions {
    let mut options = ParseOptions::gfm();
    options.constructs.frontmatter = true;
    options.constructs.math_text = true;
    options.constructs.math_flow = true;
    options
}

/// 원문 전체를 파싱해 스팬을 만든다. 범위는 바이트 offset이고 바깥 노드가 먼저 온다.
pub fn spans(text: &str) -> Vec<Span> {
    let mut out = Vec::new();
    // MDX를 켜지 않으면 파싱은 실패하지 않는다.
    if let Ok(root) = markdown::to_mdast(text, &parse_options()) {
        walk(&root, text, false, &mut out);
    }
    push_group_fences(text, &mut out);
    out
}

/// 줄 묶음(`:::`)의 여는 줄·닫는 줄을 기호로 표시한다.
fn push_group_fences(text: &str, out: &mut Vec<Span>) {
    let groups = crate::text_transform::line_groups(text);
    if groups.is_empty() {
        return;
    }
    let mut start = 0;
    for (ix, line) in text.split('\n').enumerate() {
        let range = start..start + line.len();
        start = range.end + 1;
        let fence = groups.iter().any(|group| group.is_fence(ix));
        if fence && !range.is_empty() {
            out.push(Span {
                range,
                kind: SpanKind::Marker,
            });
        }
    }
}

/// 이미지 정렬. 기본은 가운데라 마크다운에 따로 적지 않는다.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ImageAlign {
    Left,
    #[default]
    Center,
    Right,
}

/// 이미지 표시 옵션. 마크다운 뒤에 Pandoc 속성(`{width=320px .align-left}`)으로 붙인다(ADR-0007).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImageOptions {
    /// 렌더링 크기(px). `None`이면 자동.
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub align: ImageAlign,
    /// 전용 페이지에 최대한 크게 놓는다. 크기·정렬보다 앞선다.
    pub full_page: bool,
}

impl ImageOptions {
    /// Pandoc 속성 블록 안쪽(`width=320px .align-left`)을 읽는다. 모르는 속성은 무시한다.
    fn parse(attributes: &str) -> Self {
        let mut options = Self::default();
        let size = |value: &str| {
            let value = value.trim_matches('"');
            value.strip_suffix("px").unwrap_or(value).parse().ok()
        };
        for attribute in attributes.split_whitespace() {
            match attribute {
                ".align-left" => options.align = ImageAlign::Left,
                ".align-right" => options.align = ImageAlign::Right,
                ".full-page" => options.full_page = true,
                _ => {
                    if let Some(value) = attribute.strip_prefix("width=") {
                        options.width = size(value);
                    } else if let Some(value) = attribute.strip_prefix("height=") {
                        options.height = size(value);
                    }
                }
            }
        }
        options
    }

    /// Pandoc 속성 블록. 기본값뿐이면 빈 문자열.
    fn attributes(&self) -> String {
        let mut parts = Vec::new();
        if self.full_page {
            parts.push(".full-page".to_string());
        } else {
            parts.extend(self.width.map(|width| format!("width={width}px")));
            parts.extend(self.height.map(|height| format!("height={height}px")));
            match self.align {
                ImageAlign::Left => parts.push(".align-left".into()),
                ImageAlign::Center => {}
                ImageAlign::Right => parts.push(".align-right".into()),
            }
        }
        if parts.is_empty() {
            String::new()
        } else {
            format!("{{{}}}", parts.join(" "))
        }
    }
}

/// 이미지 마크다운(`![캡션](경로){속성}`)을 만든다.
///
/// 캡션의 `[`·`]`는 이스케이프하고, 경로에 공백·괄호가 있으면 `<…>`로 감싼다.
pub fn image(alt: &str, destination: &str, options: &ImageOptions) -> String {
    let mut escaped = String::new();
    for c in alt.trim().chars() {
        if matches!(c, '[' | ']' | '\\') {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    let destination = destination.trim();
    let attributes = options.attributes();
    if destination.contains(|c: char| c.is_whitespace() || matches!(c, '(' | ')')) {
        format!("![{escaped}](<{destination}>){attributes}")
    } else {
        format!("![{escaped}]({destination}){attributes}")
    }
}

/// 이미지만 있는 줄(`![캡션](경로 "제목"){속성}`)의 내용.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageLine {
    pub alt: String,
    pub destination: String,
    pub options: ImageOptions,
}

/// 한 줄이 이미지 하나뿐이면(앞 들여쓰기 3칸·뒤 공백 허용) 그 내용을 돌려준다. 줄 텍스트만 본다.
///
/// 에디터가 줄마다 즉시 판정해 이미지를 그릴 수 있도록 파서를 쓰지 않는다(`heading_level`과 같은
/// 이유). 그래서 코드 블록 안의 이미지 줄도 이미지로 본다. 속성은 ADR-0007 문법만 읽고 모르는
/// 속성은 무시한다.
pub fn image_line(line: &str) -> Option<ImageLine> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 {
        return None;
    }
    let rest = line[indent..].trim_end().strip_prefix("![")?;

    // 캡션: 짝이 맞는 `]`까지. `\`는 다음 글자를 그대로 둔다.
    let mut alt = String::new();
    let mut depth = 0;
    let mut chars = rest.char_indices();
    let alt_end = loop {
        let (ix, c) = chars.next()?;
        match c {
            '\\' => {
                let (_, next) = chars.next()?;
                if !matches!(next, '[' | ']' | '\\') {
                    alt.push('\\');
                }
                alt.push(next);
            }
            '[' => {
                depth += 1;
                alt.push(c);
            }
            ']' if depth == 0 => break ix,
            ']' => {
                depth -= 1;
                alt.push(c);
            }
            _ => alt.push(c),
        }
    };
    let rest = rest[alt_end + 1..].strip_prefix('(')?.trim_start();

    // 경로: `<…>` 또는 공백·짝 없는 `)` 앞까지.
    let (destination, rest) = if let Some(inner) = rest.strip_prefix('<') {
        let end = inner.find('>')?;
        (&inner[..end], &inner[end + 1..])
    } else {
        let mut depth = 0usize;
        let end = rest
            .char_indices()
            .find(|&(_, c)| match c {
                '(' => {
                    depth += 1;
                    false
                }
                ')' if depth == 0 => true,
                ')' => {
                    depth -= 1;
                    false
                }
                c => c.is_whitespace(),
            })
            .map_or(rest.len(), |(ix, _)| ix);
        (&rest[..end], &rest[end..])
    };
    if destination.is_empty() {
        return None;
    }

    // 제목(`"…"`·`'…'`·`(…)`)은 건너뛴다.
    let rest = rest.trim_start();
    let rest = match rest.chars().next()? {
        ')' => rest,
        open @ ('"' | '\'' | '(') => {
            let close = if open == '(' { ')' } else { open };
            let end = rest[1..].find(close)?;
            rest[end + 2..].trim_start()
        }
        _ => return None,
    };
    let rest = rest.strip_prefix(')')?;

    let options = if rest.is_empty() {
        ImageOptions::default()
    } else {
        let attributes = rest.strip_prefix('{')?.strip_suffix('}')?;
        ImageOptions::parse(attributes)
    };
    Some(ImageLine {
        alt,
        destination: destination.to_string(),
        options,
    })
}

/// 한 줄이 ATX 제목(`#`~`######`)이면 레벨을 돌려준다. 줄 텍스트만 본다.
///
/// 에디터가 줄마다 즉시 판정할 수 있도록 파서를 쓰지 않는다. 그래서 코드 블록 안의
/// `# 주석` 줄도 제목으로 본다. setext 제목(`===` 밑줄)은 아랫줄이 윗줄을 바꾸므로 제외한다.
pub fn heading_level(line: &str) -> Option<u8> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 {
        return None;
    }
    let rest = &line[indent..];
    let level = rest.len() - rest.trim_start_matches('#').len();
    if !(1..=6).contains(&level) {
        return None;
    }
    match rest[level..].chars().next() {
        None | Some(' ' | '\t') => Some(level as u8),
        _ => None,
    }
}

/// ATX 제목 줄의 여는 기호(들여쓰기 + `#`들 + 뒤 공백) 길이. 제목이 아니면 0.
pub fn heading_marker_len(line: &str) -> usize {
    let Some(level) = heading_level(line) else {
        return 0;
    };
    let open = line.len() - line.trim_start_matches(' ').len() + usize::from(level);
    line.len() - line[open..].trim_start_matches([' ', '\t']).len()
}

/// 줄의 제목 레벨을 바꾼다. `None`이면 제목 기호를 떼어 문단으로 만든다.
pub fn set_heading_level(line: &str, level: Option<u8>) -> String {
    let body = &line[heading_marker_len(line)..];
    match level {
        Some(level) => format!("{} {body}", "#".repeat(usize::from(level))),
        None => body.to_string(),
    }
}

/// ATX 제목 줄의 스팬(제목 전체 + 여는·닫는 `#`). 줄 텍스트만 보는 `heading_level`과 같은
/// 판정이라, 에디터가 줄 크기를 바꾸는 시점과 똑같이 키 입력마다 즉시 계산할 수 있다.
pub fn heading_spans(text: &str) -> Vec<Span> {
    let mut out = Vec::new();
    let mut line_start = 0;
    for line in text.split_inclusive('\n') {
        let start = line_start;
        line_start += line.len();
        let line = line.strip_suffix('\n').unwrap_or(line);
        let line = line.strip_suffix('\r').unwrap_or(line);
        let Some(level) = heading_level(line) else {
            continue;
        };
        out.push(Span {
            range: start..start + line.len(),
            kind: SpanKind::Heading(level),
        });
        let open = heading_marker_len(line);
        out.push(Span {
            range: start..start + open,
            kind: SpanKind::Marker,
        });
        // 닫는 기호: 공백 뒤(또는 본문 없이) 끝에 오는 `#`들과 그 앞뒤 공백.
        let body = line[open..].trim_end_matches([' ', '\t']);
        let closing = body.trim_end_matches('#');
        if closing.len() < body.len() && (closing.is_empty() || closing.ends_with([' ', '\t'])) {
            let close = open + closing.trim_end_matches([' ', '\t']).len();
            out.push(Span {
                range: start + close..start + line.len(),
                kind: SpanKind::Marker,
            });
        }
    }
    out
}

/// 제목 원문이 ATX(`#`로 시작)인지.
fn is_atx(source: &str) -> bool {
    source.trim_start_matches(' ').starts_with('#')
}

fn walk(node: &Node, text: &str, in_quote: bool, out: &mut Vec<Span>) {
    let mut in_quote = in_quote;
    if let Some(range) = range_of(node) {
        let mut push = |range: Range<usize>, kind| {
            if !range.is_empty() {
                out.push(Span { range, kind });
            }
        };
        match node {
            // ATX 제목은 줄 단위로 즉시 판정한다(`heading_spans`). 여기서는 setext만.
            Node::Heading(heading) if !is_atx(&text[range.clone()]) => {
                push(range.clone(), SpanKind::Heading(heading.depth));
                push_around_children(node, range, out);
            }
            Node::Strong(_) => {
                push(range.clone(), SpanKind::Strong);
                push_around_children(node, range, out);
            }
            Node::Emphasis(_) => {
                push(range.clone(), SpanKind::Emphasis);
                push_around_children(node, range, out);
            }
            Node::Delete(_) => {
                push(range.clone(), SpanKind::Strikethrough);
                push_around_children(node, range, out);
            }
            Node::Link(_) | Node::LinkReference(_) => {
                push(
                    children_range(node).unwrap_or(range.clone()),
                    SpanKind::Link,
                );
                push_around_children(node, range, out);
            }
            Node::InlineCode(_) | Node::InlineMath(_) => {
                push(range.clone(), SpanKind::InlineCode);
                let source = &text[range.clone()];
                let fence = |c: char| c == '`' || c == '$';
                let open = source.len() - source.trim_start_matches(fence).len();
                let close = source.len() - source.trim_end_matches(fence).len();
                push(range.start..range.start + open, SpanKind::Marker);
                push(range.end - close..range.end, SpanKind::Marker);
            }
            Node::Code(_) | Node::Math(_) => {
                push(range.clone(), SpanKind::CodeBlock);
                push_fences(text, range, out);
            }
            Node::Blockquote(_) => {
                push(range.clone(), SpanKind::Quote);
                // 중첩 인용의 `>`는 바깥 인용이 줄 머리에서 한꺼번에 표시한다.
                if !in_quote {
                    push_quote_markers(text, range, out);
                }
                in_quote = true;
            }
            Node::ListItem(_) => {
                // 글머리표·번호·체크박스: 항목 시작 ~ 첫 자식 시작.
                let end = children_range(node).map_or(range.end, |inner| inner.start);
                push(range.start..end, SpanKind::Marker);
            }
            // 바로 뒤 Pandoc 속성(`{width=320px}`, ADR-0007)도 기호다.
            Node::Image(_) => {
                let rest = &text[range.end..];
                let line = &rest[..rest.find('\n').unwrap_or(rest.len())];
                let attributes = line
                    .starts_with('{')
                    .then(|| line.find('}'))
                    .flatten()
                    .map_or(0, |end| end + 1);
                push(range.start..range.end + attributes, SpanKind::Marker);
            }
            Node::FootnoteReference(_) if range.len() > 3 => {
                push(range.start..range.start + 2, SpanKind::Marker);
                push(range.start + 2..range.end - 1, SpanKind::FootnoteRef);
                push(range.end - 1..range.end, SpanKind::Marker);
            }
            // 정의 머리 `[^3]:`.
            Node::FootnoteDefinition(_) => {
                if let Some(end) = text[range.clone()].find("]:") {
                    push(range.start..range.start + end + 2, SpanKind::Marker);
                }
            }
            Node::ThematicBreak(_)
            | Node::ImageReference(_)
            | Node::FootnoteReference(_)
            | Node::Yaml(_)
            | Node::Toml(_) => push(range, SpanKind::Marker),
            _ => {}
        }
    }
    if let Some(children) = node.children() {
        for child in children {
            walk(child, text, in_quote, out);
        }
    }
}

fn range_of(node: &Node) -> Option<Range<usize>> {
    node.position()
        .map(|position| position.start.offset..position.end.offset)
}

/// 첫 자식 시작 ~ 마지막 자식 끝.
fn children_range(node: &Node) -> Option<Range<usize>> {
    let children = node.children()?;
    let start = range_of(children.first()?)?.start;
    let end = range_of(children.last()?)?.end;
    Some(start..end)
}

/// 노드 범위에서 자식 범위를 뺀 앞뒤를 기호로 표시한다. 자식이 없으면 전체가 기호다(`#` 단독 등).
fn push_around_children(node: &Node, range: Range<usize>, out: &mut Vec<Span>) {
    let inner = children_range(node).unwrap_or(range.end..range.end);
    for marker in [range.start..inner.start, inner.end..range.end] {
        if !marker.is_empty() {
            out.push(Span {
                range: marker,
                kind: SpanKind::Marker,
            });
        }
    }
}

/// 펜스 코드 블록의 여는 줄·닫는 줄을 기호로 표시한다. 들여쓴 코드 블록은 펜스가 없다.
fn push_fences(text: &str, range: Range<usize>, out: &mut Vec<Span>) {
    let is_fence = |line: &str| {
        let line = line.trim_start();
        line.starts_with("```") || line.starts_with("~~~") || line.starts_with("$$")
    };
    let source = &text[range.clone()];
    let first_end = source.find('\n').unwrap_or(source.len());
    if !is_fence(&source[..first_end]) {
        return;
    }
    out.push(Span {
        range: range.start..range.start + first_end,
        kind: SpanKind::Marker,
    });
    if first_end < source.len() {
        let last_start = source.rfind('\n').map_or(0, |ix| ix + 1);
        if is_fence(&source[last_start..]) {
            out.push(Span {
                range: range.start + last_start..range.end,
                kind: SpanKind::Marker,
            });
        }
    }
}

/// 인용 블록 각 줄 머리의 `>`(중첩 `> >` 포함)를 기호로 표시한다.
fn push_quote_markers(text: &str, range: Range<usize>, out: &mut Vec<Span>) {
    let mut line_start = range.start;
    for line in text[range.clone()].split_inclusive('\n') {
        let mut end = 0;
        for (ix, c) in line.char_indices() {
            match c {
                '>' => end = ix + 1,
                ' ' | '\t' => {}
                _ => break,
            }
        }
        if end > 0 {
            out.push(Span {
                range: line_start..line_start + end,
                kind: SpanKind::Marker,
            });
        }
        line_start += line.len();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_escapes_alt_and_wraps_odd_paths() {
        let auto = ImageOptions::default();
        assert_eq!(image(" 지도 ", "a/b.png", &auto), "![지도](a/b.png)");
        assert_eq!(
            image("[1]", "https://x.io/a.png", &auto),
            "![\\[1\\]](https://x.io/a.png)"
        );
        assert_eq!(
            image("", "내 사진 (1).png", &auto),
            "![](<내 사진 (1).png>)"
        );
    }

    #[test]
    fn image_options_become_pandoc_attributes() {
        let sized = ImageOptions {
            width: Some(320),
            height: Some(200),
            align: ImageAlign::Left,
            full_page: false,
        };
        assert_eq!(
            image("", "a.png", &sized),
            "![](a.png){width=320px height=200px .align-left}"
        );
        // 한 페이지 가득이면 크기·정렬은 적지 않는다.
        let full = ImageOptions {
            full_page: true,
            ..sized
        };
        assert_eq!(image("표지", "a.png", &full), "![표지](a.png){.full-page}");
    }

    #[test]
    fn image_line_reads_back_what_image_writes() {
        let options = ImageOptions {
            width: Some(320),
            height: None,
            align: ImageAlign::Right,
            full_page: false,
        };
        for (alt, destination) in [
            ("지도", "Extras/Attachments/a.png"),
            ("[1] 표", "https://x.io/a.png?b=(1)"),
            ("", "내 사진 (1).png"),
        ] {
            let line = image(alt, destination, &options);
            assert_eq!(
                image_line(&line),
                Some(ImageLine {
                    alt: alt.into(),
                    destination: destination.into(),
                    options,
                }),
                "{line}"
            );
        }
        let full = image_line("![](a.png){.full-page}").unwrap();
        assert!(full.options.full_page);
    }

    #[test]
    fn image_line_only_matches_image_paragraph_lines() {
        let plain = image_line("  ![a](b.png \"제목\")  ").unwrap();
        assert_eq!(plain.destination, "b.png");
        assert_eq!(plain.options, ImageOptions::default());
        // 크기 단위가 없어도 px로 본다. 모르는 속성은 무시한다.
        let sized = image_line("![](a.png){width=10 height=\"20px\" .wide #id}").unwrap();
        assert_eq!(
            (sized.options.width, sized.options.height),
            (Some(10), Some(20))
        );
        for line in [
            "글 ![a](b.png)",
            "![a](b.png) 글",
            "![a](b.png){width=1",
            "![a]()",
            "![a](b.png",
            "    ![a](b.png)",
            "[a](b.png)",
        ] {
            assert_eq!(image_line(line), None, "{line}");
        }
    }

    #[test]
    fn image_and_attributes_are_markers() {
        let text = "![캡션](a.png){width=320px}\n\n![b](c.png) 끝";
        assert_eq!(
            pieces(text, SpanKind::Marker),
            ["![캡션](a.png){width=320px}", "![b](c.png)"]
        );
    }

    /// 종류별로 스팬이 덮는 원문 조각을 모은다.
    fn pieces(text: &str, kind: SpanKind) -> Vec<&str> {
        spans(text)
            .into_iter()
            .filter(|span| span.kind == kind)
            .map(|span| &text[span.range])
            .collect()
    }

    #[test]
    fn heading_markers_and_byte_offsets() {
        /// 종류별로 줄 단위 제목 스팬이 덮는 조각.
        fn heading_pieces(text: &str, kind: SpanKind) -> Vec<&str> {
            heading_spans(text)
                .into_iter()
                .filter(|span| span.kind == kind)
                .map(|span| &text[span.range])
                .collect()
        }
        // 한글이 섞여도 바이트 offset으로 잘린다
        let text = "본문\n# 제목 #\n\n본문";
        assert_eq!(heading_pieces(text, SpanKind::Heading(1)), ["# 제목 #"]);
        assert_eq!(heading_pieces(text, SpanKind::Marker), ["# ", " #"]);
        assert_eq!(heading_pieces("### 셋", SpanKind::Heading(3)), ["### 셋"]);
        // `#`가 끝에 붙은 단어는 닫는 기호가 아니다
        assert_eq!(heading_pieces("## C#\r\n", SpanKind::Marker), ["## "]);
        // 빈 제목은 전체가 기호
        assert_eq!(heading_pieces("#\n", SpanKind::Marker), ["#"]);
        assert_eq!(heading_pieces("## ##", SpanKind::Marker), ["## ", "##"]);
        // 파서 경로는 ATX 제목을 건너뛰고, setext 제목과 밑줄만 낸다
        assert!(pieces("# 제목", SpanKind::Heading(1)).is_empty());
        assert_eq!(pieces("제목\n===\n", SpanKind::Heading(1)), ["제목\n==="]);
        assert_eq!(pieces("제목\n===\n", SpanKind::Marker), ["\n==="]);
    }

    #[test]
    fn set_heading_level_replaces_the_marker() {
        assert_eq!(set_heading_level("문단", Some(2)), "## 문단");
        assert_eq!(set_heading_level("# 부 제목", Some(3)), "### 부 제목");
        assert_eq!(set_heading_level("  ##\t장", None), "장");
        assert_eq!(set_heading_level("#해시태그", Some(1)), "# #해시태그");
        assert_eq!(set_heading_level("", Some(1)), "# ");
        assert_eq!(heading_marker_len("## 장"), 3);
        assert_eq!(heading_marker_len("문단"), 0);
    }

    #[test]
    fn heading_level_from_line() {
        assert_eq!(heading_level("# 제목"), Some(1));
        assert_eq!(heading_level("###### 여섯"), Some(6));
        assert_eq!(heading_level("   ## 들여쓰기"), Some(2));
        assert_eq!(heading_level("#"), Some(1));
        assert_eq!(heading_level("#\t탭"), Some(1));
        assert_eq!(heading_level("####### 일곱"), None);
        assert_eq!(heading_level("#태그"), None);
        assert_eq!(heading_level("    # 코드"), None);
        assert_eq!(heading_level("본문 # 아님"), None);
    }

    #[test]
    fn inline_styles() {
        let text = "가 **굵게** _기울임_ ~~취소~~ `코드` 나";
        assert_eq!(pieces(text, SpanKind::Strong), ["**굵게**"]);
        assert_eq!(pieces(text, SpanKind::Emphasis), ["_기울임_"]);
        assert_eq!(pieces(text, SpanKind::Strikethrough), ["~~취소~~"]);
        assert_eq!(pieces(text, SpanKind::InlineCode), ["`코드`"]);
        assert_eq!(
            pieces(text, SpanKind::Marker),
            ["**", "**", "_", "_", "~~", "~~", "`", "`"]
        );
    }

    #[test]
    fn links() {
        let text = "[링크](https://a.b) <https://c.d>";
        assert_eq!(pieces(text, SpanKind::Link), ["링크", "https://c.d"]);
        assert_eq!(
            pieces(text, SpanKind::Marker),
            ["[", "](https://a.b)", "<", ">"]
        );
        // GFM 맨 주소는 기호가 없다
        assert!(pieces("https://e.f 끝", SpanKind::Marker).is_empty());
    }

    #[test]
    fn blocks() {
        let text = "> 인용\n> > 중첩\n\n- 항목\n1. 번호\n- [ ] 할 일\n\n---\n";
        assert_eq!(pieces(text, SpanKind::Quote).len(), 2);
        let markers = pieces(text, SpanKind::Marker);
        assert_eq!(markers, [">", "> >", "- ", "1. ", "- [ ] ", "---"]);
    }

    #[test]
    fn code_blocks() {
        let text = "```rust\nlet a = 1;\n```\n\n    들여쓴 코드\n";
        assert_eq!(pieces(text, SpanKind::CodeBlock).len(), 2);
        assert_eq!(pieces(text, SpanKind::Marker), ["```rust", "```"]);
        // 닫히지 않은 펜스는 여는 줄만
        assert_eq!(pieces("```\n코드", SpanKind::Marker), ["```"]);
    }

    #[test]
    fn footnotes() {
        // 에디터 본문: 줄 하나가 문단이라 정의가 이어진 줄에 있다.
        let text = "본문[^1] 둘[^note].\n\n[^1]: 하나\n[^note]: 이름";
        assert_eq!(pieces(text, SpanKind::FootnoteRef), ["1", "note"]);
        assert_eq!(
            pieces(text, SpanKind::Marker),
            ["[^", "]", "[^", "]", "[^1]:", "[^note]:"]
        );
    }

    #[test]
    fn frontmatter_is_marker() {
        let text = "---\ntitle: 가\n---\n\n본문";
        assert_eq!(pieces(text, SpanKind::Marker), ["---\ntitle: 가\n---"]);
    }
}
