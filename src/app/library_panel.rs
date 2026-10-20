//! 사이드바 1단: 떠 있는 라이브러리 패널(섹션 + 그룹).

use gpui_kit::assets::IconName as AssetIcon;
use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenuItem};
use gpui_kit::component::*;
use gpui_kit::gpui::prelude::{
    FluentBuilder, InteractiveElement as _, StatefulInteractiveElement as _,
};
use gpui_kit::gpui::*;

use super::style::*;
use super::{Selection, WriterApp};
use crate::{icons, library};

impl WriterApp {
    /// 사이드바 1단: 창 안쪽에 떠 있는 라이브러리 패널(섹션 + 그룹).
    ///
    /// 패널은 항상 그리고, 바깥 슬롯 너비만 스프링으로 0 ↔ `PANEL_SLOT_WIDTH` 사이에서 움직인다.
    /// 안쪽은 고정 폭이라 접히는 동안 내용이 다시 배치되지 않고 잘려 나간다.
    pub(super) fn render_library_panel(
        &self,
        cx: &mut Context<Self>,
        pal: Palette,
    ) -> impl IntoElement {
        let current = self.effective_selection();
        let project = self
            .open_project
            .as_ref()
            .and_then(|dir| self.library.as_ref()?.find_project(dir))
            .cloned();

        let mut rows: Vec<AnyElement> = Vec::new();
        let mut header: Option<AnyElement> = None;
        if let Some(project) = &project {
            header = Some(self.project_header(project, cx, pal).into_any_element());
            self.project_rows(project, current.as_ref(), &mut rows, cx, pal);
        } else if let Some(lib) = &self.library {
            for section in &lib.sections {
                let kind = section.kind;
                let collapsed = self.collapsed_sections.contains(&kind);
                let header_group = SharedString::from(format!("section-{}", kind.dir()));

                rows.push(
                    div()
                        .group(header_group.clone())
                        .mt_3()
                        .mb_1()
                        .px_4()
                        .flex()
                        .items_center()
                        .justify_between()
                        .text_xs()
                        .font_semibold()
                        .text_color(pal.muted)
                        .child(kind.title())
                        .child(
                            div()
                                .id(SharedString::from(format!("section-toggle-{}", kind.dir())))
                                .cursor_pointer()
                                .font_normal()
                                // 펼친 섹션은 헤더에 마우스를 올렸을 때만 "가리기"를 보인다.
                                .when(!collapsed, |this| {
                                    this.invisible()
                                        .group_hover(header_group, |style| style.visible())
                                })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.toggle_section(kind, cx);
                                }))
                                .child(if collapsed { "보기" } else { "가리기" }),
                        )
                        .into_any_element(),
                );

                if collapsed {
                    continue;
                }

                // 메모: 그룹을 가로지르는 "모두" 보기.
                if kind == library::SectionKind::Notes {
                    let selection = Selection::Section(kind);
                    let selected = current.as_ref() == Some(&selection);
                    rows.push(
                        self.library_row(
                            cx,
                            pal,
                            "all-notes".into(),
                            icons::all_sheets(),
                            "모두".into(),
                            None,
                            selected,
                            selection,
                            0,
                            Disclosure::None,
                        )
                        .into_any_element(),
                    );
                }

