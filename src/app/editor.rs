//! 에디터 영역: 떠 있는 툴바 + 본문, 열린 시트의 편집·자동 저장.

use gpui_base::input::{LineContext, LineStyle};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{
    Editor, EditorState, InputEvent, TextDecoration, TextDecorationCollection,
};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{ActiveTheme as _, Icon};
use gpui_kit::gpui::prelude::{FluentBuilder as _, StatefulInteractiveElement as _};
use gpui_kit::gpui::*;
use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use super::WriterApp;
use super::editor_image::ImageBlocks;
use super::editor_layout::{BookLayout, PAGE_BREAK_HEIGHT};
use super::style::*;
use crate::markdown::{self, Span, SpanKind};
use crate::{TogglePreview, icons, library};

/// 마지막 입력 후 자동 저장까지 기다리는 시간.
const AUTOSAVE_DELAY: Duration = Duration::from_secs(1);

/// 마지막 입력 후 마크업 표시를 다시 계산하기까지 기다리는 시간.
/// 그 사이 기존 표시는 편집을 따라 움직인다.
const MARKUP_DELAY: Duration = Duration::from_millis(150);

/// 마크업 기호(`#`, `**`, `[]()` 등)를 흐리게 하는 정도.
const MARKER_FADE: f32 = 0.6;

/// 인용 본문을 흐리게 하는 정도.
const QUOTE_FADE: f32 = 0.3;

/// 본문 최대 폭(가독 폭). 창이 넓으면 가운데에 둔다.
pub(super) const EDITOR_MAX_WIDTH: f32 = 680.0;

/// 본문 글자 크기.
const EDITOR_FONT_SIZE: f32 = 16.0;

/// 커서를 화면 위·아래 끝에서 떨어뜨려 두는 줄 수. 마지막 줄 아래에도 그만큼 빈 공간을 둬서
/// 문서 끝에서 입력할 때도 이 여백이 유지된다.
const CURSOR_MARGIN_LINES: usize = 3;

/// 본문 좌우 최소 여백.
const EDITOR_MIN_SIDE_PADDING: f32 = 24.0;

fn configure_text_transform(state: &mut EditorState, styles: Option<&library::Styles>) {
    let options = styles
        .map(|styles| styles.input_transform.clone())
        .unwrap_or_default();
    let paste_options = options.clone();
    if options.is_enabled() {
        state.set_typing_transform(move |document, range, inserted| {
            crate::text_transform::on_input(document, range, inserted, &options)
        });
    } else {
        state.clear_typing_transform();
    }
    state.set_paste_transform(move |text, same_format| {
        crate::text_transform::paste(text, same_format, &paste_options)
    });
}

/// 에디터에 열린 시트.
pub(super) struct OpenSheet {
    path: PathBuf,
    state: Entity<EditorState>,
    /// 저장하지 않은 변경이 있는지.
    dirty: bool,
    /// 마지막으로 읽거나 저장한 시점의 파일 수정 시각. 외부 수정 감지에 쓴다.
    disk_modified: u128,
    words: usize,
    /// 자동 저장 대기 태스크. 입력이 오면 새로 만들어 이전 대기를 취소한다.
    autosave: Option<Task<()>>,
    markup: Markup,
    /// 마크업 재계산 대기 태스크(자동 저장과 같은 방식).
    markup_refresh: Option<Task<()>>,
    /// 지금 적용한 좌우 여백. 영역 폭이 바뀔 때만 다시 넣는다.
    side_padding: Rc<Cell<Pixels>>,
    /// 시트가 든 프로젝트 폴더(프로젝트 밖이면 `None`).
    project: Option<PathBuf>,
    /// 적용한 책 스타일. 서체·줄 간격·제목 크기를 에디터에 반영한다.
    /// 프로젝트 밖(메모)이면 `None`: 책 스타일을 쓰지 않는다.
    styles: Option<library::Styles>,
    _subscription: Subscription,
}

