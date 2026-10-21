//! 책 배치: 원고 블록을 책 스타일대로 줄바꿈하고 페이지로 나눈다(ADR-0013).
//!
//! 길이는 모두 배율 1의 `Pixels`로 계산한다. PDF는 1 = 1pt, ePub은 1 = CSS 1px이다.
//! 화면에 그릴 때 배율을 곱한다(`page.rs`).
//!
//! - 줄바꿈: 문단을 한 줄로 shape한 뒤 글자 위치(`x_for_index`)로 단어(공백 묶음) 단위로 나눈다.
//!   한글·한자·가나는 글자마다 나눌 수 있고(에디터 줄바꿈과 같다), 한 단어가 폭을 넘으면 글자 단위로
//!   자른다. 닫는 부호는 앞 글자에, 여는 부호는 뒤 글자에 붙인다.
//! - 양쪽 정렬: 마지막 행을 뺀 행에서 공백 묶음마다 같은 몫을 더한다. 공백이 없는 행은 나눌 수 있는
//!   자리(한중일 글자 사이)에 나눈다(ADR-0010 에디터와 같은 규칙).
//! - 블록 사이 여백 = max(앞 블록 뒤 여백, 뒤 블록 앞 여백). 페이지 첫머리에서는 여백을 두지 않는다.

use gpui_kit::gpui::*;
use std::ops::Range;
use std::path::{Path, PathBuf};

use super::BookSource;
use crate::library::{self, Align, Placement, ResolvedStyle, StyleTarget, VerticalAlign};
use crate::markdown::{self, Block, ColumnAlign, ImageAlign, ImageLine, Inline, Rich, Table};

/// 책 글자색.
pub(super) const INK: Hsla = Hsla {
    h: 0.,
    s: 0.,
    l: 0.1,
    a: 1.,
};

/// 캡션·쪽 번호 같은 옅은 글자색.
pub(super) const MUTED_INK: Hsla = Hsla {
    h: 0.,
    s: 0.,
    l: 0.45,
    a: 1.,
};

/// 표 줄·각주 구분선 색.
const RULE_INK: Hsla = Hsla {
    h: 0.,
    s: 0.,
    l: 0.75,
    a: 1.,
};

/// 페이지 판형과 배치 규칙.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct PageSetup {
    pub width: Pixels,
    pub height: Pixels,
    pub margin: Edges<Pixels>,
    /// 본문 글자 크기(em 기준).
    pub font_size: Pixels,
    /// 이미지 속성의 px를 배치 단위로 바꾸는 배율(PDF 0.75, ePub 1).
    pub px: f32,
    /// PDF: 쪽 번호와 인쇄 차례를 넣는다. 서체는 늘 넣는다(PDF에는 서체가 들어간다).
    pub print: bool,
}

/// 배치에 쓰는 서체 이름.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Fonts {
    /// 스타일에 서체가 없을 때 쓰는 서체(설정의 에디터 서체).
    pub editor: SharedString,
    /// 서체를 넣지 않은 ePub이 보일 리더 기본 서체(시스템 서체).
    pub reader: SharedString,
    pub mono: SharedString,
}

/// 페이지 위에 놓인 것. 위치는 페이지 왼쪽 위 기준이다.
#[derive(Clone, Debug)]
pub(super) enum Item {
    /// 한 줄(또는 양쪽 정렬된 행의 한 조각) 글자.
    Text {
        origin: Point<Pixels>,
        line_height: Pixels,
        font_size: Pixels,
        text: SharedString,
        runs: Vec<TextRun>,
    },
    Image {
        bounds: Bounds<Pixels>,
        source: ImageFile,
        /// 표지는 페이지를 꽉 채운다(잘라 냄). 나머지는 비율대로 칸에 맞춘다.
        cover: bool,
    },
    /// 가는 선.
    Rule { bounds: Bounds<Pixels>, color: Hsla },
}

/// 그릴 이미지.
#[derive(Clone, Debug)]
pub(super) enum ImageFile {
    File { path: PathBuf, modified: u128 },
    Url(SharedUri),
}

#[derive(Clone, Debug, Default)]
pub(super) struct Page {
    pub items: Vec<Item>,
}

/// 차례 항목.
#[derive(Clone, Debug)]
pub(super) struct TocEntry {
    pub title: String,
    /// 0 = 부, 1 = 장, 2 = 절.
    pub depth: usize,
    /// 페이지 위치(0부터).
    pub page: usize,
}

/// 배치 결과.
pub(super) struct BookLayout {
    pub setup: PageSetup,
    pub pages: Vec<Page>,
    pub toc: Vec<TocEntry>,
}

/// 원고를 배치한다. PDF는 표지 → 속표지 → 선택적 차례 → 본문, ePub은 표지 → 속표지 → 본문 순이다.
pub(super) fn layout_book(
    source: &BookSource,
    setup: PageSetup,
    fonts: &Fonts,
    text_system: &WindowTextSystem,
    generate_toc: bool,
) -> BookLayout {
    let mut front = Vec::new();
    let mut flow = Flow::new(setup, fonts, &source.styles, text_system);

    if let Some(cover) = &source.cover
        && let Some(file) = image_file(cover)
    {
        let full = Bounds::new(Point::default(), size(setup.width, setup.height));
        // PDF 표지는 판형을 꽉 채우고, ePub 표지는 화면 안에 다 보이게 둔다.
        front.push(Page {
            items: vec![Item::Image {
                bounds: full,
                source: file.source,
                cover: setup.print,
            }],
        });
    }
    front.push(flow.title_page(&source.title, &source.authors));

    for chunk in &source.chunks {
        if !chunk.glued {
            flow.page_break();
        }
        flow.dir = chunk.dir.clone();
        let ctx = flow.root_context();
        for block in &chunk.blocks {
            flow.block(block, &ctx);
        }
    }
    let mut body = flow.finish();

    let mut toc_pages = Vec::new();
    if setup.print {
        // 쪽 번호는 본문 첫 페이지부터 1이다. 제목만 있는 페이지·전면 이미지에는 찍지 않는다.
        for (ix, page) in body.pages.iter_mut().enumerate() {
            if !body.no_folio.contains(&ix) {
                let mut folio = Flow::new(setup, fonts, &source.styles, text_system);
                folio.folio(page, ix + 1);
            }
        }
        if generate_toc && !body.toc.is_empty() {
            let flow = Flow::new(setup, fonts, &source.styles, text_system);
            toc_pages = flow.toc_pages(&body.toc);
        }
    }

    let offset = front.len() + toc_pages.len();
    let toc = body
        .toc
        .into_iter()
        .map(|entry| TocEntry {
            page: entry.page + offset,
            ..entry
        })
        .collect();
    let mut pages = front;
    pages.extend(toc_pages);
    pages.extend(body.pages);
    BookLayout { setup, pages, toc }
}

