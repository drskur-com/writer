//! 사이드바 2단: 시트 목록(카드, 끌어서 정렬, 순서 애니메이션).

use gpui_kit::component::*;
use gpui_kit::gpui::prelude::{
    FluentBuilder, InteractiveElement as _, StatefulInteractiveElement as _,
};
use gpui_kit::gpui::*;

use super::style::*;
use super::{Selection, WriterApp};
use crate::{NewSheet, icons, library};

/// 시트 카드 높이(제목 한 줄 + 발췌 두 줄 + 여백). 모든 카드가 같아서
/// 위치를 `index * 높이`로 계산하고, 순서가 바뀌면 스프링으로 옮겨 간다.
const SHEET_ROW_HEIGHT: f32 = 81.0;

/// 순서가 바뀐 시트 카드가 새 자리로 옮겨 가는 스프링.
const SHEET_ROW_SPRING: SpringConfig = SpringConfig::new(500.0, 45.0, 1.0);

/// 목록/에디터에 표시할 시트 이름. 빈 시트는 "빈 시트", 아직 읽지 않았으면 빈 문자열.
pub(super) fn sheet_label(preview: Option<&library::Preview>) -> String {
    match preview {
        Some(preview) if preview.is_empty() => "빈 시트".to_string(),
        Some(preview) => preview.title.clone(),
        None => String::new(),
    }
}

/// 시트 목록에서 끌고 있는 시트. 끄는 동안 커서를 따라다니는 미리보기로도 그린다.
#[derive(Clone)]
struct DraggedSheet {
    /// 그룹 안에서의 원래 위치.
    index: usize,
    label: SharedString,
    pal: Palette,
}

impl Render for DraggedSheet {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_3()
            .py_2()
            .max_w(px(SHEET_COLUMN_WIDTH - 32.0))
            .truncate()
            .rounded_md()
            .border_1()
            .border_color(self.pal.border)
            .bg(self.pal.background)
            .shadow_md()
            .text_sm()
            .font_semibold()
            .text_color(self.pal.foreground)
            .child(self.label.clone())
    }
}

