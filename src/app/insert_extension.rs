//! 확장 삽입 대화상자(ADR-0026): 그림을 그리는 확장을 미리보기와 설명을 보고 골라 커서 자리에
//! `` ```ext `` 블록으로 넣는다.
//!
//! 목록·설명·인자 설명은 확장 앞머리(`extension::Meta`)에서 읽는다. 인자마다 입력 칸이 있고 처음 값은
//! 앞머리의 `example`이다. 미리보기는 입력한 값으로 실제 `draw`를 불러 그리고(에디터 블록과 같은 그리기),
//! 값을 고치면 바로 다시 그린다. 넣는 블록의 인자도 입력한 값이다(빈 칸은 빼서 확장의 기본값을 쓴다).

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, FocusableExt as _, Sizable as _, WindowExt as _};
use gpui_kit::gpui::prelude::FluentBuilder as _;
use gpui_kit::gpui::*;

use super::WriterApp;
use super::editor_extension::{Frame, Player, Ticker, fit, scaled, shapes_canvas, theme};
use super::style::Palette;
use crate::InsertExtension;
use crate::extension::{self, Host, Meta};

/// 대화상자 폭.
const DIALOG_WIDTH: f32 = 720.0;

/// 목록 칸 폭.
const LIST_WIDTH: f32 = 220.0;

/// 미리보기 칸 크기.
const PREVIEW_WIDTH: f32 = 440.0;
const PREVIEW_HEIGHT: f32 = 220.0;

/// 인자 입력 칸 폭.
const ARG_INPUT_WIDTH: f32 = 150.0;

/// 고른 확장의 인자 하나: 이름, 앞머리의 설명, 입력 칸.
struct ArgField {
    key: String,
    description: String,
    input: Entity<InputState>,
}

/// 그림을 그리는 확장 목록과 고른 것.
struct ExtensionPicker {
    host: Host,
    items: Vec<Meta>,
    selected: usize,
    /// 미리보기 캔버스. 확장을 고를 때마다 처음부터 다시 그린다.
    player: Player,
    ticker: Ticker,
    /// 고른 확장의 인자 칸. 앞머리의 `args` 순서, 그다음 `args`에 없는 `example` 인자.
    fields: Vec<ArgField>,
    _subscriptions: Vec<Subscription>,
}

impl ExtensionPicker {
    fn new(root: &std::path::Path, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let host = Host::new(root);
        // 블록으로 넣을 수 있는 것(`draw`가 있는 것)만 보인다. 읽지 못하는 확장도 보여 까닭을 알린다.
        let items = extension::catalog(root)
            .into_iter()
            .filter(|meta| {
                host.get(&meta.name)
                    .as_ref()
                    .as_ref()
                    .map_or(true, |worker| worker.info.draws)
            })
            .collect();
        let mut picker = Self {
            host,
            items,
            selected: 0,
            player: Player::new(),
            ticker: Ticker::default(),
            fields: Vec::new(),
            _subscriptions: Vec::new(),
        };
        picker.select(0, window, cx);
        picker
    }

    fn selected(&self) -> Option<&Meta> {
        self.items.get(self.selected)
    }

    /// 확장 `ix`를 고르고 인자 칸을 `example` 값으로 채운다.
    fn select(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = ix;
        self.player = Player::new();
        self.fields.clear();
        self._subscriptions.clear();
        let Some(meta) = self.items.get(ix).cloned() else {
            return;
        };
        let example = |key: &str| {
            meta.example
                .iter()
                .find(|(arg, _)| arg == key)
                .map(|(_, value)| value.clone())
                .unwrap_or_default()
        };
        let extra = meta
            .example
            .iter()
            .filter(|(key, _)| {
                key != extension::SCALE_ARG && !meta.args.iter().any(|(arg, _)| arg == key)
            })
            .map(|(key, _)| (key.clone(), String::new()));
        // 배율은 앱이 처리하므로 어느 확장에나 있다.
        let scale = (
            extension::SCALE_ARG.to_string(),
            t!("extensions.scale_arg").to_string(),
        );
        let fields = meta
            .args
            .iter()
            .filter(|(key, _)| key != extension::SCALE_ARG)
            .cloned()
            .chain(extra)
            .chain(std::iter::once(scale));
        for (key, description) in fields {
            let value = example(&key);
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(t!("extensions.arg_default"))
                    .default_value(value)
            });
            self._subscriptions
                .push(cx.subscribe(&input, |_, _, event: &InputEvent, cx| {
                    if let InputEvent::Change = event {
                        cx.notify();
                    }
                }));
            self.fields.push(ArgField {
                key,
                description,
                input,
            });
        }
        cx.notify();
    }

    /// 입력한 인자 값. 빈 칸은 뺀다(확장이 기본값을 쓴다).
    fn args(&self, cx: &App) -> Vec<(String, String)> {
        self.fields
            .iter()
            .filter_map(|field| {
                let value = field.input.read(cx).value().trim().to_string();
                (!value.is_empty()).then(|| (field.key.clone(), value))
            })
            .collect()
    }

    /// 인자 칸을 앞머리의 `example` 값으로 되돌린다.
    fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.select(self.selected, window, cx);
    }
}