/// 글자 모양 하나(대상 스타일을 배치 단위로 옮긴 것).
#[derive(Clone, Debug)]
struct TextStyle {
    family: SharedString,
    size: Pixels,
    weight: FontWeight,
    italic: bool,
    line_height: Pixels,
    align: Align,
    indent: Pixels,
    before: Pixels,
    after: Pixels,
    color: Hsla,
}

impl TextStyle {
    fn scaled(&self, scale: f32) -> TextStyle {
        TextStyle {
            size: self.size * scale,
            line_height: self.line_height * scale,
            ..self.clone()
        }
    }
}

/// 블록을 놓을 가로 자리.
#[derive(Clone, Debug)]
struct Context {
    left: Pixels,
    width: Pixels,
    /// 문단 첫 줄 들여쓰기를 쓰는지(목록·인용·각주 안에서는 쓰지 않는다).
    indent: bool,
    /// 글자 크기 배율(각주 0.85).
    scale: f32,
}

/// 줄 하나: (조각 시작 x, 원문 범위)들.
struct Line {
    fragments: Vec<(Pixels, Range<usize>)>,
}

/// 본문 배치 결과(쪽 번호·차례는 바깥에서 붙인다).
struct Body {
    pages: Vec<Page>,
    toc: Vec<TocEntry>,
    /// 쪽 번호를 찍지 않는 페이지.
    no_folio: Vec<usize>,
}

/// 페이지를 채워 가는 배치기.
struct Flow<'a> {
    setup: PageSetup,
    fonts: &'a Fonts,
    styles: &'a library::Styles,
    text_system: &'a WindowTextSystem,
    pages: Vec<Page>,
    page: Page,
    /// 본문 칸 위에서부터 채운 높이.
    y: Pixels,
    /// 다음 블록 앞에 둘 여백(겹침: 큰 값).
    pending: Pixels,
    toc: Vec<TocEntry>,
    no_folio: Vec<usize>,
    /// 대상별 제목 번호.
    numbers: [usize; 3],
    /// 상대 경로 이미지의 기준(시트 폴더).
    dir: PathBuf,
}

impl<'a> Flow<'a> {
    fn new(
        setup: PageSetup,
        fonts: &'a Fonts,
        styles: &'a library::Styles,
        text_system: &'a WindowTextSystem,
    ) -> Self {
        Self {
            setup,
            fonts,
            styles,
            text_system,
            pages: Vec::new(),
            page: Page::default(),
            y: px(0.),
            pending: px(0.),
            toc: Vec::new(),
            no_folio: Vec::new(),
            numbers: [0; 3],
            dir: PathBuf::new(),
        }
    }

    fn content_width(&self) -> Pixels {
        self.setup.width - self.setup.margin.left - self.setup.margin.right
    }

    fn content_height(&self) -> Pixels {
        self.setup.height - self.setup.margin.top - self.setup.margin.bottom
    }

    fn root_context(&self) -> Context {
        Context {
            left: self.setup.margin.left,
            width: self.content_width(),
            indent: true,
            scale: 1.,
        }
    }

    fn em(&self, value: f32) -> Pixels {
        self.setup.font_size * value
    }

    /// 대상 스타일을 글자 모양으로 옮긴다.
    fn style(&self, target: StyleTarget) -> TextStyle {
        let style = self.styles.resolve(target);
        self.text_style(&style)
    }

    fn text_style(&self, style: &ResolvedStyle) -> TextStyle {
        // PDF에는 늘 서체가 들어간다. ePub은 넣지 않으면 리더 기본 서체로 보인다.
        let family = if self.setup.print || style.embed_font {
            style
                .font
                .clone()
                .map_or_else(|| self.fonts.editor.clone(), SharedString::from)
        } else {
            self.fonts.reader.clone()
        };
        let size = self.setup.font_size * style.font_size as f32;
        TextStyle {
            family,
            size,
            weight: FontWeight(f32::from(style.font_weight)),
            italic: false,
            line_height: size * style.line_height as f32,
            align: style.align,
            indent: self.em(style.first_line_indent as f32),
            before: self.em(style.space_before as f32),
            after: self.em(style.space_after as f32),
            color: INK,
        }
    }

    fn body_style(&self) -> TextStyle {
        self.style(StyleTarget::Body)
    }

    /// 글자 조각들을 원문 + TextRun으로.
    fn runs(&self, rich: &[Inline], style: &TextStyle) -> (String, Vec<TextRun>) {
        let mut text = String::new();
        let mut runs = Vec::new();
        for inline in rich {
            if inline.text.is_empty() {
                continue;
            }
            let marks = inline.marks;
            // 굵게: 보통 굵기면 700, 이미 굵은 제목이면 900.
            let weight = if marks.strong {
                FontWeight(if style.weight.0 >= 600. { 900. } else { 700. })
            } else {
                style.weight
            };
            let family = if marks.code {
                self.fonts.mono.clone()
            } else {
                style.family.clone()
            };
            let font = Font {
                weight,
                style: if marks.emphasis != style.italic {
                    FontStyle::Italic
                } else {
                    FontStyle::Normal
                },
                ..font(family)
            };
            text.push_str(&inline.text);
            runs.push(TextRun {
                len: inline.text.len(),
                font,
                color: style.color,
                background_color: None,
                underline: None,
                strikethrough: marks.strike.then_some(StrikethroughStyle {
                    thickness: px(1.),
                    color: Some(style.color),
                }),
            });
        }
        (text, runs)
    }

    // --- 페이지 ---------------------------------------------------------------

    fn page_index(&self) -> usize {
        self.pages.len()
    }

    /// 지금 페이지에 내용이나 명시적인 빈 줄이 있으면 다음 페이지로 넘긴다.
    fn page_break(&mut self) {
        if !self.page.items.is_empty() || self.y > px(0.) {
            self.pages.push(std::mem::take(&mut self.page));
        }
        self.y = px(0.);
        self.pending = px(0.);
    }

    /// 다음 블록 앞 여백을 둔다(겹치면 큰 값).
    fn space(&mut self, amount: Pixels) {
        self.pending = self.pending.max(amount);
    }

    /// 높이 `height`를 놓을 자리를 잡고 그 위쪽 y(페이지 기준)를 돌려준다.
    /// 뒤에 `keep`만큼이 함께 들어가지 않으면 다음 페이지로 넘긴다(제목 뒤 첫 줄 등).
    fn reserve(&mut self, height: Pixels, keep: Pixels) -> Pixels {
        let gap = if self.y > px(0.) {
            self.pending
        } else {
            px(0.)
        };
        if self.y > px(0.) && self.y + gap + height + keep > self.content_height() {
            self.page_break();
            return self.reserve(height, px(0.));
        }
        self.y += gap;
        self.pending = px(0.);
        let top = self.setup.margin.top + self.y;
        self.y += height;
        top
    }

