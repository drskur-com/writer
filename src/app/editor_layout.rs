//! 에디터 줄 배치: 책 스타일(ADR-0009)을 줄 단위 [`LineStyle`]로 옮긴다(ADR-0010).
//!
//! 에디터에서는 줄 하나가 문단이다(ADR-0011). 마크다운 기호는 그대로 두고(흐리게), 문단과 제목의
//! 모양만 내보낸 책에 맞춘다.
//! - 문단 첫 줄 들여쓰기, 정렬(양쪽 포함), 줄 간격
//! - 제목 크기·정렬, 문단·제목 사이 여백
//!
//! 프로젝트 밖 시트(메모)는 책 스타일을 쓰지 않는다([`BookLayout::plain`]): 왼쪽 정렬, 들여쓰기·블록
//! 여백 없음, 제목은 레벨별 기본 크기.
//!
//! 책 스타일이 새 페이지를 여는 곳에는 줄 아래 블록 몫을 잡아 "페이지가 나뉩니다" 표시를 그린다.
//! 새 페이지에서 시작하는 제목(별도 제목 페이지 포함)의 앞 줄 아래, 별도 제목 페이지인 제목 줄 아래다.
//! 시트 첫 줄이 그런 제목이면 문서 위(첫 줄 위 블록, `LineStyle::block_before`)에 그린다.
//!
//! 블록 사이 여백은 앞 줄 아래에 둔다. 두 블록 사이 여백은 앞 블록의 뒤 여백과 뒤 블록의 앞 여백 중
//! 큰 값이다. 빈 줄은 평범한 빈 행으로 남는다.

use gpui_base::input::{LineAlign, LineContext, LineStyle};
use gpui_kit::gpui::{Pixels, px};

use crate::library::{self, Align, Placement, ResolvedStyle, StyleTarget};
use crate::markdown;

/// 제목 레벨(H1~H6)별 글자 크기 배율(본문 대비). 책 스타일이 있으면 H1~H3은 부·장·절 제목 크기를 쓴다.
const HEADING_SCALES: [f32; 6] = [1.6, 1.4, 1.25, 1.15, 1.05, 1.0];

/// "페이지가 나뉩니다" 표시 높이(px). 줄 아래, 측정한 블록(이미지) 밑에 둔다.
pub(super) const PAGE_BREAK_HEIGHT: f32 = 48.0;

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
    /// 부·장·절 제목 스타일. 책 스타일이 없으면(메모) `None`.
    titles: Option<[ResolvedStyle; 3]>,
}

impl BookLayout {
    pub(super) fn new(styles: &library::Styles, font_size: f32) -> Self {
        Self {
            font_size,
            body: styles.resolve(StyleTarget::Body),
            titles: Some([
                styles.resolve(StyleTarget::Part),
                styles.resolve(StyleTarget::Chapter),
                styles.resolve(StyleTarget::Section),
            ]),
        }
    }

    /// 책 스타일 없는 배치(메모). 왼쪽 정렬, 들여쓰기·블록 여백 없음, 제목은 [`HEADING_SCALES`].
    pub(super) fn plain(font_size: f32) -> Self {
        Self {
            font_size,
            body: ResolvedStyle {
                align: Align::Left,
                first_line_indent: 0.0,
                ..StyleTarget::Body.defaults()
            },
            titles: None,
        }
    }

    /// 본문 줄 간격(배율).
    pub(super) fn line_height(&self) -> f64 {
        self.body.line_height
    }

    fn style(&self, target: StyleTarget) -> Option<&ResolvedStyle> {
        let [part, chapter, section] = self.titles.as_ref()?;
        Some(match target {
            StyleTarget::Body => &self.body,
            StyleTarget::Part => part,
            StyleTarget::Chapter => chapter,
            StyleTarget::Section => section,
        })
    }

    fn em(&self, em: f64) -> Pixels {
        px(self.font_size * em as f32)
    }

    /// 블록의 (앞 여백, 뒤 여백). 스타일 대상이 없는 블록은 문단과 같다.
    fn spacing(&self, block: Block) -> (f64, f64) {
        let style = block
            .target()
            .and_then(|target| self.style(target))
            .unwrap_or(&self.body);
        (style.space_before, style.space_after)
    }

    /// 두 블록 사이 여백(px): 앞 블록의 뒤 여백과 뒤 블록의 앞 여백 중 큰 값.
    fn gap(&self, before: Block, after: Block) -> Pixels {
        self.em(self.spacing(before).1.max(self.spacing(after).0))
    }

    /// 제목 블록의 책 스타일. 부·장·절이 아니거나 책 스타일이 없으면(메모) `None`.
    fn title(&self, block: Block) -> Option<&ResolvedStyle> {
        match block {
            Block::Heading(level) => self.style(StyleTarget::for_heading(level)?),
            _ => None,
        }
    }

    /// 이 블록이 새 페이지에서 시작하는지(별도 제목 페이지는 늘 새 페이지).
    fn starts_page(&self, block: Block) -> bool {
        self.title(block)
            .is_some_and(|title| title.placement == Placement::Page || title.page_break)
    }