/// 확장 `name`을 `args`로 그린 미리보기. `width`×`height` 칸 가운데에 비율을 지켜 맞춘다.
/// 읽기·그리기 오류는 칸에 글로 보인다. 확장 관리 대화상자도 쓴다.
#[allow(clippy::too_many_arguments)]
pub(super) fn render_preview(
    host: &Host,
    player: &Player,
    ticker: &Ticker,
    name: &str,
    args: &[(String, String)],
    (width, height): (f32, f32),
    pal: Palette,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let loaded = host.get(name);
    let scale = extension::scale_of(args);
    let args = &extension::without_scale(args);
    let message = |text: String| {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .p_4()
            .text_xs()
            .text_color(pal.danger)
            .child(text)
            .into_any_element()
    };
    let canvas_size = match &*loaded {
        Ok(worker) => worker.size(args),
        Err(err) => Err(err.clone()),
    };
    let canvas_size = match canvas_size {
        Ok(canvas_size) => canvas_size,
        Err(err) => return message(err),
    };
    let shapes = match player.frame(&loaded, args, theme(pal), ticker, window, cx) {
        Frame::Shapes(shapes) => Some(shapes),
        Frame::Pending => None,
        Frame::Error(err) => return message(err),
    };
    let (fit_width, fit_height) = fit(scaled(canvas_size, scale), px(width));
    // 높이도 칸에 맞춘다(세로로 긴 캔버스).
    let shrink = (height / f32::from(fit_height)).min(1.0);
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .child(shapes_canvas(
            shapes,
            player.cache(),
            None,
            canvas_size,
            fit_width * shrink,
            fit_height * shrink,
        ))
        .into_any_element()
}

impl Render for ExtensionPicker {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pal = Palette::from_theme(cx.theme());
        if self.items.is_empty() {
            return div()
                .h(px(PREVIEW_HEIGHT))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_1()
                .text_sm()
                .text_color(pal.muted)
                .child(t!("extensions.no_drawing"))
                .child(t!("extensions.ask_assistant_example"))
                .into_any_element();
        }
        let list = div()
            .id("extension-list")
            .w(px(LIST_WIDTH))
            .h(px(PREVIEW_HEIGHT + 120.))
            .flex_none()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_0p5()
            .children(self.items.iter().enumerate().map(|(ix, meta)| {
                let selected = ix == self.selected;
                div()
                    .id(("extension-item", ix))
                    .px_2()
                    .py_1p5()
                    .rounded_md()
                    .cursor_pointer()
                    .when(selected, |this| this.bg(pal.active))
                    .when(!selected, |this| this.hover(|this| this.bg(pal.hover)))
                    .on_click(cx.listener(move |picker, _, window, cx| {
                        if picker.selected != ix {
                            picker.select(ix, window, cx);
                        }
                    }))
                    .child(div().text_sm().child(meta.name.clone()))
                    .child(
                        div()
                            .text_xs()
                            .text_color(pal.muted)
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(meta.description.clone()),
                    )
            }));
        let meta = self.selected().cloned().unwrap_or_default();
        let args = self.args(cx);
        let preview = render_preview(
            &self.host,
            &self.player,
            &self.ticker,
            &meta.name,
            &args,
            (PREVIEW_WIDTH, PREVIEW_HEIGHT),
            pal,
            window,
            cx,
        );
        // 순서는 상관없이 예시 값과 다른지 본다(칸은 `args` 순서, 예시는 적힌 순서다).
        let changed = {
            let sorted = |mut pairs: Vec<(String, String)>| {
                pairs.sort();
                pairs
            };
            sorted(args.clone()) != sorted(meta.example.clone())
        };
        let details = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .h(px(PREVIEW_HEIGHT))
                    .rounded_md()
                    .border_1()
                    .border_color(pal.border)
                    .bg(pal.background)
                    .overflow_hidden()
                    .child(preview),
            )
            .child(
                div()
                    .text_sm()
                    .when(meta.description.is_empty(), |this| {
                        this.text_color(pal.muted)
                            .child(t!("extensions.no_description"))
                    })
                    .child(meta.description.clone()),
            )
            .when(!self.fields.is_empty(), |this| {
                this.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .text_xs()
                        .text_color(pal.muted)
                        .child(t!("extensions.args"))
                        .when(changed, |this| {
                            this.child(
                                div()
                                    .id("extension-args-reset")
                                    .cursor_pointer()
                                    .hover(|this| this.text_color(pal.foreground))
                                    .on_click(cx.listener(|picker, _, window, cx| {
                                        picker.reset(window, cx);
                                    }))
                                    .child(t!("extensions.reset_args")),
                            )
                        }),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .children(self.fields.iter().map(|field| {
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .text_xs()
                                .child(
                                    div()
                                        .w(px(72.))
                                        .flex_none()
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .font_family("monospace")
                                        .child(field.key.clone()),
                                )
                                .child(
                                    div()
                                        .w(px(ARG_INPUT_WIDTH))
                                        .flex_none()
                                        .child(Input::new(&field.input).small().focus_ring(false)),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .text_color(pal.muted)
                                        .child(field.description.clone()),
                                )
                        })),
                )
            });
        div()
            .flex()
            .gap_3()
            .child(list)
            .child(details)
            .into_any_element()
    }
}

