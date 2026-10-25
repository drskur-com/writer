//! 에디터의 할 일 체크 상자(ADR-0023): GFM 할 일 항목(`- [ ] 일정`)의 기호 자리에 ☐·☑를 그리고,
//! 누르면 원문의 `[ ]`↔`[x]`를 바꾸고 완료일(`✅ 날짜`, ADR-0024)을 붙이거나 뗀다.
//!
//! 체크 상자는 줄 오버레이(gpui-base 패치)로 기호 위에 덮어 그린다. 원문은 그대로 남고, 커서가
//! 그 줄에 들어오면 오버레이가 빠져 `- [ ] ` 글자를 고칠 수 있다.

use gpui_base::input::LineOverlayContext;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::input::EditorState;
use gpui_kit::component::*;
use gpui_kit::gpui::prelude::FluentBuilder as _;
use gpui_kit::gpui::*;

use super::style::Palette;
use crate::{icons, library, markdown};

/// 체크 상자 한 변의 길이(줄 높이에 대한 비율과 한도).
const BOX_SCALE: f32 = 0.55;
const BOX_MIN: f32 = 12.;
const BOX_MAX: f32 = 18.;
/// 체크 상자와 항목 글 사이 간격(상자 한 변에 대한 비율).
const GAP_SCALE: f32 = 0.4;

/// 할 일 줄의 첫 행 여백(미완료, 완료). 비례 서체에서는 `[ ]`과 `[x]`의 폭이 달라 체크하면 뒤 글이 밀린다.
/// 좁은 쪽에 폭 차이만큼 여백을 주어 체크 여부와 상관없이 글이 같은 자리에 오게 한다.
/// 줄 스타일이라 커서가 그 줄에 들어가 원문이 보여도 그대로다.
pub(super) fn check_indents(family: &SharedString, font_size: f32, cx: &App) -> (Pixels, Pixels) {
    let text_system = WindowTextSystem::new(cx.text_system().clone());
    let width = |text: &str| {
        text_system
            .layout_line(
                text,
                px(font_size),
                &[TextRun {
                    len: text.len(),
                    font: font(family.clone()),
                    color: gpui_kit::gpui::black(),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                }],
                None,
            )
            .width
    };
    let delta = width("[x]") - width("[ ]");
    (delta.max(px(0.)), (-delta).max(px(0.)))
}

/// 에디터에 할 일 체크 상자를 단다.
pub(super) fn install(state: &mut EditorState, cx: &mut Context<EditorState>) {
    let editor = cx.entity().downgrade();
    state.set_line_overlays(
        |_, line| {
            markdown::task_marker(line)
                .map(|marker| vec![marker.cover])
                .unwrap_or_default()
        },
        move |overlay, _, cx| render(overlay, editor.clone(), cx),
        cx,
    );
}

fn render(overlay: &LineOverlayContext, editor: WeakEntity<EditorState>, cx: &App) -> AnyElement {
    let Some(marker) = markdown::task_marker(&overlay.text) else {
        return Empty.into_any_element();
    };
    let pal = Palette::from_theme(cx.theme());
    let side = px((f32::from(overlay.size.height) * BOX_SCALE).clamp(BOX_MIN, BOX_MAX));
    let (line, check, checked) = (overlay.line, marker.check, marker.checked);
    // 기호 글자를 가리도록 배경을 칠한다. 상자는 덮은 범위의 오른쪽 끝(항목 글 바로 앞)에 둔다.
    // 누르면 커서를 옮기지 않고 체크만 바꾼다.
    div()
        .size_full()
        .bg(pal.background)
        .flex()
        .items_center()
        .justify_end()
        .pr(side * GAP_SCALE)
        .cursor_pointer()
        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
            window.prevent_default();
            cx.stop_propagation();
            editor
                .update(cx, |state, cx| toggle(state, line, check, window, cx))
                .ok();
        })
        .child(check_box(side, checked, pal))
        .into_any_element()
}

/// 할 일 체크 상자 모양. 에디터와 할 일 목록이 같이 쓴다.
pub(super) fn check_box(side: Pixels, checked: bool, pal: Palette) -> Div {
    div()
        .size(side)
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(3.))
        .border_1()
        .map(|this| {
            if checked {
                this.bg(pal.tint).border_color(pal.tint).child(
                    Icon::new(icons::task_done())
                        .size(side * 0.8)
                        .text_color(pal.tint_foreground),
                )
            } else {
                this.border_color(pal.muted)
            }
        })
}

/// `line`번 줄의 할 일 체크를 뒤집는다. 그 사이 줄이 바뀌어 같은 자리에 체크 상자가 없으면 그대로 둔다.
fn toggle(
    state: &mut EditorState,
    line: usize,
    check: usize,
    window: &mut Window,
    cx: &mut Context<EditorState>,
) {
    let text = state.value();
    let Some((start, line_text)) = text
        .split('\n')
        .scan(0, |offset, line_text| {
            let start = *offset;
            *offset += line_text.len() + 1;
            Some((start, line_text))
        })
        .nth(line)
    else {
        return;
    };
    let Some(marker) = markdown::task_marker(line_text).filter(|marker| marker.check == check)
    else {
        return;
    };
    let Some(toggled) = markdown::toggle_todo(line_text, &library::today()) else {
        return;
    };
    // 체크 상자 글자부터 줄 끝까지 한 번에 바꾼다(완료일 `✅`도 함께, 되돌리기 한 번).
    let offset = start + marker.check;
    state.replace_range_keeping_selections(
        offset..start + line_text.len(),
        &toggled[marker.check..],
        window,
        cx,
    );
}