                let mut has_group = false;
                for entry in &section.entries {
                    let library::Entry::Group(group) = entry else {
                        continue;
                    };
                    has_group = true;
                    let selection = Selection::Group(group.path.clone());
                    let selected = current.as_ref() == Some(&selection);
                    let row = self.library_row(
                        cx,
                        pal,
                        format!("group-{}", group.path.display()).into(),
                        icons::group_icon(group.icon.as_deref()),
                        group.name.clone().into(),
                        group
                            .color
                            .as_deref()
                            .and_then(|color| project_color(color, cx.theme())),
                        selected,
                        selection,
                        0,
                        Disclosure::None,
                    );
                    // 프로젝트는 우클릭으로 편집·삭제한다.
                    if kind == library::SectionKind::Projects {
                        let app = cx.entity().downgrade();
                        let dir = group.path.clone();
                        rows.push(
                            row.context_menu(move |menu, _, cx| {
                                let (edit_app, edit_dir) = (app.clone(), dir.clone());
                                let (delete_app, delete_dir) = (app.clone(), dir.clone());
                                let danger = cx.theme().danger;
                                menu.min_w(px(200.0))
                                    .item(
                                        PopupMenuItem::element(move |_, _| {
                                            menu_row(icons::edit(), "편집…", None)
                                        })
                                        .on_click(
                                            move |_, window, cx| {
                                                edit_app
                                                    .update(cx, |app, cx| {
                                                        app.open_edit_project(&edit_dir, window, cx)
                                                    })
                                                    .ok();
                                            },
                                        ),
                                    )
                                    .separator()
                                    // 되돌리기 어려운 동작이라 빨간색으로 구분한다.
                                    .item(
                                        PopupMenuItem::element(move |_, _| {
                                            menu_row(
                                                icons::trash(),
                                                "휴지통으로 이동",
                                                Some(danger),
                                            )
                                        })
                                        .on_click(
                                            move |_, window, cx| {
                                                delete_app
                                                    .update(cx, |app, cx| {
                                                        app.confirm_delete_project(
                                                            &delete_dir,
                                                            window,
                                                            cx,
                                                        )
                                                    })
                                                    .ok();
                                            },
                                        ),
                                    )
                            })
                            .into_any_element(),
                        );
                    } else {
                        rows.push(row.into_any_element());
                    }
                }