    fn finish(mut self) -> Body {
        self.page_break();
        Body {
            pages: self.pages,
            toc: self.toc,
            no_folio: self.no_folio,
        }
    }

    // --- 줄바꿈 ---------------------------------------------------------------

    /// 문단을 줄로 나눈다. `\n`은 강제 줄바꿈이다. 범위는 `text` 기준이다.
    fn break_lines(
        &self,
        text: &str,
        runs: &[TextRun],
        style: &TextStyle,
        width: Pixels,
        indent: Pixels,
    ) -> Vec<Line> {
        let mut lines = Vec::new();
        let mut start = 0;
        for segment in text.split('\n') {
            let range = start..start + segment.len();
            start = range.end + 1;
            if segment.is_empty() {
                lines.push(Line {
                    fragments: Vec::new(),
                });
                continue;
            }
            let segment_runs = slice_runs(runs, range.clone());
            let layout = self
                .text_system
                .layout_line(segment, style.size, &segment_runs, None);
            let first = lines.is_empty();
            let broken = break_segment(
                segment,
                |ix| layout.x_for_index(ix),
                width,
                if first { indent } else { px(0.) },
                style.align,
            );
            lines.extend(broken.into_iter().map(|line| {
                Line {
                    fragments: line
                        .fragments
                        .into_iter()
                        .map(|(x, r)| (x, r.start + range.start..r.end + range.start))
                        .collect(),
                }
            }));
        }
        lines
    }

    /// 줄들을 `top`부터 아래로 놓는다.
    fn place_lines(
        &mut self,
        lines: &[Line],
        text: &str,
        runs: &[TextRun],
        style: &TextStyle,
        left: Pixels,
        top: Pixels,
    ) {
        for (ix, line) in lines.iter().enumerate() {
            let y = top + style.line_height * ix as f32;
            for (x, range) in &line.fragments {
                self.page.items.push(Item::Text {
                    origin: point(left + *x, y),
                    line_height: style.line_height,
                    font_size: style.size,
                    text: text[range.clone()].to_string().into(),
                    runs: slice_runs(runs, range.clone()),
                });
            }
        }
    }

    /// 한 줄 글자를 바로 놓는다(쪽 번호·캡션 등). `align`은 `left..left+width` 안 정렬이다.
    fn place_text(
        &mut self,
        rich: &[Inline],
        style: &TextStyle,
        left: Pixels,
        width: Pixels,
        top: Pixels,
    ) -> Pixels {
        let (text, runs) = self.runs(rich, style);
        let lines = self.break_lines(&text, &runs, style, width, px(0.));
        self.place_lines(&lines, &text, &runs, style, left, top);
        style.line_height * lines.len() as f32
    }

    // --- 블록 ---------------------------------------------------------------

    fn block(&mut self, block: &Block, ctx: &Context) {
        match block {
            Block::Heading {
                level,
                text,
                unnumbered,
            } => self.heading(*level, text, *unnumbered, ctx),
            Block::Paragraph(text) => {
                let mut style = self.body_style().scaled(ctx.scale);
                if !ctx.indent {
                    style.indent = px(0.);
                }
                self.paragraph(text, &style, ctx, None);
            }
            Block::Lines(text) => {
                let mut style = self.body_style().scaled(ctx.scale);
                style.indent = px(0.);
                self.paragraph(text, &style, ctx, None);
            }
            Block::BlankLines(count) => {
                let style = self.body_style().scaled(ctx.scale);
                for _ in 0..*count {
                    self.reserve(style.line_height, px(0.));
                }
            }
            Block::Quote(blocks) => {
                let inset = self.em(1.5);
                let inner = Context {
                    left: ctx.left + inset,
                    width: (ctx.width - inset * 2.).max(self.em(4.)),
                    indent: false,
                    ..ctx.clone()
                };
                self.space(self.em(0.75));
                for block in blocks {
                    self.block(block, &inner);
                }
                self.space(self.em(0.75));
            }
            Block::List {
                ordered,
                start,
                items,
            } => {
                self.space(self.em(0.5));
                for (ix, item) in items.iter().enumerate() {
                    let marker = if *ordered {
                        format!("{}.", *start as usize + ix)
                    } else {
                        "•".to_string()
                    };
                    self.list_item(&marker, item, ctx);
                }
                self.space(self.em(0.5));
            }
            Block::Code(code) => self.code(code, ctx),
            Block::Image(image) => self.image(image, ctx),
            Block::Rule => {
                let style = TextStyle {
                    align: Align::Center,
                    indent: px(0.),
                    ..self.body_style()
                };
                self.space(self.em(1.));
                let top = self.reserve(style.line_height, px(0.));
                self.place_text(
                    &markdown::plain("*\u{2003}*\u{2003}*"),
                    &style,
                    ctx.left,
                    ctx.width,
                    top,
                );
                self.space(self.em(1.));
            }
            Block::Table(table) => self.table(table, ctx),
            Block::Footnotes(notes) => {
                let scale = 0.85;
                self.space(self.em(1.5));
                let top = self.reserve(px(1.), self.body_style().line_height * scale);
                self.page.items.push(Item::Rule {
                    bounds: Bounds::new(point(ctx.left, top), size(ctx.width / 3., px(0.5))),
                    color: RULE_INK,
                });
                self.space(self.em(0.5));
                let inner = Context {
                    indent: false,
                    scale,
                    ..ctx.clone()
                };
                for (number, blocks) in notes {
                    self.list_item(&format!("{number}."), blocks, &inner);
                }
            }
        }
    }