impl WriterApp {
    /// 사이드바 2단: 선택한 그룹(또는 "모두")의 시트 목록. 항상 보인다.
    pub(super) fn render_sheet_column(
        &self,
        cx: &mut Context<Self>,
        pal: Palette,
    ) -> impl IntoElement {
        let current = self.effective_selection();
        let title = current
            .as_ref()
            .map(|selection| self.selection_label(selection))
            .unwrap_or_else(|| "시트".to_string());
        let sheets = current
            .as_ref()
            .map(|selection| self.sheets_for(selection))
            .unwrap_or_default();

        let toolbar = window_drag_area(
            div()
                .h(px(TOOLBAR_HEIGHT))
                .flex_shrink_0()
                .px_3()
                .flex()
                .items_center()
                .gap_2(),
        )
        // 패널이 접히면 (macOS) 신호등 자리를 비우고 다시 열기 버튼을 둔다.
        .when(!self.sidebar_visible, |this| {
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
        .child(div().flex_1())
        .child(tool_group(pal).child(icon_button("sheet-filter", icons::filter(), false, pal)))
        .child(tool_group(pal).child(
            icon_button("new-sheet", icons::compose(), true, pal).on_click(
                cx.listener(|this, _, window, cx| this.on_new_sheet(&NewSheet, window, cx)),
            ),
        ));

        let list: AnyElement = if sheets.is_empty() {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(pal.muted)
                .child("시트 없음")
                .into_any_element()
        } else {
            // 그룹을 볼 때만 끌어서 순서를 바꾼다("모두"는 수정순 고정).
            let group_path = match &current {
                Some(Selection::Group(path)) => Some(path.clone()),
                _ => None,
            };
            // 다른 목록으로 바꿨을 때 같은 시트가 옛 자리에서 날아오지 않도록 목록별로 키를 나눈다.
            let list_key = match &current {
                Some(Selection::Group(path)) => path.display().to_string(),
                Some(Selection::Section(kind)) => kind.dir().to_string(),
                None => String::new(),
            };
            let mut items: Vec<AnyElement> = Vec::with_capacity(sheets.len());
            for (index, sheet) in sheets.into_iter().enumerate() {
                let path = sheet.path.clone();
                let selected = self.selected_sheet.as_ref() == Some(&path);
                let preview = self.previews.get(&path).map(|(_, preview)| preview);
                let is_empty = preview.is_some_and(|preview| preview.is_empty());
                let excerpt = preview
                    .map(|preview| preview.excerpt.clone())
                    .unwrap_or_default();
                let label = sheet_label(preview);
                // 구분선은 카드 양옆을 비워 두고(본문 글자와 같은 들여쓰기) 긋는다.
                // 선택한 카드는 둥근 배경으로 감싸고 그 카드의 구분선은 숨긴다.
                let content = div()
                    .h_full()
                    .overflow_hidden()
                    .py_3()
                    .border_b_1()
                    .border_color(if selected {
                        gpui_kit::gpui::transparent_black()
                    } else {
                        pal.panel_border
                    })
                    .text_sm()
                    .text_color(pal.foreground)
                    .flex()
                    .flex_col()
                    .gap_1()
                    // 빈 시트는 흐린 기본 굵기로 가볍게 보인다.
                    // 아직 안 읽은 시트도 한 줄 높이를 차지하게 한다.
                    .child(
                        div()
                            .min_h(rems(1.25))
                            .truncate()
                            .when(is_empty, |this| this.text_color(pal.muted))
                            .when(!is_empty, |this| this.font_semibold())
                            .child(label.clone()),
                    )
                    .when(!excerpt.is_empty(), |this| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(pal.muted)
                                .line_clamp(2)
                                .text_ellipsis()
                                .child(excerpt),
                        )
                    });
                let card = div().h_full().px_2().child(
                    div()
                        .h_full()
                        .px_2()
                        .rounded_md()
                        .when(selected, |this| this.bg(pal.active))
                        .child(content),
                );

                // 끌어다 놓을 자리는 카드를 감싼 줄의 위/아래 선으로 보여준다.
                let row = div()
                    .id(SharedString::from(format!("sheet-{}", path.display())))
                    .cursor_pointer()
                    .on_click(cx.listener({
                        let path = path.clone();
                        move |this, _, _, cx| {
                            this.selected_sheet = Some(path.clone());
                            cx.notify();
                        }
                    }))
                    .child(card);
                let row = match &group_path {
                    Some(group_path) => {
                        let group_path = group_path.clone();
                        let dragged = DraggedSheet {
                            index,
                            label: label.into(),
                            pal,
                        };
                        row.on_drag(dragged, |dragged, _, _, cx| cx.new(|_| dragged.clone()))
                            .drag_over::<DraggedSheet>(move |style, dragged, _, _| {
                                if dragged.index < index {
                                    style.border_b_2().border_color(pal.accent)
                                } else if dragged.index > index {
                                    style.border_t_2().border_color(pal.accent)
                                } else {
                                    style
                                }
                            })
                            .on_drop(cx.listener(
                                move |this, dragged: &DraggedSheet, window, cx| {
                                    this.move_sheet(&group_path, dragged.index, index, window, cx);
                                },
                            ))
                    }
                    None => row,
                };
                // 카드는 절대 위치로 둔다. 스프링 상태는 시트 경로로 이어지므로
                // 순서가 바뀌면 이전 자리에서 새 자리로 미끄러져 간다.
                let top = index as f32 * SHEET_ROW_HEIGHT;
                let row = row.absolute().left_0().right_0().h(px(SHEET_ROW_HEIGHT));
                items.push(
                    row.with_spring(
                        SharedString::from(format!("sheet-pos-{list_key}-{}", path.display())),
                        SpringAnimation::new(SHEET_ROW_SPRING).to(top),
                        |this, y| this.top(px(y)),
                    )
                    .into_any_element(),
                );
            }
            let content_height = items.len() as f32 * SHEET_ROW_HEIGHT;
            div()
                .id("sheet-list")
                .flex_1()
                .overflow_y_scroll()
                .child(
                    div()
                        .relative()
                        .w_full()
                        .h(px(content_height))
                        .children(items),
                )
                .into_any_element()
        };

        div()
            .w(px(SHEET_COLUMN_WIDTH))
            .h_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .bg(pal.background)
            .border_r_1()
            .border_color(pal.border)
            .child(toolbar)
            .child(
                div()
                    .px_4()
                    .pb_2()
                    .border_b_1()
                    .border_color(pal.panel_border)
                    .text_lg()
                    .font_semibold()
                    .text_color(pal.foreground)
                    .child(title),
            )
            .child(list)
    }
}