/// 줄 스타일: 책 스타일대로 문단·제목을 배치한다(ADR-0010). 제목 줄 확대는 gpui-base 패치(ADR-0006),
/// 이미지 줄은 커서가 들어올 때만 글자를 보인다(ADR-0008). 책 스타일이 없으면 기본 배치다.
fn line_style(styles: Option<&library::Styles>) -> impl Fn(LineContext<'_>) -> LineStyle + 'static {
    let layout = book_layout(styles);
    move |line| layout.line_style(line)
}

fn book_layout(styles: Option<&library::Styles>) -> BookLayout {
    styles.map_or_else(
        || BookLayout::plain(EDITOR_FONT_SIZE),
        |styles| BookLayout::new(styles, EDITOR_FONT_SIZE),
    )
}

/// 제목 레벨(H1~H6)별 굵기. 책 스타일이 없거나 부·장·절이 아니면 굵게.
fn heading_weights(styles: Option<&library::Styles>) -> [FontWeight; 6] {
    std::array::from_fn(|index| {
        let target = library::StyleTarget::for_heading(index as u8 + 1);
        match (styles, target) {
            (Some(styles), Some(target)) => {
                FontWeight(f32::from(styles.resolve(target).font_weight))
            }
            _ => FontWeight::BOLD,
        }
    })
}

/// "여기서 페이지가 나뉩니다" 표시: 가운데 글자 양옆으로 가는 선.
fn page_break_mark(size: Size<Pixels>, cx: &App) -> AnyElement {
    let pal = Palette::from_theme(cx.theme());
    let rule = || div().h(px(1.0)).flex_1().bg(pal.panel_border);
    div()
        .w(size.width)
        .h(size.height)
        .flex()
        .items_center()
        .gap_2()
        .text_xs()
        .text_color(pal.muted)
        .child(rule())
        .child("여기서 페이지가 나뉩니다")
        .child(rule())
        .into_any_element()
}

/// 마크다운 마크업 표시. 겹치는 스팬이 같은 스타일 속성을 다투지 않도록 종류별로 나눈다.
/// 먼저 만든 묶음이 우선한다.
struct Markup {
    /// ATX 제목 굵기와 `#` 흐림. 줄 크기와 함께 바뀌도록 키 입력마다 즉시 다시 계산한다.
    headings: TextDecorationCollection,
    /// 제목 레벨(H1~H6)별 굵기. 책 스타일이 있으면 H1~H3은 부·장·절 굵기를 쓴다.
    heading_weights: [FontWeight; 6],
    /// 기호 흐림.
    markers: TextDecorationCollection,
    /// 굵게·기울임·취소선·인라인 코드·링크.
    inline: TextDecorationCollection,
    /// setext 제목·인용·코드 블록.
    blocks: TextDecorationCollection,
}

impl Markup {
    fn new(state: &Entity<EditorState>, styles: Option<&library::Styles>, cx: &mut App) -> Self {
        state.update(cx, |state, cx| Self {
            headings: state.create_decorations_collection(Vec::new(), cx),
            heading_weights: heading_weights(styles),
            markers: state.create_decorations_collection(Vec::new(), cx),
            inline: state.create_decorations_collection(Vec::new(), cx),
            blocks: state.create_decorations_collection(Vec::new(), cx),
        })
    }

    /// ATX 제목 표시를 원문에서 바로 다시 계산한다. 줄 텍스트만 보므로 긴 문서도 가볍다.
    fn apply_headings(&self, text: &str, cx: &mut App) {
        let decorations = markdown::heading_spans(text)
            .into_iter()
            .map(|span| {
                let style = match span.kind {
                    SpanKind::Marker => HighlightStyle {
                        fade_out: Some(MARKER_FADE),
                        ..Default::default()
                    },
                    SpanKind::Heading(level) => self.heading_weights[usize::from(level) - 1].into(),
                    _ => FontWeight::BOLD.into(),
                };
                TextDecoration::new(span.range, style)
            })
            .collect();
        self.headings.set(decorations, cx);
    }

    /// 파서 스팬으로 표시를 바꾼다. 색은 지금 테마에서 가져온다.
    fn apply(&self, spans: Vec<Span>, cx: &mut App) {
        let theme = cx.theme();
        let code_background = theme.muted;
        let link = theme.link;
        let (mut markers, mut inline, mut blocks) = (Vec::new(), Vec::new(), Vec::new());
        for span in spans {
            let (target, style) = match span.kind {
                SpanKind::Marker => (
                    &mut markers,
                    HighlightStyle {
                        fade_out: Some(MARKER_FADE),
                        ..Default::default()
                    },
                ),
                SpanKind::Strong => (&mut inline, FontWeight::BOLD.into()),
                SpanKind::Emphasis => (&mut inline, FontStyle::Italic.into()),
                SpanKind::Strikethrough => (
                    &mut inline,
                    HighlightStyle {
                        strikethrough: Some(StrikethroughStyle {
                            thickness: px(1.0),
                            color: None,
                        }),
                        ..Default::default()
                    },
                ),
                SpanKind::InlineCode => (
                    &mut inline,
                    HighlightStyle {
                        background_color: Some(code_background),
                        ..Default::default()
                    },
                ),
                SpanKind::Link => (
                    &mut inline,
                    HighlightStyle {
                        color: Some(link),
                        underline: Some(UnderlineStyle {
                            thickness: px(1.0),
                            color: None,
                            wavy: false,
                        }),
                        ..Default::default()
                    },
                ),
                // setext 제목. ATX 제목은 `apply_headings`가 맡는다.
                SpanKind::Heading(_) => (&mut blocks, FontWeight::BOLD.into()),
                SpanKind::Quote => (
                    &mut blocks,
                    HighlightStyle {
                        fade_out: Some(QUOTE_FADE),
                        ..Default::default()
                    },
                ),
                SpanKind::CodeBlock => (
                    &mut blocks,
                    HighlightStyle {
                        background_color: Some(code_background),
                        ..Default::default()
                    },
                ),
            };
            target.push(TextDecoration::new(span.range, style));
        }
        self.markers.set(markers, cx);
        self.inline.set(inline, cx);
        self.blocks.set(blocks, cx);
    }
}

impl WriterApp {
    /// 에디터 영역: 떠 있는 툴바 + 본문(빈 상태 또는 선택한 시트).
    pub(super) fn render_editor_column(
        &self,
        cx: &mut Context<Self>,
        pal: Palette,
    ) -> impl IntoElement {
        let toolbar = window_drag_area(
            div()
                .h(px(TOOLBAR_HEIGHT))
                .flex_shrink_0()
                .px_3()
                .flex()
                .items_center()
                .gap_2(),
        )
        // 시트 목록 칸이 숨은 책 스타일 화면에서는 패널 다시 열기 버튼을 여기 둔다.
        .when(self.book_style.is_some() && !self.sidebar_visible, |this| {
            #[cfg(target_os = "macos")]
            let this = this.pl(px(TRAFFIC_LIGHT_SPACE));
            this.child(tool_group(pal).child(
                icon_button("show-sidebar", icons::panel(), true, pal).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.sidebar_visible = true;
                        cx.notify();
                    },
                )),
            ))
        })
        .child(
            tool_group(pal)
                .child(icon_button("nav-back", icons::back(), false, pal))
                .child(icon_button("nav-forward", icons::forward(), false, pal)),
        )
        .child(tool_group(pal).child(icon_button("search", icons::search(), false, pal)))
        .child(div().flex_1())
        // 파일 뷰어에서는 편집 도구 대신 파일 열기·보기·정보 버튼을 둔다.
        .map(|this| match &self.open_file {
            Some(file) => this.child(self.file_actions(file, pal)),
            // 책 스타일 화면에는 편집 도구가 없다.
            None if self.book_style.is_some() => this,
            None => this
                .child(
                    tool_group(pal)
                        .child(icon_button("format-bold", icons::bold(), false, pal))
                        .child(icon_button("format-italic", icons::italic(), false, pal))
                        .child(icon_button("format-link", icons::link(), false, pal))
                        .child(self.line_kind_button(cx, pal)),
                )
                .child(
                    tool_group(pal)
                        .child(
                            icon_button("editor-compile", icons::build(), self.editor_selection().is_some(), pal).on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.open_selection_compile_dialog(window, cx);
                                }),
                            ),
                        )
                        .child(
                            icon_button("toggle-preview", icons::book_preview(), self.editor_selection().is_some(), pal)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.on_toggle_preview(&TogglePreview, window, cx)
                                })),
                        ),
                ),
        });

        let body: AnyElement = match &self.editor {
            None if let Some(view) = &self.book_style => view.clone().into_any_element(),
            // 파일 그룹의 파일을 골랐으면 에디터 대신 뷰어를 보여 준다.
            None if let Some(file) = &self.open_file => self.render_file_viewer(file, pal),
            Some(editor) => {
                let line_height = book_layout(editor.styles.as_ref()).line_height();
                let font = editor
                    .styles
                    .as_ref()
                    .and_then(|styles| styles.resolve(library::StyleTarget::Body).font)
                    .map_or_else(
                        || crate::settings::Prefs::editor_font_family(cx),
                        SharedString::from,
                    );
                div()
                    .flex_1()
                    .min_h_0()
                    .relative()
                    // 입력창은 영역을 꽉 채워 스크롤바가 오른쪽 끝에 붙게 하고,
                    // 본문은 좌우 여백으로 가독 폭(EDITOR_MAX_WIDTH) 가운데에 둔다.
                    .child(
                        // 코드 에디터 컴포넌트지만 산문용으로 글꼴·줄 높이를 덮어쓴다.
                        Editor::new(&editor.state)
                            .appearance(false)
                            .bordered(false)
                            .size_full()
                            .font_family(font)
                            .text_size(px(EDITOR_FONT_SIZE))
                            .line_height(relative(line_height as f32)),
                    )
                    // 영역 폭을 재서 여백을 맞춘다(사이드바 애니메이션 중에도 따라감).
                    .child({
                        let state = editor.state.clone();
                        let applied = editor.side_padding.clone();
                        canvas(
                            move |bounds, _, cx| {
                                let side = ((bounds.size.width - px(EDITOR_MAX_WIDTH)) / 2.0)
                                    .max(px(EDITOR_MIN_SIDE_PADDING));
                                if applied.get() != side {
                                    applied.set(side);
                                    state.update(cx, |state, cx| {
                                        state.set_editor_paddings(Edges {
                                            left: side,
                                            right: side,
                                            ..Default::default()
                                        });
                                        cx.notify();
                                    });
                                }
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .size_full()
                    })
                    // 단어 수는 툴바 아래 오른쪽 위에 둔다(Ulysses).
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .right_6()
                            .text_xs()
                            .text_color(pal.muted)
                            .child(format!("{} 개 단어", editor.words)),
                    )
                    .into_any_element()
            }
            None => div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_2xl()
                .text_color(pal.muted.opacity(0.7))
                // 파일 그룹을 보고 있으면 파일을 고르라고 안내한다.
                .child(if self.viewing_file_group() {
                    "선택한 파일 없음"
                } else {
                    "선택한 시트 없음"
                })
                .into_any_element(),
        };

        div()
            .flex_1()
            .h_full()
            .flex()
            .flex_col()
            .bg(pal.background)
            .child(toolbar)
            .child(body)
    }
}