    /// 문단 하나. `marker`가 있으면 첫 줄 왼쪽(`ctx.left` 앞)에 목록 기호를 함께 놓는다.
    fn paragraph(
        &mut self,
        rich: &Rich,
        style: &TextStyle,
        ctx: &Context,
        marker: Option<(&str, Pixels)>,
    ) {
        let (text, runs) = self.runs(rich, style);
        let lines = self.break_lines(&text, &runs, style, ctx.width, style.indent);
        self.space(style.before);
        // 한 줄씩 놓아 페이지 끝에서 문단이 나뉘게 한다. 첫 줄만 페이지 끝에 남거나(외톨이 줄)
        // 마지막 줄만 다음 페이지로 넘어가지(과부 줄) 않게 그다음 줄과 함께 둔다.
        // 세 줄 이하 문단은 나누지 않는다.
        let last = lines.len().saturating_sub(1);
        for (ix, line) in lines.iter().enumerate() {
            let keep = match (ix, last) {
                (0, 1..=2) => style.line_height * last as f32,
                (0, _) => style.line_height,
                (ix, last) if last > 2 && ix + 1 == last => style.line_height,
                _ => px(0.),
            };
            let top = self.reserve(style.line_height, keep);
            self.place_lines(
                std::slice::from_ref(line),
                &text,
                &runs,
                style,
                ctx.left,
                top,
            );
            if ix == 0
                && let Some((marker, width)) = marker
            {
                let marker_style = TextStyle {
                    align: Align::Left,
                    ..style.clone()
                };
                self.place_text(
                    &markdown::plain(marker),
                    &marker_style,
                    ctx.left - width,
                    width,
                    top,
                );
            }
        }
        self.space(style.after);
    }

    /// 목록 항목(각주 포함): 기호는 걸어 두고 내용은 그만큼 들여 놓는다.
    fn list_item(&mut self, marker: &str, blocks: &[Block], ctx: &Context) {
        let hang = self.em(1.6 * ctx.scale);
        let inner = Context {
            left: ctx.left + hang,
            width: (ctx.width - hang).max(self.em(4.)),
            indent: false,
            ..ctx.clone()
        };
        let mut blocks = blocks.iter();
        match blocks.next() {
            Some(Block::Paragraph(text) | Block::Lines(text)) => {
                let mut style = self.body_style().scaled(ctx.scale);
                style.indent = px(0.);
                style.after = px(0.);
                self.paragraph(text, &style, &inner, Some((marker, hang)));
            }
            first => {
                // 문단으로 시작하지 않는 항목은 기호를 한 줄에 따로 둔다.
                let style = self.body_style().scaled(ctx.scale);
                let top = self.reserve(style.line_height, px(0.));
                self.place_text(&markdown::plain(marker), &style, ctx.left, hang, top);
                if let Some(first) = first {
                    self.block(first, &inner);
                }
            }
        }
        for block in blocks {
            self.block(block, &inner);
        }
    }

    fn heading(&mut self, level: u8, text: &Rich, unnumbered: bool, ctx: &Context) {
        let Some(target) = StyleTarget::for_heading(level) else {
            // `####` 이하: 본문 크기의 굵은 문단.
            let style = TextStyle {
                weight: FontWeight::BOLD,
                indent: px(0.),
                align: Align::Left,
                before: self.em(1.),
                after: self.em(0.25),
                ..self.body_style()
            };
            let (string, runs) = self.runs(text, &style);
            let lines = self.break_lines(&string, &runs, &style, ctx.width, px(0.));
            self.space(style.before);
            let height = style.line_height * lines.len() as f32;
            let top = self.reserve(height, self.body_style().line_height);
            self.place_lines(&lines, &string, &runs, &style, ctx.left, top);
            self.space(style.after);
            return;
        };
        let resolved = self.styles.resolve(target);
        let depth = match target {
            StyleTarget::Part => 0,
            StyleTarget::Chapter => 1,
            _ => 2,
        };
        // 번호와 접두사는 목차 포함 여부와 무관하게 붙인다(TOC에만 넣지 않는다).
        // 다만 그리지도 않고 목차에도 넣지 않는 제목은 번호를 쓰지 않는다.
        // `{unnumbered}` 제목(시트 첫 줄만)은 번호를 소비하지 않고 접두사도 붙지 않는다(ADR-0018).
        let numbered = !unnumbered && (resolved.placement != Placement::Hidden || resolved.in_toc);
        if numbered {
            self.numbers[depth] += 1;
        }
        let prefix = if numbered {
            resolved.prefix_text(self.numbers[depth])
        } else {
            String::new()
        };
        let mut title = markdown::plain(prefix);
        title.extend(text.iter().cloned());
        let style = self.text_style(&resolved);
        let new_page = resolved.placement == Placement::Page || resolved.page_break;
        if new_page {
            self.page_break();
        }
        let page = self.page_index();
        if resolved.in_toc {
            // 차례에서는 접두사(번호)와 제목의 개행(`<br>`)을 스페이스로 바꾼다.
            let toc_prefix = if numbered {
                resolved.toc_prefix_text(self.numbers[depth])
            } else {
                String::new()
            };
            let mut toc_title = markdown::plain(toc_prefix);
            toc_title.extend(text.iter().cloned());
            self.toc.push(TocEntry {
                title: markdown::rich_text(&toc_title)
                    .replace(['\n', '\r'], " ")
                    .trim()
                    .to_string(),
                depth,
                page,
            });
        }

        let (string, runs) = self.runs(&title, &style);
        let lines = self.break_lines(&string, &runs, &style, ctx.width, style.indent);
        let height = style.line_height * lines.len() as f32;
        match resolved.placement {
            Placement::Hidden => {}
            Placement::Page => {
                let free = (self.content_height() - height).max(px(0.));
                let offset = match resolved.vertical_align {
                    VerticalAlign::Top => px(0.),
                    VerticalAlign::Center => free / 2.,
                    VerticalAlign::Bottom => free,
                };
                let top = self.setup.margin.top + offset;
                self.place_lines(&lines, &string, &runs, &style, ctx.left, top);
                self.no_folio.push(page);
                self.y = self.content_height();
                self.page_break();
            }
            Placement::Inline => {
                self.space(style.before);
                let top = self.reserve(height, self.body_style().line_height);
                self.place_lines(&lines, &string, &runs, &style, ctx.left, top);
                self.space(style.after);
            }
        }
    }

    fn code(&mut self, code: &str, ctx: &Context) {
        let base = self.body_style();
        let style = TextStyle {
            family: self.fonts.mono.clone(),
            size: base.size * 0.85 * ctx.scale,
            weight: FontWeight::NORMAL,
            italic: false,
            line_height: base.size * 0.85 * ctx.scale * 1.45,
            align: Align::Left,
            indent: px(0.),
            before: self.em(0.75),
            after: self.em(0.75),
            color: INK,
        };
        let inset = self.em(1.);
        let inner = Context {
            left: ctx.left + inset,
            width: (ctx.width - inset).max(self.em(4.)),
            ..ctx.clone()
        };
        self.space(style.before);
        for line in code.split('\n') {
            // 공백을 줄바꿈하지 않는 공백으로 바꿔 들여쓰기를 지키고, 긴 줄은 글자 단위로 자른다.
            let line = line.replace('\t', "    ").replace(' ', "\u{a0}");
            let rich = markdown::plain(line);
            let (text, runs) = self.runs(&rich, &style);
            let lines = self.break_lines(&text, &runs, &style, inner.width, px(0.));
            for line in &lines {
                let top = self.reserve(style.line_height, px(0.));
                self.place_lines(
                    std::slice::from_ref(line),
                    &text,
                    &runs,
                    &style,
                    inner.left,
                    top,
                );
            }
        }
        self.space(style.after);
    }