                if kind == library::SectionKind::Projects && !has_group {
                    rows.push(
                        div()
                            .id("new-project")
                            .mx_3()
                            .mt_1()
                            .h(px(26.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_md()
                            .bg(pal.tint)
                            .text_xs()
                            .text_color(pal.tint_foreground)
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_new_project(window, cx);
                            }))
                            .child("새로운 프로젝트…")
                            .into_any_element(),
                    );
                }
            }
        }

        let panel = div()
            .w(px(PANEL_WIDTH))
            .h_full()
            .flex()
            .flex_col()
            .rounded_xl()
            .bg(pal.panel)
            .border_1()
            .border_color(pal.panel_border)
            .shadow_md()
            .child(
                // 위쪽 줄: (macOS) 신호등 버튼 자리 + 오른쪽 도구 버튼.
                // 프로젝트 화면이면 왼쪽에 라이브러리로 돌아가는 버튼을 둔다.
                window_drag_area(
                    div()
                        .h(px(TOOLBAR_HEIGHT - PANEL_INSET * 2.0))
                        .flex_shrink_0()
                        .px_2()
                        .flex()
                        .items_center()
                        .gap_1(),
                )
                .when(project.is_some(), |this| {
                    #[cfg(target_os = "macos")]
                    let this = this.pl(px(TRAFFIC_LIGHT_SPACE - PANEL_INSET));
                    this.child(
                        icon_button("back-to-library", icons::back_to_library(), true, pal)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.leave_project(window, cx);
                            })),
                    )
                })
                .child(div().flex_1())
                .child(
                    icon_button("new-group", icons::new_group(), self.library.is_some(), pal)
                        .on_click(cx.listener(
                            |this, _, window, cx| match this.open_project.clone() {
                                Some(dir) => this.open_new_group(dir, true, window, cx),
                                None => this.open_new_project(window, cx),
                            },
                        )),
                )
                .child(
                    icon_button("hide-sidebar", icons::panel(), true, pal).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.sidebar_visible = false;
                            cx.notify();
                        },
                    )),
                ),
            )
            .children(header)
            .child(
                div()
                    .id("library-rows")
                    .flex_1()
                    .overflow_y_scroll()
                    .pb_2()
                    .children(rows),
            );

        div()
            .h_full()
            .flex_shrink_0()
            .overflow_hidden()
            .child(
                div()
                    .w(px(PANEL_SLOT_WIDTH))
                    .h_full()
                    .pl(px(PANEL_INSET))
                    .pr(px(PANEL_GAP))
                    .py(px(PANEL_INSET))
                    .child(panel),
            )
            .with_spring(
                "library-panel",
                SpringAnimation::new(SIDEBAR_SPRING).to(if self.sidebar_visible {
                    1.0_f32
                } else {
                    0.0
                }),
                |this, open| this.w(px(PANEL_SLOT_WIDTH * open.clamp(0.0, 1.0))),
            )
    }

    /// 라이브러리 패널의 한 줄(“모두”, 그룹). `depth`만큼 들여 쓴다.
    ///
    /// 프로젝트를 누르면 사이드바를 그 프로젝트 화면으로 바꾼다.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn library_row(
        &self,
        cx: &mut Context<Self>,
        pal: Palette,
        id: SharedString,
        icon: AssetIcon,
        label: SharedString,
        tint: Option<Hsla>,
        selected: bool,
        selection: Selection,
        depth: usize,
        disclosure: Disclosure,
    ) -> Stateful<Div> {
        let (text_color, icon_color) = if selected {
            (pal.accent_foreground, pal.accent_foreground)
        } else {
            (pal.foreground, tint.unwrap_or(pal.foreground.opacity(0.7)))
        };

        let gutter = match disclosure {
            Disclosure::None => None,
            Disclosure::Leaf => Some(
                div()
                    .w(px(DISCLOSURE_WIDTH))
                    .mr(px(-DISCLOSURE_OVERLAP))
                    .flex_shrink_0()
                    .into_any_element(),
            ),
            Disclosure::Branch { expanded } => {
                let Selection::Group(dir) = selection.clone() else {
                    unreachable!("펼침 표시는 그룹에만 붙는다");
                };
                Some(
                    div()
                        .id(SharedString::from(format!("disclosure-{}", dir.display())))
                        .w(px(DISCLOSURE_WIDTH))
                        .mr(px(-DISCLOSURE_OVERLAP))
                        .flex_shrink_0()
                        .flex()
                        .justify_center()
                        // 펼치기만 하고 줄 선택으로 번지지 않게 막는다.
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.toggle_group(&dir, cx);
                        }))
                        .child(
                            Icon::new(icons::disclosure(expanded))
                                .xsmall()
                                .text_color(pal.muted),
                        )
                        .into_any_element(),
                )
            }
        };

        div()
            .id(id)
            .mx_2()
            .px_2()
            // 트리 줄은 펼침 표시가 왼쪽 여백 자리를 대신한다. 하위 단계마다 한 칸씩 들인다.
            .when(!matches!(disclosure, Disclosure::None), |this| {
                this.pl(px(DISCLOSURE_WIDTH * depth as f32))
            })
            .h(px(30.0))
            .flex()
            .items_center()
            .gap_2()
            .rounded_lg()
            .text_sm()
            .text_color(text_color)
            .when(selected, |this| this.bg(pal.accent).font_semibold())
            .when(!selected, |this| {
                this.hover(move |style| style.bg(pal.hover))
            })
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, window, cx| {
                if let Selection::Group(dir) = &selection
                    && this.open_project.is_none()
                    && this
                        .library
                        .as_ref()
                        .is_some_and(|lib| lib.find_project(dir).is_some())
                {
                    this.enter_project(dir, window, cx);
                    return;
                }
                this.selected = Some(selection.clone());
                this.select_sheet(None, false, window, cx);
                this.load_previews(cx);
            }))
            .children(gutter)
            .child(Icon::new(icon).small().text_color(icon_color))
            .child(label)
    }
}

/// 하위 그룹 펼침 표시(›) 칸 폭. 들여쓰기 한 단계도 이 폭이다.
const DISCLOSURE_WIDTH: f32 = 12.0;

/// 펼침 표시 칸을 아이콘 쪽으로 겹치는 폭. 표시와 아이콘이 딱 붙어 보이게 한다.
const DISCLOSURE_OVERLAP: f32 = 6.0;

/// 줄 앞의 펼침 표시.
#[derive(Clone, Copy)]
pub(super) enum Disclosure {
    /// 칸을 두지 않는다(라이브러리 화면).
    None,
    /// 하위 그룹이 없어 빈 칸만 둔다(프로젝트 화면에서 아이콘을 나란히 세운다).
    Leaf,
    /// 하위 그룹이 있다. 누르면 펼치거나 접는다.
    Branch { expanded: bool },
}

/// 우클릭 메뉴 한 줄: 아이콘 + 글자. 모든 줄이 같은 모양이어야 글자가 나란히 선다.
pub(super) fn menu_row(icon: AssetIcon, label: &'static str, color: Option<Hsla>) -> Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .when_some(color, |this, color| this.text_color(color))
        .child(Icon::new(icon).small())
        .child(label)
}