    /// 이 블록 뒤에서 페이지가 나뉘는지: 다음 블록이 새 페이지에서 시작하거나, 이 블록이 별도
    /// 제목 페이지다.
    fn breaks_after(&self, block: Block, next: Option<Block>) -> bool {
        next.is_some_and(|next| self.starts_page(next))
            || self
                .title(block)
                .is_some_and(|title| title.placement == Placement::Page)
    }

    /// 줄과 다음 줄을 보고 줄 스타일을 정한다. 줄 하나가 문단이다(ADR-0011).
    pub(super) fn line_style(&self, line: LineContext<'_>) -> LineStyle {
        let block = Block::of(line.text);
        let next = line.next.map(Block::of);
        let mut style = self.block_style(block, self.gap_after(block, next));
        if self.breaks_after(block, next) {
            // 페이지가 나뉘면 다음 제목의 앞 여백은 없다(새 페이지 첫머리). 제목 페이지 뒤도 같다.
            style.block_height = px(PAGE_BREAK_HEIGHT);
            style.space_after = match self.title(block) {
                Some(title) if title.placement == Placement::Page => px(0.),
                _ => self.em(self.spacing(block).1),
            };
        }
        if line.prev.is_none() && self.starts_page(block) {
            style.block_before = px(PAGE_BREAK_HEIGHT);
        }
        style
    }

    /// 블록 모양(정렬·들여쓰기·제목 크기)과 줄 아래 여백.
    fn block_style(&self, block: Block, space_after: Pixels) -> LineStyle {
        match block {
            // 빈 줄은 평범한 빈 행이다. 마크다운 렌더링에서는 사라지지만 쓰는 화면에는 남긴다.
            Block::Blank => LineStyle::default(),
            // 이미지 줄은 커서가 있을 때만 글자가 보인다(ADR-0008).
            Block::Image => LineStyle {
                collapsed: true,
                ..Default::default()
            },
            Block::Heading(level) => {
                let style = StyleTarget::for_heading(level).and_then(|target| self.style(target));
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
        // 페이지 나눔은 따로 본다(`page_breaks_before_new_page_titles_and_after_title_pages`).
        styles.chapter.page_break = Some(false);
        styles.part.placement = Some(Placement::Inline);
        styles.part.page_break = Some(false);
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
    fn page_breaks_before_new_page_titles_and_after_title_pages() {
        // 기본값: 부는 별도 제목 페이지, 장은 새 페이지, 절은 이어서.
        let mut styles = library::Styles::default();
        styles.body.space_after = Some(0.5);
        let layout = BookLayout::new(&styles, 10.0);
        let before_chapter = layout.line_style(context(None, "문단", Some("## 장")));
        assert_eq!(before_chapter.block_height, px(PAGE_BREAK_HEIGHT));
        // 장의 앞 여백(3em) 대신 문단 뒤 여백만 둔다.
        assert_eq!(before_chapter.space_after, px(5.));
        let blank = layout.line_style(context(Some("문단"), "", Some("# 부")));
        assert_eq!(blank.block_height, px(PAGE_BREAK_HEIGHT));
        let part = layout.line_style(context(Some(""), "# 부", Some("문단")));
        assert_eq!(part.block_height, px(PAGE_BREAK_HEIGHT));
        assert_eq!(part.space_after, px(0.));
        let before_section = layout.line_style(context(None, "문단", Some("### 절")));
        assert_eq!(before_section.block_height, px(0.));
        let chapter = layout.line_style(context(None, "## 장", Some("문단")));
        assert_eq!(chapter.block_height, px(0.));
        // 시트 첫 줄 제목은 문서 위에 표시한다.
        assert_eq!(chapter.block_before, px(PAGE_BREAK_HEIGHT));
        let later = layout.line_style(context(Some(""), "## 장", Some("문단")));
        assert_eq!(later.block_before, px(0.));
        let section = layout.line_style(context(None, "### 절", Some("문단")));
        assert_eq!(section.block_before, px(0.));

        let mut styles = library::Styles::default();
        styles.chapter.page_break = Some(false);
        styles.section.placement = Some(Placement::Hidden);
        styles.section.page_break = Some(true);
        let layout = BookLayout::new(&styles, 10.0);
        let before_chapter = layout.line_style(context(None, "문단", Some("## 장")));
        assert_eq!(before_chapter.block_height, px(0.));
        // 숨긴 제목도 새 페이지는 연다.
        let before_section = layout.line_style(context(None, "문단", Some("### 절")));
        assert_eq!(before_section.block_height, px(PAGE_BREAK_HEIGHT));
    }

    #[test]
    fn notes_ignore_book_styles() {
        let layout = BookLayout::plain(10.0);
        let text = layout.line_style(context(Some(""), "문단", Some("## 장")));
        assert_eq!(text.first_indent, px(0.));
        assert_eq!(text.align, LineAlign::Start);
        assert_eq!(text.space_after, px(0.));
        let heading = layout.line_style(context(None, "## 장", Some("문단")));
        assert_eq!(heading.text_scale, HEADING_SCALES[1]);
        assert_eq!(heading.align, LineAlign::Start);
        assert_eq!(heading.space_after, px(0.));
        let before_part = layout.line_style(context(None, "문단", Some("# 부")));
        assert_eq!(before_part.block_height, px(0.));
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