    fn table(&mut self, table: &Table, ctx: &Context) {
        let columns = table.columns();
        if columns == 0 {
            return;
        }
        let gap = self.em(0.8);
        let pad = self.em(0.35);
        let base = TextStyle {
            align: Align::Left,
            indent: px(0.),
            ..self.body_style().scaled(0.9 * ctx.scale)
        };
        let header = TextStyle {
            weight: FontWeight::BOLD,
            ..base.clone()
        };
        let style_of = |row_ix: usize, column: usize| TextStyle {
            align: match table.align(column) {
                ColumnAlign::Left => Align::Left,
                ColumnAlign::Center => Align::Center,
                ColumnAlign::Right => Align::Right,
            },
            ..if row_ix == 0 { header.clone() } else { base.clone() }
        };
        // 열 너비는 칸 글자를 한 줄로 놓았을 때의 폭을 따른다.
        let mut natural = vec![0f32; columns];
        for (row_ix, row) in table.rows.iter().enumerate() {
            for (column, cell) in row.iter().enumerate() {
                let width = self.natural_width(cell, &style_of(row_ix, column));
                natural[column] = natural[column].max(f32::from(width));
            }
        }
        let available = f32::from(ctx.width - gap * (columns - 1) as f32);
        let widths: Vec<Pixels> = column_widths(&natural, available).into_iter().map(px).collect();
        let lefts: Vec<Pixels> = widths
            .iter()
            .scan(ctx.left, |left, width| {
                let this = *left;
                *left += *width + gap;
                Some(this)
            })
            .collect();
        let rows: Vec<_> = table
            .rows
            .iter()
            .enumerate()
            .map(|(row_ix, row)| {
                let cells: Vec<_> = (0..columns)
                    .map(|column| {
                        let style = style_of(row_ix, column);
                        let empty = Rich::new();
                        let cell = row.get(column).unwrap_or(&empty);
                        let (text, runs) = self.runs(cell, &style);
                        let lines = self.break_lines(&text, &runs, &style, widths[column], px(0.));
                        (style, text, runs, lines)
                    })
                    .collect();
                let lines = cells.iter().map(|(_, _, _, lines)| lines.len()).max();
                let height = base.line_height * lines.unwrap_or(1).max(1) as f32 + pad * 2.;
                (cells, height)
            })
            .collect();
        let rule = |top: Pixels| Item::Rule {
            bounds: Bounds::new(point(ctx.left, top), size(ctx.width, px(0.5))),
            color: RULE_INK,
        };
        let place_row = |this: &mut Self, row_ix: usize, top: Pixels| {
            let (cells, height) = &rows[row_ix];
            for (column, (style, text, runs, lines)) in cells.iter().enumerate() {
                this.place_lines(lines, text, runs, style, lefts[column], top + pad);
            }
            this.page.items.push(rule(top + *height));
        };
        self.space(self.em(1.));
        for row_ix in 0..rows.len() {
            let height = rows[row_ix].1;
            // 머리 행은 첫 본문 행과 같은 쪽에 둔다.
            let keep = if row_ix == 0 { rows.get(1).map_or(px(0.), |row| row.1) } else { px(0.) };
            let mut top = self.reserve(height, keep);
            let new_page = top == self.setup.margin.top;
            if row_ix == 0 || new_page {
                self.page.items.push(rule(top));
            }
            // 다음 쪽으로 넘어간 표는 머리 행을 다시 그린다.
            if row_ix > 0 && new_page {
                place_row(self, 0, top);
                top += rows[0].1;
                self.y += rows[0].1;
            }
            place_row(self, row_ix, top);
        }
        self.space(self.em(1.));
    }

    /// 글자를 줄바꿈하지 않고 놓았을 때 가장 긴 줄의 폭(`\n`은 강제 줄바꿈).
    fn natural_width(&self, rich: &[Inline], style: &TextStyle) -> Pixels {
        let (text, runs) = self.runs(rich, style);
        let mut start = 0;
        let mut widest = px(0.);
        for segment in text.split('\n') {
            let range = start..start + segment.len();
            start = range.end + 1;
            if segment.is_empty() {
                continue;
            }
            let layout = self.text_system.layout_line(
                segment,
                style.size,
                &slice_runs(&runs, range),
                None,
            );
            widest = widest.max(layout.x_for_index(segment.len()));
        }
        widest
    }

    fn image(&mut self, image: &ImageLine, ctx: &Context) {
        let caption = image.alt.trim();
        let caption_style = TextStyle {
            align: Align::Center,
            indent: px(0.),
            italic: true,
            color: MUTED_INK,
            before: px(0.),
            after: px(0.),
            ..self.body_style().scaled(0.85)
        };
        let file = image_source(&self.dir, &image.destination);
        let Some(file) = file else {
            let style = TextStyle {
                align: Align::Left,
                indent: px(0.),
                color: MUTED_INK,
                ..self.body_style().scaled(0.85)
            };
            self.space(self.em(0.5));
            let top = self.reserve(style.line_height, px(0.));
            let text = format!("[이미지를 찾을 수 없음: {}]", image.destination);
            self.place_text(&markdown::plain(text), &style, ctx.left, ctx.width, top);
            self.space(self.em(0.5));
            return;
        };
        let caption_height = if caption.is_empty() {
            px(0.)
        } else {
            self.em(0.5) + caption_style.line_height
        };
        let px_scale = self.setup.px;
        let natural = file
            .size
            .map(|(w, h)| size(px(w * px_scale), px(h * px_scale)));

        if image.options.full_page {
            // 전용 페이지에 칸 안에 맞춰 가운데 둔다.
            self.page_break();
            let page = self.page_index();
            let area = size(self.content_width(), self.content_height() - caption_height);
            let fitted = match natural {
                Some(natural) => fit(natural, area),
                None => area,
            };
            let left = self.setup.margin.left + (area.width - fitted.width) / 2.;
            let top = self.setup.margin.top + (area.height - fitted.height) / 2.;
            self.page.items.push(Item::Image {
                bounds: Bounds::new(point(left, top), fitted),
                source: file.source,
                cover: false,
            });
            if !caption.is_empty() {
                self.place_text(
                    &markdown::plain(caption),
                    &caption_style,
                    self.setup.margin.left,
                    self.content_width(),
                    top + fitted.height + self.em(0.5),
                );
            }
            self.no_folio.push(page);
            self.y = self.content_height();
            self.page_break();
            return;
        }

        let options = &image.options;
        let mut shown = match natural {
            Some(natural) => {
                let aspect = natural.height / natural.width;
                match (options.width, options.height) {
                    (Some(w), Some(h)) => size(px(w as f32 * px_scale), px(h as f32 * px_scale)),
                    (Some(w), None) => {
                        let w = px(w as f32 * px_scale);
                        size(w, w * aspect)
                    }
                    (None, Some(h)) => {
                        let h = px(h as f32 * px_scale);
                        size(h / aspect, h)
                    }
                    (None, None) => natural,
                }
            }
            // 비율을 모르는 웹 이미지는 16:9 칸.
            None => size(ctx.width, ctx.width * 0.5625),
        };
        let max = size(ctx.width, self.content_height() - caption_height);
        if shown.width > max.width || shown.height > max.height {
            shown = fit(shown, max);
        }
        self.space(self.em(1.));
        let top = self.reserve(shown.height + caption_height, px(0.));
        let left = match options.align {
            ImageAlign::Left => ctx.left,
            ImageAlign::Center => ctx.left + (ctx.width - shown.width) / 2.,
            ImageAlign::Right => ctx.left + ctx.width - shown.width,
        };
        self.page.items.push(Item::Image {
            bounds: Bounds::new(point(left, top), shown),
            source: file.source,
            cover: false,
        });
        if !caption.is_empty() {
            self.place_text(
                &markdown::plain(caption),
                &caption_style,
                ctx.left,
                ctx.width,
                top + shown.height + self.em(0.5),
            );
        }
        self.space(self.em(1.));
    }

