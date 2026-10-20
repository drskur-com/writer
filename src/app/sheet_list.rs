//! 사이드바 2단: 시트 목록(카드, 끌어서 정렬, 순서 애니메이션).

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::*;
use gpui_kit::gpui::prelude::{
    FluentBuilder, InteractiveElement as _, StatefulInteractiveElement as _,
};
use gpui_kit::gpui::*;

use super::library_ops::SortChange;
use super::library_panel::menu_row;
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

/// 접합 표시 선 굵기와, 이어지지 않는 쪽 끝을 카드 안으로 들이는 정도(px).
const GLUE_BAR_WIDTH: f32 = 3.0;
const GLUE_BAR_INSET: f32 = 14.0;

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

/// 끄는 접합 묶음 미리보기에 보일 최대 시트 수. 넘으면 "외 N개"로 줄인다.
const DRAG_PREVIEW_MAX_SHEETS: usize = 5;

/// 시트 목록에서 끌고 있는 시트. 끄는 동안 커서를 따라다니는 미리보기로도 그린다.
/// 파일 그룹의 파일을 끌 때도 같은 모양을 쓴다.
#[derive(Clone)]
pub(super) struct DraggedSheet {
    /// 그룹 안에서의 원래 위치.
    pub(super) index: usize,
    /// 함께 옮기는 시트 이름(목록 순서). 접합 묶음이면 여럿이다(ADR-0012).
    pub(super) labels: Vec<SharedString>,
    pub(super) pal: Palette,
}

