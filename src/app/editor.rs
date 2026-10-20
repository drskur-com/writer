//! 에디터 영역: 떠 있는 툴바 + 본문.

use gpui_kit::component::*;
use gpui_kit::gpui::prelude::{FluentBuilder, StatefulInteractiveElement as _};
use gpui_kit::gpui::*;

use super::WriterApp;
use super::sheet_list::sheet_label;
use super::style::*;
use crate::{TogglePreview, icons};

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

        let body: AnyElement = match self
            .selected_sheet
            .as_ref()
            .and_then(|path| self.find_sheet(path))
        {
            Some(sheet) => {
                let preview = self.previews.get(&sheet.path).map(|(_, preview)| preview);
                let is_empty = preview.is_some_and(|preview| preview.is_empty());
                div()
                    .flex_1()
                    .px(px(48.0))
                    .py_6()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .text_2xl()
                            .when(is_empty, |this| this.text_color(pal.muted))
                            .when(!is_empty, |this| {
                                this.font_bold().text_color(pal.foreground)
                            })
                            .child(sheet_label(preview)),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(pal.muted)
                            .child("에디터 연결은 M2에서 진행합니다."),
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