    // --- 앞뒤 페이지 -----------------------------------------------------------

    /// 속표지: 책 제목과 지은이를 위쪽 1/3 즈음 가운데에.
    fn title_page(&mut self, title: &str, authors: &[String]) -> Page {
        let part = self.style(StyleTarget::Part);
        let title_style = TextStyle {
            size: self.em(2.),
            line_height: self.em(2.) * 1.3,
            weight: FontWeight::BOLD,
            align: Align::Center,
            ..part.clone()
        };
        let author_style = TextStyle {
            size: self.em(1.1),
            line_height: self.em(1.1) * 1.5,
            weight: FontWeight::NORMAL,
            align: Align::Center,
            ..part
        };
        let left = self.setup.margin.left;
        let width = self.content_width();
        let mut top = self.setup.margin.top + self.content_height() * 0.28;
        top += self.place_text(&markdown::plain(title), &title_style, left, width, top);
        if !authors.is_empty() {
            top += self.em(1.5);
            self.place_text(
                &markdown::plain(authors.join(", ")),
                &author_style,
                left,
                width,
                top,
            );
        }
        std::mem::take(&mut self.page)
    }

    /// 인쇄 차례: "차례" + 항목(깊이만큼 들여 쓰고 오른쪽에 쪽 번호).
    fn toc_pages(mut self, entries: &[TocEntry]) -> Vec<Page> {
        let heading = TextStyle {
            align: Align::Center,
            indent: px(0.),
            ..self.style(StyleTarget::Chapter)
        };
        let body = TextStyle {
            align: Align::Left,
            indent: px(0.),
            ..self.body_style()
        };
        let left = self.setup.margin.left;
        let width = self.content_width();
        let height = self.place_text(
            &markdown::plain("차례"),
            &heading,
            left,
            width,
            self.setup.margin.top,
        );
        self.y = height;
        self.space(heading.after.max(self.em(2.)));
        let number_width = self.em(3.);
        for entry in entries {
            let style = TextStyle {
                weight: if entry.depth == 0 {
                    FontWeight::BOLD
                } else {
                    body.weight
                },
                ..body.clone()
            };
            let inset = self.em(1.5) * entry.depth as f32;
            let (text, runs) = self.runs(&markdown::plain(entry.title.clone()), &style);
            let lines =
                self.break_lines(&text, &runs, &style, width - inset - number_width, px(0.));
            if entry.depth == 0 {
                self.space(self.em(0.6));
            }
            let entry_height = style.line_height * lines.len() as f32;
            let top = self.reserve(entry_height, px(0.));
            self.place_lines(&lines, &text, &runs, &style, left + inset, top);
            let number = TextStyle {
                align: Align::Right,
                weight: FontWeight::NORMAL,
                ..style.clone()
            };
            // 본문 첫 페이지가 1쪽이다.
            let folio = (entry.page + 1).to_string();
            self.place_text(
                &markdown::plain(folio),
                &number,
                left + width - number_width,
                number_width,
                top + entry_height - style.line_height,
            );
        }
        self.finish().pages
    }

    /// 페이지 아래 여백 가운데에 쪽 번호를 찍는다.
    fn folio(&mut self, page: &mut Page, number: usize) {
        let style = TextStyle {
            align: Align::Center,
            indent: px(0.),
            color: MUTED_INK,
            weight: FontWeight::NORMAL,
            ..self.body_style().scaled(0.8)
        };
        let top = self.setup.height - self.setup.margin.bottom / 2. - style.line_height / 2.;
        self.place_text(
            &markdown::plain(number.to_string()),
            &style,
            self.setup.margin.left,
            self.content_width(),
            top,
        );
        page.items.append(&mut self.page.items);
    }
}

/// `size`를 비율대로 `area` 안에 맞춘다.
fn fit(natural: Size<Pixels>, area: Size<Pixels>) -> Size<Pixels> {
    let scale = (area.width / natural.width)
        .min(area.height / natural.height)
        .max(0.);
    size(natural.width * scale, natural.height * scale)
}

/// 읽은 이미지 정보. `size`는 원본 픽셀 크기(모르면 `None`).
struct ResolvedImage {
    source: ImageFile,
    size: Option<(f32, f32)>,
}

/// 이미지 경로 → 파일. 상대 경로는 시트 폴더 기준이다. 읽을 수 없으면 `None`.
fn image_source(dir: &Path, destination: &str) -> Option<ResolvedImage> {
    if destination.starts_with("http://") || destination.starts_with("https://") {
        return Some(ResolvedImage {
            source: ImageFile::Url(destination.to_string().into()),
            size: None,
        });
    }
    let path = destination.strip_prefix("file://").unwrap_or(destination);
    let path = Path::new(path);
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        dir.join(path)
    };
    image_file(&path)
}

fn image_file(path: &Path) -> Option<ResolvedImage> {
    let size = imagesize::size(path).ok()?;
    if size.width == 0 || size.height == 0 {
        return None;
    }
    Some(ResolvedImage {
        source: ImageFile::File {
            path: path.to_path_buf(),
            modified: library::modified_nanos(path),
        },
        size: Some((size.width as f32, size.height as f32)),
    })
}

