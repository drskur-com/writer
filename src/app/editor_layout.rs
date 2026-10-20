//! 에디터 줄 배치: 책 스타일(ADR-0009)을 줄 단위 [`LineStyle`]로 옮긴다(ADR-0010).
//!
//! 에디터에서는 줄 하나가 문단이다(ADR-0011). 마크다운 기호는 그대로 두고(흐리게), 문단과 제목의
//! 모양만 내보낸 책에 맞춘다.
//! - 문단 첫 줄 들여쓰기, 정렬(양쪽 포함), 줄 간격
//! - 제목 크기·정렬, 문단·제목 사이 여백
//!
//! 블록 사이 여백은 앞 줄 아래에 둔다. 두 블록 사이 여백은 앞 블록의 뒤 여백과 뒤 블록의 앞 여백 중
//! 큰 값이다. 빈 줄은 평범한 빈 행으로 남는다.

use gpui_base::input::{LineAlign, LineContext, LineStyle};
use gpui_kit::gpui::{Pixels, px};

use crate::library::{self, Align, ResolvedStyle, StyleTarget};
use crate::markdown;

/// 제목 레벨(H1~H6)별 글자 크기 배율(본문 대비). H1~H3은 책 스타일의 부·장·절 제목 크기를 쓴다.
const HEADING_SCALES: [f32; 6] = [1.6, 1.4, 1.25, 1.15, 1.05, 1.0];

/// 줄 하나가 이루는 블록 종류.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Block {
    /// 빈 줄(공백만 있어도).
    Blank,
    Heading(u8),
    /// 이미지 줄(ADR-0008). 이미지 블록은 따로 그린다.
    Image,
    /// 목록·인용·코드·표처럼 문단 모양(들여쓰기·정렬)을 입히지 않는 줄.
    Other,
    /// 문단 줄.
    Text,
}

impl Block {
    fn of(line: &str) -> Block {
        if line.trim().is_empty() {
            return Block::Blank;
        }
        if let Some(level) = markdown::heading_level(line) {
            return Block::Heading(level);
        }
        if markdown::image_line(line).is_some() {
            return Block::Image;
        }
        let rest = line.trim_start();
        let indent = line.len() - rest.len();
        let marker = rest.split_once(' ').map_or(rest, |(marker, _)| marker);
        let list = matches!(marker, "-" | "*" | "+")
            || marker.strip_suffix(['.', ')']).is_some_and(|number| {
                !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit())
            });
        let other = indent >= 4
            || list
            || rest.starts_with('>')
            || rest.starts_with("```")
            || rest.starts_with("~~~")
            || rest.starts_with('|');
        if other { Block::Other } else { Block::Text }
    }

    /// 블록 모양을 정하는 스타일 대상. 부·장·절이 아닌 제목은 없다.
    fn target(self) -> Option<StyleTarget> {
        match self {
            Block::Heading(level) => StyleTarget::for_heading(level),
            Block::Text => Some(StyleTarget::Body),
            _ => None,
        }
    }
}

/// 책 스타일을 에디터 줄 스타일로 옮기는 규칙. 시트를 열 때와 스타일이 바뀔 때 만든다.
#[derive(Clone)]
pub(super) struct BookLayout {
    /// 본문 글자 크기(px). em 단위 값의 기준.
    font_size: f32,
    body: ResolvedStyle,
    part: ResolvedStyle,
    chapter: ResolvedStyle,
    section: ResolvedStyle,
}

impl BookLayout {
    pub(super) fn new(styles: &library::Styles, font_size: f32) -> Self {
        Self {
            font_size,
            body: styles.resolve(StyleTarget::Body),
            part: styles.resolve(StyleTarget::Part),
            chapter: styles.resolve(StyleTarget::Chapter),
            section: styles.resolve(StyleTarget::Section),
        }
    }

    fn style(&self, target: StyleTarget) -> &ResolvedStyle {
        match target {
            StyleTarget::Body => &self.body,
            StyleTarget::Part => &self.part,
            StyleTarget::Chapter => &self.chapter,
            StyleTarget::Section => &self.section,
        }
    }

    fn em(&self, em: f64) -> Pixels {
        px(self.font_size * em as f32)
    }

    /// 블록의 (앞 여백, 뒤 여백). 스타일 대상이 없는 블록은 문단과 같다.
    fn spacing(&self, block: Block) -> (f64, f64) {
        let style = block
            .target()
            .map_or(&self.body, |target| self.style(target));
        (style.space_before, style.space_after)
    }

    /// 두 블록 사이 여백(px): 앞 블록의 뒤 여백과 뒤 블록의 앞 여백 중 큰 값.
    fn gap(&self, before: Block, after: Block) -> Pixels {
        self.em(self.spacing(before).1.max(self.spacing(after).0))
    }