impl OpenSheet {
    /// 열린 시트 파일 경로.
    pub(super) fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// 편집 상태(본문·커서).
    pub(super) fn state(&self) -> &Entity<EditorState> {
        &self.state
    }
}

/// 커서가 있는 줄의 바이트 범위(줄바꿈 제외).
fn cursor_line(text: &str, cursor: usize) -> std::ops::Range<usize> {
    let cursor = cursor.min(text.len());
    let start = text[..cursor].rfind('\n').map_or(0, |ix| ix + 1);
    let end = text[cursor..]
        .find('\n')
        .map_or(text.len(), |ix| cursor + ix);
    start..end
}

/// 줄 종류 메뉴 항목: (제목 레벨, 이름). 프로젝트에서는 부·장·절(ADR-0009), 메모에서는 제목 1~3.
fn line_kinds(book: bool) -> [(Option<u8>, &'static str); 4] {
    if book {
        [
            (None, "본문"),
            (Some(1), "부 제목"),
            (Some(2), "장 제목"),
            (Some(3), "절 제목"),
        ]
    } else {
        [
            (None, "본문"),
            (Some(1), "제목 1"),
            (Some(2), "제목 2"),
            (Some(3), "제목 3"),
        ]
    }
}

impl WriterApp {
    /// 시트를 선택해 에디터에 연다(`None`이면 선택 해제). 열려 있던 시트는 먼저 저장한다.
    ///
    /// 저장에 실패하면 편집 내용을 잃지 않도록 시트를 바꾸지 않는다.
    pub(super) fn select_sheet(
        &mut self,
        path: Option<PathBuf>,
        focus: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let already_open = self.editor.as_ref().map(|editor| &editor.path) == path.as_ref();
        if !already_open {
            if let Err(err) = self.save_open_sheet(cx) {
                self.notify(format!("시트를 저장할 수 없습니다: {err}"), window, cx);
                return;
            }
            self.editor = None;
            if let Some(path) = &path {
                self.open_sheet(path.clone(), window, cx);
            }
        }
        self.selected_sheet = path;
        self.selected_sheets.clear();
        self.open_file = None;
        self.book_style = None;
        if focus && let Some(editor) = &self.editor {
            editor.state.update(cx, |state, cx| state.focus(window, cx));
        }
        cx.notify();
    }

    /// 시트 파일을 읽어 새 편집 상태를 만든다.
    fn open_sheet(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        // 파일은 표준 마크다운, 에디터는 줄 하나가 문단이다(ADR-0011).
        let text = match library::read_sheet(&path) {
            Ok(text) => crate::text_transform::to_editor(&text),
            Err(err) => {
                self.notify(format!("시트를 열 수 없습니다: {err}"), window, cx);
                return;
            }
        };
        let disk_modified = library::modified_nanos(&path);
        let words = library::count_words(text.chars());
        let project = self.library.as_ref().and_then(|lib| lib.project_of(&path));
        let styles = project.as_deref().map(library::read_styles);
        // 마지막 줄 아래 빈 공간(scroll_beyond_last_line)은 코드 에디터 모드에만 있어서
        // EditorState를 쓰고 코드용 기능(줄 번호·들여쓰기 안내선·괄호 자동 닫기·접기)은 끈다.
        // 빈 공간이 없으면 문서 끝에서 커서 여백을 맞추려 끝 너머로 스크롤했다가 되돌아오며 출렁인다.
        let image_app = cx.entity().downgrade();
        let image_sheet = path.clone();
        let state = cx.new(|cx| {
            let mut state = EditorState::new(window, cx)
                .clipboard_text_transform(
                    "com.drskur.writer.editor-text.v1",
                    crate::text_transform::from_clipboard,
                )
                .line_number(false)
                .indent_guides(false)
                .auto_close(false)
                .smart_indent(false)
                .folding(false)
                .cursor_surrounding_lines(Some(CURSOR_MARGIN_LINES))
                .scroll_beyond_last_line(Some(CURSOR_MARGIN_LINES));
            configure_text_transform(&mut state, styles.as_ref());
            state.set_line_style(Some(line_style(styles.as_ref())), cx);
            // 이미지 줄 아래에 이미지를 그린다(ADR-0008).
            let images = Rc::new(ImageBlocks::new(&path));
            let measure = images.clone();
            // 줄 블록 = 이미지(측정한 몫) + 줄 스타일이 잡은 나머지(페이지 나눔 표시, ADR-0010).
            // 첫 줄 위 블록은 페이지 나눔 표시뿐이다.
            state.set_line_blocks(
                move |line, wrap_width| measure.measure(line, wrap_width),
                move |block, _, cx| {
                    if block.above {
                        return page_break_mark(block.size, cx);
                    }
                    let image = images.measure(&block.text, Some(block.size.width));
                    let mark = block.size.height - image;
                    let render_image = |size| {
                        let app = image_app.clone();
                        let sheet = image_sheet.clone();
                        let line = block.line;
                        let original = block.text.clone();
                        div()
                            .id(("editor-image", line))
                            .cursor_pointer()
                            .on_click(move |event: &ClickEvent, window, cx| {
                                if event.click_count() == 2 {
                                    cx.stop_propagation();
                                    app.update(cx, |app, cx| {
                                        app.edit_image(&sheet, line, &original, window, cx);
                                    })
                                    .ok();
                                }
                            })
                            .child(images.render(&block.text, size, cx))
                            .into_any_element()
                    };
                    if mark < px(PAGE_BREAK_HEIGHT) {
                        return render_image(block.size);
                    }
                    div()
                        .flex()
                        .flex_col()
                        .when(image > px(0.), |this| {
                            this.child(render_image(size(block.size.width, image)))
                        })
                        .child(page_break_mark(size(block.size.width, mark), cx))
                        .into_any_element()
                },
                cx,
            );
            state.set_value(text.clone(), window, cx);
            state
        });
        let markup = Markup::new(&state, styles.as_ref(), cx);
        markup.apply_headings(&text, cx);
        markup.apply(markdown::spans(&text), cx);
        let subscription = cx.subscribe_in(
            &state,
            window,
            |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::Change => this.on_editor_changed(window, cx),
                // 포커스가 빠지면 기다리지 않고 저장한다.
                InputEvent::Blur => {
                    if let Err(err) = this.save_open_sheet(cx) {
                        this.notify(format!("시트를 저장할 수 없습니다: {err}"), window, cx);
                    }
                }
                _ => {}
            },
        );
        self.editor = Some(OpenSheet {
            path,
            state,
            dirty: false,
            disk_modified,
            words,
            autosave: None,
            markup,
            markup_refresh: None,
            side_padding: Rc::new(Cell::new(px(-1.0))),
            project,
            styles,
            _subscription: subscription,
        });
    }

    /// 테마가 바뀌면 열린 시트의 마크업 색을 다시 입힌다.
    pub(super) fn refresh_markup(&mut self, cx: &mut Context<Self>) {
        if let Some(editor) = &self.editor {
            let text = editor.state.read(cx).value();
            editor.markup.apply(markdown::spans(&text), cx);
        }
    }

    /// 본문이 바뀌면 단어 수를 갱신하고 자동 저장·마크업 재계산을 (다시) 예약한다.
    pub(super) fn on_editor_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = &mut self.editor else {
            return;
        };
        editor.dirty = true;
        editor.words = library::count_words(editor.state.read(cx).text().chars());
        // 제목 크기는 엔진이 즉시 바꾸므로 굵기·`#` 흐림도 같은 프레임에 맞춘다.
        let text = editor.state.read(cx).value();
        editor.markup.apply_headings(&text, cx);
        editor.markup_refresh = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(MARKUP_DELAY).await;
            let Ok(text) = this.update(cx, |this, cx| {
                this.editor
                    .as_ref()
                    .map(|editor| editor.state.read(cx).value())
            }) else {
                return;
            };
            let Some(text) = text else {
                return;
            };
            // 긴 문서(2만 단어 ≈ 35ms)도 UI 스레드를 막지 않도록 파싱은 백그라운드에서.
            let spans = cx
                .background_executor()
                .spawn(async move { markdown::spans(&text) })
                .await;
            this.update(cx, |this, cx| {
                if let Some(editor) = &this.editor {
                    editor.markup.apply(spans, cx);
                }
            })
            .ok();
        }));
        editor.autosave = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(AUTOSAVE_DELAY).await;
            this.update_in(cx, |this, window, cx| {
                if let Err(err) = this.save_open_sheet(cx) {
                    this.notify(format!("시트를 저장할 수 없습니다: {err}"), window, cx);
                }
            })
            .ok();
        }));
        cx.notify();
    }

    /// 열린 시트에 저장하지 않은 변경이 있으면 원자적으로 저장한다.
    pub(super) fn save_open_sheet(&mut self, cx: &mut Context<Self>) -> std::io::Result<()> {
        let Some(editor) = &mut self.editor else {
            return Ok(());
        };
        if !editor.dirty {
            return Ok(());
        }
        editor.autosave = None;
        let text = editor.state.read(cx).value();
        editor.disk_modified =
            library::write_sheet(&editor.path, &crate::text_transform::to_markdown(&text))?;
        editor.dirty = false;
        Ok(())
    }

    /// 열린 시트를 저장하고 닫는다(라이브러리 닫기·종료). 실패는 기록만 한다.
    pub(super) fn close_sheet(&mut self, cx: &mut Context<Self>) {
        if let Err(err) = self.save_open_sheet(cx) {
            eprintln!("시트 저장 실패: {err}");
        }
        self.editor = None;
        self.selected_sheet = None;
        self.open_file = None;
        self.book_style = None;
    }

    /// 다시 스캔한 뒤 열린 시트를 맞춘다.
    ///
    /// - 책 스타일이 바뀌었으면 다시 입힌다.
    /// - 파일이 사라졌으면 닫는다.
    /// - 밖에서 바뀌었고 저장하지 않은 변경이 없으면 다시 읽는다.
    ///   (저장하지 않은 변경이 있으면 지금은 그대로 두고 다음 저장이 덮어쓴다.)
    pub(super) fn sync_open_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = &mut self.editor else {
            return;
        };
        if let Some(project) = &editor.project {
            let styles = Some(library::read_styles(project));
            if styles != editor.styles {
                let line_style = line_style(styles.as_ref());
                editor.markup.heading_weights = heading_weights(styles.as_ref());
                editor.styles = styles;
                editor
                    .state
                    .update(cx, |state, cx| {
                        state.set_line_style(Some(line_style), cx);
                        configure_text_transform(state, editor.styles.as_ref());
                    });
                let text = editor.state.read(cx).value();
                editor.markup.apply_headings(&text, cx);
            }
        }
        if !editor.path.is_file() {
            self.editor = None;
            self.selected_sheet = None;
            return;
        }
        let modified = library::modified_nanos(&editor.path);
        if editor.dirty || modified == editor.disk_modified {
            return;
        }
        let Ok(text) =
            library::read_sheet(&editor.path).map(|text| crate::text_transform::to_editor(&text))
        else {
            return;
        };
        editor.words = library::count_words(text.chars());
        editor.disk_modified = modified;
        editor
            .state
            .update(cx, |state, cx| state.set_value(text.clone(), window, cx));
        editor.markup.apply_headings(&text, cx);
        editor.markup.apply(markdown::spans(&text), cx);
    }
}

