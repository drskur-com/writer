//! 사이드바 1단: 떠 있는 라이브러리 패널(섹션 + 그룹).

use gpui_kit::assets::IconName as AssetIcon;
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

        let mut rows: Vec<AnyElement> = Vec::new();
        if let Some(lib) = &self.library {
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
                            selected,
                            selection,
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
                    rows.push(
                        self.library_row(
                            cx,
                            pal,
                            format!("group-{}", group.path.display()).into(),
                            icons::group_icon(group.icon.as_deref()),
                            group.name.clone().into(),
                            selected,
                            selection,
                        )
                        .into_any_element(),
                    );
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
                                this.notify("새 프로젝트 (준비 중)", window, cx);
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
                window_drag_area(
                    div()
                        .h(px(TOOLBAR_HEIGHT - PANEL_INSET * 2.0))
                        .flex_shrink_0()
                        .px_2()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap_1(),
                )
                .child(icon_button("new-group", icons::new_group(), false, pal))
                .child(
                    icon_button("hide-sidebar", icons::panel(), true, pal).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.sidebar_visible = false;
                            cx.notify();
                        },
                    )),
                ),
            )
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

    /// 라이브러리 패널의 한 줄(“모두”, 그룹).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn library_row(
        &self,
        cx: &mut Context<Self>,
        pal: Palette,
        id: SharedString,
        icon: AssetIcon,
        label: SharedString,
        selected: bool,
        selection: Selection,
    ) -> Stateful<Div> {
        let (text_color, icon_color) = if selected {
            (pal.accent_foreground, pal.accent_foreground)
        } else {
            (pal.foreground, pal.foreground.opacity(0.7))
        };

        div()
            .id(id)
            .mx_2()
            .px_2()
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
                this.selected = Some(selection.clone());
                this.select_sheet(None, false, window, cx);
                this.load_previews(cx);
            }))
            .child(Icon::new(icon).small().text_color(icon_color))
            .child(label)
    }
}