    /// 줄과 다음 줄을 보고 줄 스타일을 정한다. 줄 하나가 문단이다(ADR-0011).
    pub(super) fn line_style(&self, line: LineContext<'_>) -> LineStyle {
        let block = Block::of(line.text);
        let space_after = self.gap_after(block, line.next.map(Block::of));
        match block {
            // 빈 줄은 평범한 빈 행이다. 마크다운 렌더링에서는 사라지지만 쓰는 화면에는 남긴다.
            Block::Blank => LineStyle::default(),
            // 이미지 줄은 커서가 있을 때만 글자가 보인다(ADR-0008).
            Block::Image => LineStyle {
                collapsed: true,
                ..Default::default()
            },
            Block::Heading(level) => {
                let style = StyleTarget::for_heading(level).map(|target| self.style(target));
                let text_scale = style.map_or(HEADING_SCALES[usize::from(level) - 1], |style| {
                    style.font_size as f32
                });
                let line_height = style.map_or(self.body.line_height, |style| style.line_height);
                LineStyle {
                    text_scale,
                    height_scale: (text_scale * (line_height / self.body.line_height) as f32)
                        .max(1.0),
                    align: style.map_or(LineAlign::Start, |style| align(style.align)),
                    first_indent: style.map_or(px(0.), |style| self.em(style.first_line_indent)),
                    space_after,
                    ..Default::default()
                }
            }
            Block::Text => LineStyle {
                align: align(self.body.align),
                first_indent: self.em(self.body.first_line_indent),
                space_after,
                ..Default::default()
            },
            Block::Other => LineStyle {
                space_after,
                ..Default::default()
            },
        }
    }

    /// 이 줄 아래 여백. 빈 줄 앞에서는 문단 뒤 여백을 두고(빈 행은 그 아래에 이어진다),
    /// 목록·인용·코드 줄끼리는 붙인다.
    fn gap_after(&self, block: Block, next: Option<Block>) -> Pixels {
        match next {
            None => px(0.),
            Some(Block::Blank) => self.gap(block, Block::Text),
            Some(Block::Other) if block == Block::Other => px(0.),
            Some(next) => self.gap(block, next),
        }
    }
}

fn align(align: Align) -> LineAlign {
    match align {
        Align::Left => LineAlign::Start,
        Align::Center => LineAlign::Center,
        Align::Right => LineAlign::End,
        Align::Justify => LineAlign::Justify,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context<'a>(prev: Option<&'a str>, text: &'a str, next: Option<&'a str>) -> LineContext<'a> {
        LineContext { text, prev, next }
    }

    fn layout() -> BookLayout {
        let mut styles = library::Styles::default();
        styles.body.first_line_indent = Some(1.0);
        styles.body.space_after = Some(0.5);
        styles.chapter.space_before = Some(3.0);
        styles.chapter.space_after = Some(1.0);
        BookLayout::new(&styles, 10.0)
    }

    #[test]
    fn every_line_is_an_indented_justified_paragraph() {
        let layout = layout();
        for prev in [None, Some(""), Some("앞 문단")] {
            let style = layout.line_style(context(prev, "문단", Some("다음 문단")));
            assert_eq!(style.first_indent, px(10.));
            assert_eq!(style.align, LineAlign::Justify);
            assert_eq!(style.space_after, px(5.));
        }
    }

    #[test]
    fn blank_lines_are_plain_rows_after_the_paragraph_gap() {
        let layout = layout();
        let blank = layout.line_style(context(Some("문단"), "", Some("## 장")));
        assert_eq!(blank, LineStyle::default());
        let before_blank = layout.line_style(context(None, "문단", Some("")));
        assert_eq!(before_blank.space_after, px(5.));
    }

    #[test]
    fn gaps_take_the_larger_side() {
        let layout = layout();
        let heading = layout.line_style(context(Some(""), "## 장", Some("문단")));
        assert_eq!(heading.space_after, px(10.));
        assert_eq!(heading.align, LineAlign::Center);
        let text = layout.line_style(context(Some(""), "문단", Some("## 장")));
        assert_eq!(text.space_after, px(30.));
        let item = layout.line_style(context(None, "- 항목", Some("- 다음 항목")));
        assert_eq!(item.space_after, px(0.));
    }

    #[test]
    fn lists_quotes_and_code_keep_plain_layout() {
        let layout = layout();
        for line in ["- 항목", "1. 항목", "> 인용", "```rust", "    코드"] {
            let style = layout.line_style(context(Some(""), line, None));
            assert_eq!(style.first_indent, px(0.), "{line}");
            assert_eq!(style.align, LineAlign::Start, "{line}");
        }
    }
}