impl WriterApp {
    /// 확장 삽입 대화상자를 연다. 열린 시트가 없으면 안내만 한다.
    pub(super) fn on_insert_extension(
        &mut self,
        _: &InsertExtension,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(root) = self.library.as_ref().map(|lib| lib.root.clone()) else {
            return;
        };
        let Some(editor) = &self.editor else {
            self.notify(t!("common.open_sheet_first"), window, cx);
            return;
        };
        let target = editor.state().clone();
        let app = cx.entity().downgrade();
        let picker = cx.new(|cx| ExtensionPicker::new(&root, window, cx));
        // 고른 확장을 입력한 인자로 넣는다. 넣었으면 `true`(대화상자를 닫는다).
        let submit = {
            let picker = picker.clone();
            std::rc::Rc::new(move |window: &mut Window, cx: &mut App| {
                let (name, args) = {
                    let picker = picker.read(cx);
                    let Some(meta) = picker.selected() else {
                        return false;
                    };
                    (meta.name.clone(), picker.args(cx))
                };
                app.update(cx, |app, cx| {
                    app.insert_extension(&target, &name, &args, window, cx)
                })
                .ok();
                true
            })
        };
        window.open_dialog(cx, move |dialog, _, cx| {
            let has_items = !picker.read(cx).items.is_empty();
            let (enter, ok) = (submit.clone(), submit.clone());
            dialog
                .w(px(DIALOG_WIDTH))
                .title(t!("extensions.insert_title"))
                .close_button(false)
                // 인자 칸의 Enter는 대화상자의 확인으로 올라온다.
                .on_ok(move |_, window, cx| enter(window, cx))
                .child(picker.clone())
                .footer(
                    DialogFooter::new()
                        .child(
                            Button::new("insert-extension-cancel")
                                .label(t!("common.cancel"))
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .when(has_items, |footer| {
                            footer.child(
                                Button::new("insert-extension-ok")
                                    .primary()
                                    .label(t!("common.insert"))
                                    .on_click(move |_, window, cx| {
                                        if ok(window, cx) {
                                            window.close_dialog(cx);
                                        }
                                    }),
                            )
                        }),
                )
        });
    }

    /// 대화상자를 연 시트(`target`)가 아직 열려 있으면 커서 자리에 블록을 넣는다. 블록은 제 줄에 둔다.
    fn insert_extension(
        &mut self,
        target: &Entity<gpui_kit::component::input::EditorState>,
        name: &str,
        args: &[(String, String)],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = &self.editor else {
            return;
        };
        if editor.state() != target {
            self.notify(t!("extensions.sheet_closed"), window, cx);
            return;
        }
        editor.state().update(cx, |state, cx| {
            let text = state.value();
            let at = state.selected_range().end.min(text.len());
            let block = extension::block_text(name, args);
            let mut inserted = String::new();
            if at > 0 && !text[..at].ends_with('\n') {
                inserted.push('\n');
            }
            inserted.push_str(&block);
            // 커서는 블록 다음 줄의 처음에 둔다. 커서가 블록 밖에 있어야 펜스가 접혀 그림이 보인다.
            let cursor = if text[at..].starts_with('\n') {
                at + inserted.len() + 1
            } else {
                inserted.push('\n');
                at + inserted.len()
            };
            state.set_selected_range(at..at, cx);
            state.replace(inserted, window, cx);
            state.set_selected_range(cursor..cursor, cx);
            state.focus(window, cx);
        });
        self.on_editor_changed(window, cx);
    }
}