impl WriterApp {
    /// 툴바의 줄 종류 버튼: 커서가 있는 줄을 본문 또는 부·장·절 제목으로 바꾼다.
    fn line_kind_button(&self, cx: &mut Context<Self>, pal: Palette) -> AnyElement {
        let Some(editor) = &self.editor else {
            return icon_button("format-paragraph", icons::paragraph(), false, pal)
                .into_any_element();
        };
        let (state, book, app) = (
            editor.state.clone(),
            editor.styles.is_some(),
            cx.entity().downgrade(),
        );
        Button::new("format-paragraph")
            .icon(Icon::new(icons::paragraph()).text_color(pal.foreground.opacity(0.8)))
            .ghost()
            .size(px(30.0))
            .rounded(px(15.0))
            .tooltip(if book {
                "본문 · 부 · 장 · 절"
            } else {
                "본문 · 제목"
            })
            .dropdown_menu(move |menu, _, cx| {
                // 메뉴를 열 때 커서 줄을 읽는다.
                let current = {
                    let state = state.read(cx);
                    let text = state.value();
                    let line = &text[cursor_line(&text, state.cursor())];
                    markdown::heading_level(line)
                };
                line_kinds(book)
                    .into_iter()
                    .fold(menu.min_w(px(160.0)), |menu, (level, title)| {
                        let app = app.clone();
                        menu.item(
                            PopupMenuItem::new(title)
                                .checked(current == level)
                                .on_click(move |_, window, cx| {
                                    app.update(cx, |app, cx| {
                                        app.set_line_heading(level, window, cx)
                                    })
                                    .ok();
                                }),
                        )
                    })
            })
            .into_any_element()
    }

    /// 커서가 있는 줄의 제목 레벨을 바꾼다(`None`이면 본문). 커서는 같은 글자 앞에 둔다.
    fn set_line_heading(&mut self, level: Option<u8>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = &self.editor else {
            return;
        };
        let changed = editor.state.update(cx, |state, cx| {
            let text = state.value();
            let cursor = state.cursor().min(text.len());
            let range = cursor_line(&text, cursor);
            let line = &text[range.clone()];
            let new_line = markdown::set_heading_level(line, level);
            if new_line == line {
                state.focus(window, cx);
                return false;
            }
            let column = (cursor - range.start).saturating_sub(markdown::heading_marker_len(line))
                + markdown::heading_marker_len(&new_line);
            let start = range.start;
            state.set_selected_range(range, cx);
            state.replace(new_line, window, cx);
            state.set_selected_range(start + column..start + column, cx);
            state.focus(window, cx);
            true
        });
        if changed {
            self.on_editor_changed(window, cx);
        }
    }
}