impl Render for DraggedSheet {
    /// 시트 하나는 이름 한 줄. 접합 묶음은 한 덩어리로 옮겨 가는 것이 보이게 이름을 이어 붙이고
    /// 목록처럼 왼쪽 선으로 잇는다.
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let pal = self.pal;
        let card = div()
            .px_3()
            .py_2()
            .max_w(px(SHEET_COLUMN_WIDTH - 32.0))
            .rounded_md()
            .border_1()
            .border_color(pal.border)
            .bg(pal.background)
            .shadow_md()
            .text_sm()
            .text_color(pal.foreground);
        if self.labels.len() < 2 {
            return card
                .truncate()
                .font_semibold()
                .children(self.labels.first().cloned());
        }
        let rest = self.labels.len().saturating_sub(DRAG_PREVIEW_MAX_SHEETS);
        let lines = self
            .labels
            .iter()
            .take(DRAG_PREVIEW_MAX_SHEETS)
            .map(|label| {
                div()
                    .h(px(TITLE_LINE_HEIGHT))
                    .line_height(px(TITLE_LINE_HEIGHT))
                    .truncate()
                    .font_semibold()
                    .child(label.clone())
            });
        card.flex()
            .gap_2()
            .child(
                div()
                    .w(px(GLUE_BAR_WIDTH))
                    .flex_shrink_0()
                    .rounded_full()
                    .bg(pal.accent),
            )
            .child(
                div()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .children(lines)
                    .when(rest > 0, |this| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(pal.muted)
                                .child(format!("외 {rest}개 시트")),
                        )
                    }),
            )
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
        // 파일 그룹은 시트를 만들 수 없으니 작성 버튼 대신 파일 종류 필터를 둔다.
        .map(|this| {
            if sheet_group {
                this.child(tool_group(pal).child(icon_button(
                    "sheet-filter",
                    icons::filter(),
                    false,
                    pal,
                )))
                .child(tool_group(pal).child(
                    icon_button("new-sheet", icons::compose(), true, pal).on_click(
                        cx.listener(|this, _, window, cx| this.on_new_sheet(&NewSheet, window, cx)),
                    ),
                ))
            } else {
                this.child(tool_group(pal).child(self.file_filter_button(cx, pal)))
            }
        });

        let list: AnyElement = if let Some(mut files) = files {
            let (path, sort) = sort.clone().expect("파일 그룹은 그룹 선택에서만 나온다");
            self.sort_files(&mut files, sort);
            if let Some(kind) = self.file_filter {
                files.retain(|file| file.kind.filter_kind() == kind);
            }
            // 수동 정렬이고 거르지 않았을 때만 끌어서 순서를 바꾼다(위치가 그룹 순서와 같아야 한다).
            let manual = (sort.mode == library::SortMode::Manual && self.file_filter.is_none())
                .then_some(path);
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
            // 접합은 수동 순서에서만 뜻이 있다(앞 시트에 이어짐).
            let glued: Vec<bool> = sheets
                .iter()
                .enumerate()
                .map(|(index, sheet)| group_path.is_some() && index > 0 && sheet.glued)
                .collect();
            // 끄는 중이면 놓일 자리: (시트 위치, 위쪽인지). 접합 묶음은 묶음째 옮기므로(ADR-0012)
            // 대상 묶음의 앞(위로 옮길 때)이나 뒤(아래로 옮길 때) 가장자리에만 보인다.
            let drop_edge =
                self.sheet_drop
                    .filter(|_| cx.has_active_drag())
                    .and_then(|(from, over)| {
                        let (block, target) =
                            (glued_block(&glued, from), glued_block(&glued, over));
                        if block == target {
                            None
                        } else if target.start < block.start {
                            Some((target.start, true))
                        } else {
                            Some((target.end - 1, false))
                        }
                    });
            let labels: Vec<SharedString> = sheets
                .iter()
                .map(|sheet| {
                    sheet_label(self.previews.get(&sheet.path).map(|(_, preview)| preview)).into()
                })
                .collect();
            let paths: std::rc::Rc<Vec<std::path::PathBuf>> =
                std::rc::Rc::new(sheets.iter().map(|sheet| sheet.path.clone()).collect());
            let mut items: Vec<AnyElement> = Vec::with_capacity(sheets.len());
            for (index, sheet) in sheets.into_iter().enumerate() {
                let path = sheet.path.clone();
                let selected = self.selected_sheet.as_ref() == Some(&path)
                    || self.selected_sheets.contains(&path);
                let (glued_prev, glued_next) =
                    (glued[index], glued.get(index + 1).copied().unwrap_or(false));
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
                    // 접합한 시트끼리는 구분선 없이 붙인다.
                    .border_color(if selected || glued_next {
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
                let card = div()
                    .h_full()
                    .px_2()
                    .child(
                        div()
                            .h_full()
                            .px_2()
                            .rounded_md()
                            .when(selected, |this| this.bg(pal.active))
                            .child(content),
                    )
                    // 접합한 시트는 왼쪽 선으로 잇는다. 이어지는 쪽은 카드 끝까지 긋는다.
                    .when(glued_prev || glued_next, |this| {
                        this.child(
                            div()
                                .absolute()
                                .left(px(3.0))
                                .w(px(GLUE_BAR_WIDTH))
                                .top(px(if glued_prev { 0.0 } else { GLUE_BAR_INSET }))
                                .bottom(px(if glued_next { 0.0 } else { GLUE_BAR_INSET }))
                                .rounded_full()
                                .bg(pal.accent),
                        )
                    })
                    .context_menu({
                        let (app, path, paths) =
                            (cx.entity().downgrade(), path.clone(), paths.clone());
                        let group_path = group_path.clone();
                        move |menu, _, cx| {
                            sheet_menu(menu, &app, &path, &paths, group_path.as_deref(), cx)
                        }
                    });

                // 끌어다 놓을 자리는 카드를 감싼 줄의 위/아래 선으로 보여준다.
                let row = div()
                    .id(SharedString::from(format!("sheet-{}", path.display())))
                    .cursor_pointer()
                    .on_click(cx.listener({
                        let (path, paths) = (path.clone(), paths.clone());
                        move |this, event: &ClickEvent, window, cx| {
                            // Shift+클릭: 연 시트부터 누른 시트까지 넓혀 고른다(연 시트는 그대로).
                            if event.modifiers().shift
                                && let Some(range) = this
                                    .selected_sheet
                                    .as_ref()
                                    .and_then(|anchor| paths.iter().position(|p| p == anchor))
                                    .map(|anchor| anchor.min(index)..anchor.max(index) + 1)
                            {
                                this.selected_sheets = paths[range].to_vec();
                                cx.notify();
                                return;
                            }
                            this.select_sheet(Some(path.clone()), false, window, cx);
                        }
                    }))
                    .child(card);
                let row = match &group_path {
                    Some(group_path) => {
                        let group_path = group_path.clone();
                        // 접합 묶음은 묶음째 옮기므로 미리보기도 묶음 전체다.
                        let dragged = DraggedSheet {
                            index,
                            labels: labels[glued_block(&glued, index)].to_vec(),
                            pal,
                        };
                        row.on_drag(dragged, |dragged, _, _, cx| cx.new(|_| dragged.clone()))
                            .on_drag_move(cx.listener(
                                move |this, event: &DragMoveEvent<DraggedSheet>, _, cx| {
                                    if !event.bounds.contains(&event.event.position) {
                                        return;
                                    }
                                    let drop = Some((event.drag(cx).index, index));
                                    if this.sheet_drop != drop {
                                        this.sheet_drop = drop;
                                        cx.notify();
                                    }
                                },
                            ))
                            .map(|row| match drop_edge {
                                Some((edge, true)) if edge == index => {
                                    row.border_t_2().border_color(pal.accent)
                                }
                                Some((edge, false)) if edge == index => {
                                    row.border_b_2().border_color(pal.accent)
                                }
                                _ => row,
                            })
                            .on_drop(cx.listener(
                                move |this, dragged: &DraggedSheet, window, cx| {
                                    this.sheet_drop = None;
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
                // 목록 밖으로 끌고 나가면 놓일 자리 표시를 지운다.
                .on_drag_move(
                    cx.listener(|this, event: &DragMoveEvent<DraggedSheet>, _, cx| {
                        if !event.bounds.contains(&event.event.position)
                            && this.sheet_drop.take().is_some()
                        {
                            cx.notify();
                        }
                    }),
                )
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

/// `index` 시트가 든 접합 묶음(`glued[i]` = i번 시트가 앞 시트에 이어짐). `library::move_sheet`와 같은 규칙이다.
fn glued_block(glued: &[bool], index: usize) -> std::ops::Range<usize> {
    let mut start = index;
    while start > 0 && glued[start] {
        start -= 1;
    }
    let mut end = index + 1;
    while glued.get(end).copied().unwrap_or(false) {
        end += 1;
    }
    start..end
}

/// 시트 카드 우클릭 메뉴: 시트 접합, 접합 해제.
///
/// 누른 시트가 선택(Shift+클릭) 안에 있으면 선택한 시트 전체에, 아니면 누른 시트에 적용한다.
/// 접합은 수동 순서 그룹(`group_path`)에서만 할 수 있다.
fn sheet_menu(
    menu: PopupMenu,
    app: &WeakEntity<WriterApp>,
    path: &std::path::Path,
    paths: &[std::path::PathBuf],
    group_path: Option<&std::path::Path>,
    cx: &App,
) -> PopupMenu {
    let Some(clicked) = paths.iter().position(|p| p == path) else {
        return menu;
    };
    let (range, sheets) = match app.upgrade() {
        Some(app) => {
            let app = app.read(cx);
            let selected: Vec<usize> = app
                .selected_sheets
                .iter()
                .filter_map(|selected| paths.iter().position(|p| p == selected))
                .collect();
            let range = match (selected.iter().min(), selected.iter().max()) {
                (Some(&start), Some(&end)) if selected.contains(&clicked) => start..end + 1,
                _ => clicked..clicked + 1,
            };
            let sheets = group_path
                .and_then(|group| app.library.as_ref()?.find_group(group))
                .map(|group| group.sheets.clone())
                .unwrap_or_default();
            (range, sheets)
        }
        None => return menu,
    };
    let glued = |index: usize| index > 0 && sheets.get(index).is_some_and(|sheet| sheet.glued);
    // 접합: 둘 이상 골랐고 아직 다 이어지지 않았을 때. 해제: 범위 안이나 바로 뒤가 이어져 있을 때.
    let can_glue = range.len() > 1 && (range.start + 1..range.end).any(|index| !glued(index));
    let can_unglue = (range.start..range.end + 1).any(glued);
    let item = |icon, label, enabled: bool, glue: bool| {
        let (app, group, range) = (
            app.clone(),
            group_path.map(std::path::Path::to_path_buf),
            range.clone(),
        );
        PopupMenuItem::element(move |_, _| menu_row(icon, label, None))
            .disabled(!enabled || group.is_none())
            .on_click(move |_, window, cx| {
                let Some(group) = &group else {
                    return;
                };
                app.update(cx, |app, cx| {
                    app.set_glued(group, range.clone(), glue, window, cx)
                })
                .ok();
            })
    };
    menu.min_w(px(200.0))
        .item(item(icons::glue(), "시트 접합", can_glue, true))
        .item(item(icons::unglue(), "접합 해제", can_unglue, false))
}
