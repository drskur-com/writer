//! 사이드바 2단: 시트 목록(카드, 끌어서 정렬, 순서 애니메이션).

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::*;
use gpui_kit::gpui::prelude::{
    FluentBuilder, InteractiveElement as _, StatefulInteractiveElement as _,
};
use gpui_kit::gpui::*;

use super::library_ops::SortChange;
use super::style::*;
use super::{GroupSort, Selection, WriterApp};
use crate::{NewSheet, icons, library};

/// 시트 카드 높이(제목 한 줄 + 발췌 두 줄 + 여백). 모든 카드가 같아서
/// 위치를 `index * 높이`로 계산하고, 순서가 바뀌면 스프링으로 옮겨 간다.
///
/// gpui의 `text_sm`/`text_xs`는 줄 높이를 바꾸지 않으므로(기본 1.618배) 아래 줄 높이를 직접 지정한다.
/// 12(위 여백) + 20(제목) + 4(간격) + 16×2(발췌) + 12(아래 여백) + 1(구분선) = 81.
const SHEET_ROW_HEIGHT: f32 = 81.0;

/// 카드 제목 줄 높이(px).
const TITLE_LINE_HEIGHT: f32 = 20.0;

/// 카드 발췌 줄 높이(px).
const EXCERPT_LINE_HEIGHT: f32 = 16.0;

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
/// 파일 그룹의 파일을 끌 때도 같은 모양을 쓴다.
#[derive(Clone)]
pub(super) struct DraggedSheet {
    /// 그룹 안에서의 원래 위치.
    pub(super) index: usize,
    pub(super) label: SharedString,
    pub(super) pal: Palette,
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
    /// 시트 목록 제목 오른쪽의 정렬 버튼. 누르면 정렬 방법과(수동이 아니면) 방향을 고르고,
    /// 고른 것은 그룹 메타에 저장한다.
    fn sort_button(
        &self,
        group_path: std::path::PathBuf,
        current: GroupSort,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let app = cx.entity().downgrade();
        let pick = move |path: &std::path::PathBuf, change: SortChange| {
            let (app, path) = (app.clone(), path.clone());
            move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                app.update(cx, |app, cx| app.set_sort(&path, change, window, cx))
                    .ok();
            }
        };
        Button::new("sheet-sort")
            .icon(Icon::new(icons::sort()))
            .ghost()
            .small()
            .tooltip(current.mode.title())
            .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
                let menu =
                    library::SortMode::ALL
                        .into_iter()
                        .fold(menu.min_w(px(160.0)), |menu, mode| {
                            menu.item(
                                PopupMenuItem::new(mode.title())
                                    .checked(mode == current.mode)
                                    .on_click(pick(&group_path, SortChange::Mode(mode))),
                            )
                        });
                let Some((ascending, descending)) = current.mode.direction_titles() else {
                    return menu;
                };
                menu.separator()
                    .item(
                        PopupMenuItem::new(ascending)
                            .checked(!current.descending)
                            .on_click(pick(&group_path, SortChange::Descending(false))),
                    )
                    .item(
                        PopupMenuItem::new(descending)
                            .checked(current.descending)
                            .on_click(pick(&group_path, SortChange::Descending(true))),
                    )
            })
    }

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
        // 파일 그룹(첨부파일 등)이면 시트 대신 파일 목록을 보여 준다.
        let files = match &current {
            Some(Selection::Group(path)) => self
                .library
                .as_ref()
                .and_then(|lib| lib.find_group(path))
                .filter(|group| group.kind == library::GroupKind::Files)
                .map(|group| group.files.clone()),
            _ => None,
        };
        let sheet_group = files.is_none();
        // 그룹의 정렬 방법. 고를 수 없는 그룹(프로젝트 원고)은 버튼을 두지 않는다.
        let sort = match &current {
            Some(Selection::Group(path)) => Some((path.clone(), self.sort_for(path))),
            _ => None,
        };

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
            icon_button("new-sheet", icons::compose(), sheet_group, pal).when(
                sheet_group,
                |this| {
                    this.on_click(
                        cx.listener(|this, _, window, cx| this.on_new_sheet(&NewSheet, window, cx)),
                    )
                },
            ),
        ));

        let list: AnyElement = if let Some(mut files) = files {
            let (path, sort) = sort.clone().expect("파일 그룹은 그룹 선택에서만 나온다");
            self.sort_files(&mut files, sort);
            // 수동 정렬일 때만 끌어서 순서를 바꾼다.
            let manual = (sort.mode == library::SortMode::Manual).then_some(path);
            self.render_file_list(files, manual, cx, pal)
        } else if sheets.is_empty() {
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
            // 수동 정렬인 그룹을 볼 때만 끌어서 순서를 바꾼다("모두"는 수정순 고정).
            let group_path = match &sort {
                Some((path, sort)) if sort.mode == library::SortMode::Manual => Some(path.clone()),
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
                    // 빈 시트는 흐린 기본 굵기로, 카드 가운데에 가볍게 보인다(Ulysses).
                    .when(is_empty, |this| this.justify_center().items_center())
                    // 아직 안 읽은 시트도 한 줄 높이를 차지하게 한다.
                    .child(
                        div()
                            .h(px(TITLE_LINE_HEIGHT))
                            .line_height(px(TITLE_LINE_HEIGHT))
                            .truncate()
                            .when(is_empty, |this| this.text_color(pal.muted))
                            .when(!is_empty, |this| this.font_semibold())
                            .child(label.clone()),
                    )
                    .when(!excerpt.is_empty(), |this| {
                        this.child(
                            div()
                                .text_xs()
                                .line_height(px(EXCERPT_LINE_HEIGHT))
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
                        move |this, _, window, cx| {
                            this.select_sheet(Some(path.clone()), false, window, cx);
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
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .border_b_1()
                    .border_color(pal.panel_border)
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_lg()
                            .font_semibold()
                            .text_color(pal.foreground)
                            .child(title),
                    )
                    .when_some(
                        sort.filter(|(_, sort)| sort.choosable),
                        |this, (path, sort)| this.child(self.sort_button(path, sort, cx)),
                    ),
            )
            .child(list)
    }
}
