use gpui_kit::gpui::*;
use gpui_kit::component::{button::*, *};

struct WriterApp {
    status_text: String,
}

impl Render for WriterApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0x18181b)) // zinc-900 dark background
            .text_color(rgb(0xf4f4f5))
            // 1. 상단 타이틀바 / 툴바
            .child(
                div()
                    .h(px(40.0))
                    .px_4()
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(rgb(0x27272a))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().font_bold().child("Writer"))
                            .child(div().text_xs().text_color(rgb(0xa1a1aa)).child("v0.1.0")),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Button::new("new-file")
                                    .label("New")
                                    .on_click(cx.listener(|this, _, _, _| {
                                        this.status_text = "새 문서를 준비했습니다.".to_string();
                                    })),
                            )
                            .child(
                                Button::new("save-file")
                                    .primary()
                                    .label("Save")
                                    .on_click(cx.listener(|this, _, _, _| {
                                        this.status_text = "문서가 저장되었습니다.".to_string();
                                    })),
                            ),
                    ),
            )
            // 2. 메인 바디 (사이드바 + 에디터 캔버스)
            .child(
                div()
                    .flex_1()
                    .flex()
                    .size_full()
                    // 사이드바
                    .child(
                        div()
                            .w(px(220.0))
                            .h_full()
                            .p_3()
                            .border_r_1()
                            .border_color(rgb(0x27272a))
                            .bg(rgb(0x141416))
                            .child(
                                div()
                                    .text_xs()
                                    .font_semibold()
                                    .text_color(rgb(0x71717a))
                                    .mb_2()
                                    .child("EXPLORER"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(
                                        div()
                                            .px_2()
                                            .py_1()
                                            .rounded_md()
                                            .bg(rgb(0x27272a))
                                            .text_sm()
                                            .child("📄 Untitled-1.md"),
                                    ),
                            ),
                    )
                    // 에디터 중앙 영역
                    .child(
                        div()
                            .flex_1()
                            .h_full()
                            .p_6()
                            .flex()
                            .flex_col()
                            .bg(rgb(0x1e1e24))
                            .child(
                                div()
                                    .text_xl()
                                    .font_bold()
                                    .mb_4()
                                    .child("GPUI Editor Ready"),
                            )
                            .child(
                                div()
                                    .text_color(rgb(0xa1a1aa))
                                    .line_height(relative(1.6))
                                    .child("gpui-kit 기반 에디터 앱 껍데기가 성공적으로 구동되었습니다.\n여기에 에디터 버퍼와 컴포넌트를 연결할 수 있습니다."),
                            ),
                    ),
            )
            // 3. 하단 상태 바
            .child(
                div()
                    .h(px(26.0))
                    .px_3()
                    .flex()
                    .items_center()
                    .justify_between()
                    .bg(rgb(0x09090b))
                    .text_xs()
                    .text_color(rgb(0x71717a))
                    .child(div().child(self.status_text.clone()))
                    .child(div().child("UTF-8 | Markdown | GPUI")),
            )
    }
}

fn main() {
    gpui_kit::application().run(|cx| {
        gpui_kit::init(cx);

        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(1024.0), px(680.0)),
                cx,
            ))),
            titlebar: Some(TitlebarOptions {
                title: Some("Writer".into()),
                appears_transparent: false,
                traffic_light_position: None,
            }),
            ..Default::default()
        };

        gpui_kit::open_window(options, cx, |_, cx| {
            cx.new(|_| WriterApp {
                status_text: "준비 완료".to_string(),
            })
        })
        .expect("창 열기에 실패했습니다");
    });
}