/// `runs`에서 `range`(바이트) 부분만 잘라 낸다.
fn slice_runs(runs: &[TextRun], range: Range<usize>) -> Vec<TextRun> {
    let mut out = Vec::new();
    let mut start = 0;
    for run in runs {
        let end = start + run.len;
        let from = start.max(range.start);
        let to = end.min(range.end);
        if from < to {
            out.push(TextRun {
                len: to - from,
                ..run.clone()
            });
        }
        start = end;
        if start >= range.end {
            break;
        }
    }
    out
}

/// 줄을 나눌 수 있는 공백인지. 줄바꿈하지 않는 공백(U+00A0)은 뺀다.
fn is_break_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\u{3000}')
}

/// 글자마다 줄을 나눌 수 있는 한중일 문자. 한글도 음절마다 나눈다(에디터 줄바꿈과 같다).
fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x11FF   // 한글 자모
        | 0x3040..=0x30FF // 히라가나·가타카나
        | 0x3130..=0x318F // 한글 호환 자모
        | 0x3400..=0x4DBF // 한자 확장 A
        | 0x4E00..=0x9FFF // 한자
        | 0xAC00..=0xD7A3 // 한글 음절
        | 0xF900..=0xFAFF // 호환 한자
    )
}

/// 줄 첫머리에 올 수 없는 문장 부호(앞 글자에 붙인다).
fn is_closing_punctuation(c: char) -> bool {
    matches!(
        c,
        '、' | '。'
            | '，'
            | '．'
            | '」'
            | '』'
            | '）'
            | '》'
            | '〉'
            | '】'
            | '！'
            | '？'
            | '：'
            | '；'
            | 'ー'
            | '.'
            | ','
            | '!'
            | '?'
            | ':'
            | ';'
            | ')'
            | ']'
            | '}'
            | '”'
            | '’'
            | '…'
    )
}

/// 줄 끝에 올 수 없는 여는 부호(뒤 글자에 붙인다).
fn is_opening_punctuation(c: char) -> bool {
    matches!(
        c,
        '“' | '‘' | '(' | '[' | '{' | '「' | '『' | '（' | '《' | '〈' | '【' | '"' | '\''
    )
}

/// 나눌 수 없는 글자 덩어리와 그 뒤 공백: `[start, end)` + 공백 끝.
#[derive(Clone, Copy, Debug)]
struct Token {
    start: usize,
    end: usize,
    space_end: usize,
}

fn tokens(text: &str) -> Vec<Token> {
    let mut out: Vec<Token> = Vec::new();
    let mut chars = text.char_indices().peekable();
    while let Some(&(ix, c)) = chars.peek() {
        if is_break_space(c) {
            // 맨 앞 공백은 첫 덩어리의 일부로 둔다(들여쓰기 흉내).
            chars.next();
            match out.last_mut() {
                Some(last) => last.space_end = ix + c.len_utf8(),
                None => out.push(Token {
                    start: ix,
                    end: ix + c.len_utf8(),
                    space_end: ix + c.len_utf8(),
                }),
            }
            continue;
        }
        let start = ix;
        let mut end = ix;
        if is_cjk(c) {
            chars.next();
            end += c.len_utf8();
        } else {
            while let Some(&(ix, c)) = chars.peek() {
                if is_break_space(c) || is_cjk(c) {
                    break;
                }
                chars.next();
                end = ix + c.len_utf8();
            }
        }
        // 닫는 부호는 앞 덩어리에 붙인다.
        while let Some(&(ix, c)) = chars.peek() {
            if !is_closing_punctuation(c) {
                break;
            }
            chars.next();
            end = ix + c.len_utf8();
        }
        // 여는 부호만 있는 덩어리 뒤에서는 나누지 않는다(“가 줄 끝에 홀로 남지 않게).
        match out.last_mut() {
            Some(last)
                if last.space_end == start
                    && text[last.start..last.end]
                        .chars()
                        .all(is_opening_punctuation) =>
            {
                last.end = end;
                last.space_end = end;
            }
            _ => out.push(Token {
                start,
                end,
                space_end: end,
            }),
        }
    }
    out
}

/// 표 열 너비. `natural`은 칸 글자를 한 줄로 놓은 폭의 열별 최댓값이다.
/// 모두 들어가면 남는 폭을 열마다 똑같이 더한다. 넘치면 몫(남은 폭 ÷ 남은 열)보다 좁은 열은 제 폭을
/// 쓰고, 나머지 열이 남은 폭을 제 폭에 비례해 나눈다.
pub(in crate::app) fn column_widths(natural: &[f32], available: f32) -> Vec<f32> {
    let count = natural.len();
    if count == 0 {
        return Vec::new();
    }
    let total: f32 = natural.iter().sum();
    if total <= available {
        let extra = (available - total) / count as f32;
        return natural.iter().map(|width| width + extra).collect();
    }
    let mut fixed = vec![false; count];
    let mut rest = available;
    loop {
        let open = fixed.iter().filter(|fixed| !**fixed).count();
        let share = rest / open as f32;
        let narrow: Vec<usize> =
            (0..count).filter(|&ix| !fixed[ix] && natural[ix] <= share).collect();
        if narrow.is_empty() || narrow.len() == open {
            break;
        }
        for ix in narrow {
            fixed[ix] = true;
            rest -= natural[ix];
        }
    }
    let wide: f32 = (0..count).filter(|&ix| !fixed[ix]).map(|ix| natural[ix]).sum();
    (0..count)
        .map(|ix| if fixed[ix] { natural[ix] } else { rest * natural[ix] / wide })
        .collect()
}

/// 한 문단 조각(강제 줄바꿈 사이)을 줄로 나눈다. `x`는 바이트 위치의 가로 위치다.
fn break_segment(
    text: &str,
    x: impl Fn(usize) -> Pixels,
    width: Pixels,
    first_indent: Pixels,
    align: Align,
) -> Vec<Line> {
    let mut tokens = tokens(text);
    let mut lines = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let indent = if lines.is_empty() {
            first_indent
        } else {
            px(0.)
        };
        let available = (width - indent).max(px(1.));
        let start = tokens[i].start;
        let mut j = i;
        while j < tokens.len() {
            let w = x(tokens[j].end) - x(start);
            if w <= available {
                j += 1;
                continue;
            }
            if j == i {
                // 한 덩어리가 폭을 넘으면 글자 단위로 자른다(적어도 한 글자).
                let token = tokens[j];
                let mut cut = token.start;
                for (ix, c) in text[token.start..token.end].char_indices() {
                    let next = token.start + ix + c.len_utf8();
                    if x(next) - x(start) > available && cut > token.start {
                        break;
                    }
                    cut = next;
                }
                if cut < token.end {
                    tokens[j] = Token {
                        start: token.start,
                        end: cut,
                        space_end: cut,
                    };
                    tokens.insert(
                        j + 1,
                        Token {
                            start: cut,
                            ..token
                        },
                    );
                }
                j += 1;
            }
            break;
        }
        let last_line = j >= tokens.len();
        lines.push(align_line(
            &tokens[i..j],
            &x,
            available,
            indent,
            align,
            last_line,
        ));
        i = j;
    }
    lines
}

