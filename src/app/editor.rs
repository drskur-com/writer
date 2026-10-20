//! 에디터 영역: 떠 있는 툴바 + 본문, 열린 시트의 편집·자동 저장.

use gpui_kit::component::input::{Editor, EditorState, InputEvent};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::gpui::prelude::StatefulInteractiveElement as _;
use gpui_kit::gpui::*;
use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use super::WriterApp;
use super::style::*;
use crate::{TogglePreview, icons, library};

/// 마지막 입력 후 자동 저장까지 기다리는 시간.
const AUTOSAVE_DELAY: Duration = Duration::from_secs(1);

/// 본문 최대 폭(가독 폭). 창이 넓으면 가운데에 둔다.
const EDITOR_MAX_WIDTH: f32 = 680.0;

/// 본문 글자 크기.
const EDITOR_FONT_SIZE: f32 = 16.0;

/// 본문 줄 높이(글자 크기 대비).
const EDITOR_LINE_HEIGHT: f32 = 1.6;

/// 커서를 화면 위·아래 끝에서 떨어뜨려 두는 줄 수. 마지막 줄 아래에도 그만큼 빈 공간을 둬서
/// 문서 끝에서 입력할 때도 이 여백이 유지된다.
const CURSOR_MARGIN_LINES: usize = 3;

/// 본문 좌우 최소 여백.
const EDITOR_MIN_SIDE_PADDING: f32 = 24.0;

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
    /// 지금 적용한 좌우 여백. 영역 폭이 바뀔 때만 다시 넣는다.
    side_padding: Rc<Cell<Pixels>>,
    _subscription: Subscription,
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
        .child(
            tool_group(pal)
                .child(icon_button("nav-back", icons::back(), false, pal))
                .child(icon_button("nav-forward", icons::forward(), false, pal)),
        )
        .child(tool_group(pal).child(icon_button("search", icons::search(), false, pal)))
        .child(div().flex_1())
        .child(
            tool_group(pal)
                .child(icon_button("format-bold", icons::bold(), false, pal))
                .child(icon_button("format-italic", icons::italic(), false, pal))
                .child(icon_button("format-link", icons::link(), false, pal))
                .child(icon_button(
                    "format-paragraph",
                    icons::paragraph(),
                    false,
                    pal,
                )),
        )
        .child(
            tool_group(pal)
                .child(icon_button("share", icons::share(), false, pal))
                .child(icon_button("export", icons::export(), false, pal)),
        )
        .child(tool_group(pal).child(
            icon_button("toggle-preview", icons::preview(), true, pal).on_click(cx.listener(
                |this, _, window, cx| this.on_toggle_preview(&TogglePreview, window, cx),
            )),
        ));

        let body: AnyElement = match &self.editor {
            Some(editor) => div()
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
                        .font_family(cx.theme().font_family.clone())
                        .text_size(px(EDITOR_FONT_SIZE))
                        .line_height(relative(EDITOR_LINE_HEIGHT)),
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
                .into_any_element(),
            None => div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_2xl()
                .text_color(pal.muted.opacity(0.7))
                .child("선택한 시트 없음")
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
        if focus && let Some(editor) = &self.editor {
            editor.state.update(cx, |state, cx| state.focus(window, cx));
        }
        cx.notify();
    }

    /// 시트 파일을 읽어 새 편집 상태를 만든다.
    fn open_sheet(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let text = match library::read_sheet(&path) {
            Ok(text) => text,
            Err(err) => {
                self.notify(format!("시트를 열 수 없습니다: {err}"), window, cx);
                return;
            }
        };
        let disk_modified = library::modified_nanos(&path);
        let words = library::count_words(text.chars());
        // 마지막 줄 아래 빈 공간(scroll_beyond_last_line)은 코드 에디터 모드에만 있어서
        // EditorState를 쓰고 코드용 기능(줄 번호·들여쓰기 안내선·괄호 자동 닫기·접기)은 끈다.
        // 빈 공간이 없으면 문서 끝에서 커서 여백을 맞추려 끝 너머로 스크롤했다가 되돌아오며 출렁인다.
        let state = cx.new(|cx| {
            let mut state = EditorState::new(window, cx)
                .line_number(false)
                .indent_guides(false)
                .auto_close(false)
                .smart_indent(false)
                .folding(false)
                .cursor_surrounding_lines(Some(CURSOR_MARGIN_LINES))
                .scroll_beyond_last_line(Some(CURSOR_MARGIN_LINES));
            state.set_value(text, window, cx);
            state
        });
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
            side_padding: Rc::new(Cell::new(px(-1.0))),
            _subscription: subscription,
        });
    }

    /// 본문이 바뀌면 단어 수를 갱신하고 자동 저장을 (다시) 예약한다.
    fn on_editor_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = &mut self.editor else {
            return;
        };
        editor.dirty = true;
        editor.words = library::count_words(editor.state.read(cx).text().chars());
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
        editor.disk_modified = library::write_sheet(&editor.path, &text)?;
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
    }

    /// 다시 스캔한 뒤 열린 시트를 맞춘다.
    ///
    /// - 파일이 사라졌으면 닫는다.
    /// - 밖에서 바뀌었고 저장하지 않은 변경이 없으면 다시 읽는다.
    ///   (저장하지 않은 변경이 있으면 지금은 그대로 두고 다음 저장이 덮어쓴다.)
    pub(super) fn sync_open_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = &mut self.editor else {
            return;
        };
        if !editor.path.is_file() {
            self.editor = None;
            self.selected_sheet = None;
            return;
        }
        let modified = library::modified_nanos(&editor.path);
        if editor.dirty || modified == editor.disk_modified {
            return;
        }
        let Ok(text) = library::read_sheet(&editor.path) else {
            return;
        };
        editor.words = library::count_words(text.chars());
        editor.disk_modified = modified;
        editor
            .state
            .update(cx, |state, cx| state.set_value(text, window, cx));
    }
}