/// 덩어리들을 한 줄로 놓는다.
fn align_line(
    tokens: &[Token],
    x: &impl Fn(usize) -> Pixels,
    available: Pixels,
    indent: Pixels,
    align: Align,
    last_line: bool,
) -> Line {
    let start = tokens[0].start;
    let end = tokens[tokens.len() - 1].end;
    let used = x(end) - x(start);
    let slack = (available - used).max(px(0.));
    let whole = |offset: Pixels| Line {
        fragments: vec![(indent + offset, start..end)],
    };
    match align {
        Align::Left => whole(px(0.)),
        Align::Center => whole(slack / 2.),
        Align::Right => whole(slack),
        Align::Justify if last_line || tokens.len() < 2 => whole(px(0.)),
        Align::Justify => {
            let spaced = tokens[..tokens.len() - 1]
                .iter()
                .filter(|token| token.space_end > token.end)
                .count();
            // 공백이 있으면 공백 묶음에만, 없으면 덩어리 사이마다 나눈다.
            let gaps = if spaced > 0 { spaced } else { tokens.len() - 1 };
            let extra = slack / gaps as f32;
            let mut fragments: Vec<(Pixels, Range<usize>)> = Vec::new();
            let mut shift = px(0.);
            for (ix, token) in tokens.iter().enumerate() {
                let at = indent + x(token.start) - x(start) + shift;
                // 공백 없이 이어진 덩어리는 한 조각으로 그린다(공백 묶음에만 나눌 때).
                match fragments.last_mut() {
                    Some((_, range)) if spaced > 0 && range.end == token.start => {
                        range.end = token.end;
                    }
                    _ => fragments.push((at, token.start..token.end)),
                }
                let gap = spaced == 0 || token.space_end > token.end;
                if ix + 1 < tokens.len() && gap {
                    shift += extra;
                }
            }
            Line { fragments }
        }
    }
}

#[cfg(test)]
mod tests {
    // `gpui::*`의 `test` 매크로와 겹치지 않도록 하나씩 가져온다.
    use super::{Align, INK, break_segment, column_widths, slice_runs};
    use gpui_kit::gpui::{TextRun, font, px};

    /// 글자마다 폭 10인 가짜 측정.
    fn breaks(text: &str, width: f32, indent: f32, align: Align) -> Vec<Vec<(f32, String)>> {
        let positions: Vec<usize> = text
            .char_indices()
            .map(|(ix, _)| ix)
            .chain([text.len()])
            .collect();
        let x = |ix: usize| px(positions.iter().position(|&p| p == ix).unwrap() as f32 * 10.);
        break_segment(text, x, px(width), px(indent), align)
            .into_iter()
            .map(|line| {
                line.fragments
                    .into_iter()
                    .map(|(x, range)| (f32::from(x), text[range].to_string()))
                    .collect()
            })
            .collect()
    }

    #[test]
    fn breaks_at_spaces_and_indents_first_line() {
        let lines = breaks("aa bb cc dd", 60., 20., Align::Left);
        assert_eq!(
            lines,
            vec![
                vec![(20., "aa".to_string())],
                vec![(0., "bb cc".to_string())],
                vec![(0., "dd".to_string())],
            ]
        );
    }

    #[test]
    fn justify_spreads_spaces_except_last_line() {
        // "aa bb" = 50, 폭 70 → 공백 하나에 20을 더한다.
        let lines = breaks("aa bb cc", 70., 0., Align::Justify);
        assert_eq!(
            lines,
            vec![
                vec![(0., "aa".to_string()), (50., "bb".to_string())],
                vec![(0., "cc".to_string())],
            ]
        );
    }

    #[test]
    fn cjk_breaks_between_characters_and_keeps_closing_marks() {
        let lines = breaks("漢字漢字。漢", 30., 0., Align::Left);
        assert_eq!(
            lines,
            vec![
                vec![(0., "漢字漢".to_string())],
                vec![(0., "字。漢".to_string())],
            ]
        );
    }

    #[test]
    fn hangul_breaks_between_syllables_with_punctuation_attached() {
        let lines = breaks("“가나다.” 라", 40., 0., Align::Left);
        assert_eq!(
            lines,
            vec![
                // `다.”`는 한 덩어리라 `“가나` 뒤에서 나뉜다.
                vec![(0., "“가나".to_string())],
                vec![(0., "다.”".to_string())],
                vec![(0., "라".to_string())],
            ]
        );
    }

    #[test]
    fn long_words_are_cut() {
        let lines = breaks("abcdefgh", 30., 0., Align::Left);
        let texts: Vec<_> = lines.iter().map(|line| line[0].1.clone()).collect();
        assert_eq!(texts, vec!["abc", "def", "gh"]);
    }

    #[test]
    fn center_and_right_alignment() {
        assert_eq!(
            breaks("ab", 60., 0., Align::Center),
            vec![vec![(20., "ab".to_string())]]
        );
        assert_eq!(
            breaks("ab", 60., 0., Align::Right),
            vec![vec![(40., "ab".to_string())]]
        );
    }

    #[test]
    fn runs_are_sliced() {
        let run = |len| TextRun {
            len,
            font: font("a"),
            color: INK,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let runs = vec![run(3), run(4), run(2)];
        let lens: Vec<_> = slice_runs(&runs, 2..8).iter().map(|r| r.len).collect();
        assert_eq!(lens, vec![1, 4, 1]);
    }

    #[test]
    fn column_widths_follow_content() {
        // 다 들어가면 남는 폭을 똑같이 더한다.
        assert_eq!(column_widths(&[10., 50.], 100.), [30., 70.]);
        // 넘치면 좁은 열은 제 폭, 넓은 열이 나머지를 비례해 나눈다.
        assert_eq!(column_widths(&[20., 300., 150.], 170.), [20., 100., 50.]);
        // 모두 몫보다 넓으면 전부 비례한다.
        assert_eq!(column_widths(&[200., 200.], 100.), [50., 50.]);
        // 좁은 열을 빼면 몫이 커져 다음 열도 제 폭에 들어간다.
        assert_eq!(column_widths(&[10., 45., 200.], 120.), [10., 45., 65.]);
    }
}
