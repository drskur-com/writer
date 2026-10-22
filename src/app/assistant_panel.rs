//! 오른쪽 어시스턴트(LLM) 패널. 에디터 툴바 오른쪽 끝 버튼으로 열고 닫는다.
//!
//! 아직 모델에 연결하지 않았다. 화면 모양을 잡으려고 샘플 대화를 보여 준다.

use gpui_kit::component::*;
use gpui_kit::gpui::prelude::FluentBuilder;
use gpui_kit::gpui::*;

use super::WriterApp;
use super::style::*;
use crate::icons;

/// 말한 쪽.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    User,
    Agent,
}

struct Message {
    role: Role,
    text: &'static str,
}

/// 대화 기록에서 묶는 기간.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Period {
    Today,
    Yesterday,
    ThisWeek,
}

impl Period {
    fn label(self) -> &'static str {
        match self {
            Period::Today => "오늘",
            Period::Yesterday => "어제",
            Period::ThisWeek => "지난 7일",
        }
    }
}

struct Conversation {
    title: &'static str,
    /// 마지막 메시지를 주고받은 때. 기록 화면에서 기간으로 묶고, 줄 오른쪽에 시각을 적는다.
    period: Period,
    time: &'static str,
    messages: &'static [Message],
}

impl Conversation {
    /// 기록 화면에 보일 마지막 메시지 한 줄.
    fn preview(&self) -> &'static str {
        self.messages
            .last()
            .map(|message| {
                message
                    .text
                    .lines()
                    .find(|line| !line.is_empty())
                    .unwrap_or("")
            })
            .unwrap_or("")
    }
}

const fn user(text: &'static str) -> Message {
    Message {
        role: Role::User,
        text,
    }
}

const fn agent(text: &'static str) -> Message {
    Message {
        role: Role::Agent,
        text,
    }
}

/// 샘플 대화. 모델을 붙이면 대화 저장소로 바꾼다.
const SAMPLE_CONVERSATIONS: &[Conversation] = &[
    Conversation {
        title: "3장 첫 문단 다듬기",
        period: Period::Today,
        time: "오후 9:12",
        messages: &[
            user("3장 첫 문단이 너무 늘어져. 뜻은 그대로 두고 짧게 줄여 줄래?"),
            agent(
                "세 문장이 같은 내용(비 오는 저녁의 정류장)을 되풀이하고 있어요. \
                 하나로 묶으면 이렇게 됩니다.\n\n\
                 \"비가 그치지 않는 저녁, 정류장에는 우산 없는 사람들만 남았다.\"",
            ),
            user("좋아. 그런데 '남았다'보다 덜 쓸쓸한 말은 없을까?"),
            agent(
                "'모여 있었다'나 '서성였다'를 권해요. 다음 문단에서 인물들이 \
                 말을 주고받으니 '모여 있었다'가 흐름에 더 맞습니다.",
            ),
        ],
    },
    Conversation {
        title: "등장인물 이름 점검",
        period: Period::Today,
        time: "오후 2:40",
        messages: &[
            user("원고 전체에서 등장인물 이름이 섞여 쓰인 곳이 있는지 봐 줘."),
            agent(
                "두 곳을 찾았어요.\n\n\
                 • 2장 「귀갓길」: '윤서'와 '윤아'가 같은 인물로 쓰였어요.\n\
                 • 5장 「편지」: '박 선생'이 앞에서는 '박 교수'였어요.\n\n\
                 어느 쪽으로 맞출지 알려 주시면 고칠 문장을 보여 드릴게요.",
            ),
            user("'윤서', '박 선생'으로 맞추자."),
            agent(
                "알겠어요. 2장 3곳, 5장 1곳을 바꾸면 됩니다. 바꾸기 전에 문장마다 확인해 드릴게요.",
            ),
        ],
    },
    Conversation {
        title: "목차 구성 제안",
        period: Period::Yesterday,
        time: "어제",
        messages: &[
            user("지금 장 순서가 어색한 것 같아. 어떻게 생각해?"),
            agent(
                "4장 「겨울 바다」가 시간상 2장보다 앞선 일이에요. 회상으로 두려는 게 \
                 아니라면 2장 앞으로 옮기는 걸 권해요. 그러면 1부가 계절 순서(가을 → 겨울)로 \
                 이어집니다.",
            ),
        ],
    },
    Conversation {
        title: "맞춤법 한 번에 보기",
        period: Period::ThisWeek,
        time: "10월 19일",
        messages: &[
            user("1장에서 띄어쓰기 틀린 곳만 골라 줘."),
            agent("'할수 있다' → '할 수 있다' 등 7곳이에요. 문장마다 바꿀 모양을 붙여 드릴게요."),
        ],
    },
    Conversation {
        title: "책 소개 문구 초안",
        period: Period::ThisWeek,
        time: "10월 17일",
        messages: &[
            user("뒤표지에 넣을 소개 문구를 세 문장으로 써 줘."),
            agent(
                "\"비 오는 정류장에서 시작된 한 계절의 이야기.\" 처럼 장면으로 여는 안과,                  인물로 여는 안을 하나씩 준비했어요.",
            ),
        ],
    },
];

/// 패널 폭.
const ASSISTANT_PANEL_WIDTH: f32 = 360.0;

impl WriterApp {
    /// 어시스턴트 패널이 보이는지. 시트를 연 때만 열 수 있다.
    pub(super) fn assistant_open(&self) -> bool {
        self.assistant_visible && self.editor.is_some()
    }

    /// 어시스턴트 패널 토글 버튼. 닫혀 있으면 에디터 툴바 오른쪽 끝에, 열려 있으면 패널 툴바
    /// 오른쪽 끝에 둔다(좁은 창에서 에디터 툴바가 잘려도 닫을 수 있게).
    /// 시트를 열지 않았으면 흐리게 두고 누를 수 없다.
    pub(super) fn assistant_toggle(
        &self,
        cx: &mut Context<Self>,
        pal: Palette,
    ) -> impl IntoElement {
        let enabled = self.editor.is_some();
        tool_group(pal).child(
            icon_button("toggle-assistant", icons::assistant(), enabled, pal)
                .when(self.assistant_open(), |this| this.bg(pal.active))
                .when(enabled, |this| {
                    this.on_click(cx.listener(|this, _, _, cx| {
                        this.assistant_visible = !this.assistant_visible;
                        cx.notify();
                    }))
                }),
        )
    }

    /// 오른쪽 어시스턴트 패널. 항상 그리고, 바깥 슬롯 폭만 스프링으로 0 ↔ 패널 폭 사이에서 움직인다.
    pub(super) fn render_assistant_panel(
        &self,
        cx: &mut Context<Self>,
        pal: Palette,
    ) -> impl IntoElement {
        let current = self
            .assistant_conversation
            .min(SAMPLE_CONVERSATIONS.len() - 1);
        let conversation = &SAMPLE_CONVERSATIONS[current];

        let toolbar = window_drag_area(
            div()
                .h(px(TOOLBAR_HEIGHT))
                .flex_shrink_0()
                .pl_4()
                .pr_3()
                .flex()
                .items_center()
                .gap_2(),
        )
        // 기록 화면이면 대화로 돌아가는 버튼, 대화 화면이면 기록을 여는 버튼을 둔다.
        .child(
            tool_group(pal).child(
                icon_button(
                    "assistant-history",
                    if self.assistant_history {
                        icons::back()
                    } else {
                        icons::history()
                    },
                    true,
                    pal,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.assistant_history = !this.assistant_history;
                    cx.notify();
                })),
            ),
        )
        .child(div().flex_1().min_w_0().truncate().font_semibold().child(
            if self.assistant_history {
                "대화 기록"
            } else {
                conversation.title
            },
        ))
        .child(tool_group(pal).child(icon_button("assistant-new", icons::compose(), false, pal)))
        .child(self.assistant_toggle(cx, pal));

        let messages = div()
            .id("assistant-messages")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px_4()
            .py_3()
            .flex()
            .flex_col()
            .gap_3()
            .children(
                conversation
                    .messages
                    .iter()
                    .map(|message| message_bubble(message, pal)),
            );

        // 입력창 자리. 모델을 붙이기 전이라 입력은 받지 않는다.
        let composer = div().flex_shrink_0().p_3().child(
            div()
                .flex()
                .items_end()
                .gap_2()
                .px_3()
                .py_2()
                .rounded_lg()
                .border_1()
                .border_color(pal.panel_border)
                .bg(pal.panel)
                .child(
                    div()
                        .flex_1()
                        .min_h(px(24.0))
                        .text_sm()
                        .text_color(pal.muted)
                        .child("열린 시트에 대해 물어보세요…"),
                )
                .child(icon_button("assistant-send", icons::send(), false, pal)),
        );

        let panel = div()
            .w(px(ASSISTANT_PANEL_WIDTH))
            .h_full()
            .flex()
            .flex_col()
            .bg(pal.background)
            .border_l_1()
            .border_color(pal.border)
            .child(toolbar)
            .map(|this| {
                if self.assistant_history {
                    this.child(self.render_assistant_history(current, cx, pal))
                } else {
                    this.child(messages).child(composer)
                }
            });

        div()
            .h_full()
            .flex_shrink_0()
            .overflow_hidden()
            .child(panel)
            .with_spring(
                "assistant-panel",
                SpringAnimation::new(SIDEBAR_SPRING).to(if self.assistant_open() {
                    1.0_f32
                } else {
                    0.0
                }),
                |this, open| this.w(px(ASSISTANT_PANEL_WIDTH * open.clamp(0.0, 1.0))),
            )
    }
}

impl WriterApp {
    /// 대화 기록 화면. 기간(오늘·어제·지난 7일)으로 묶고, 줄마다 제목·시각·마지막 메시지를 보인다.
    /// 줄을 누르면 그 대화로 돌아간다.
    fn render_assistant_history(
        &self,
        current: usize,
        cx: &mut Context<Self>,
        pal: Palette,
    ) -> impl IntoElement {
        let mut rows: Vec<AnyElement> = Vec::new();
        let mut last_period = None;
        for (ix, item) in SAMPLE_CONVERSATIONS.iter().enumerate() {
            if last_period != Some(item.period) {
                last_period = Some(item.period);
                rows.push(
                    div()
                        .px_2()
                        .pt_3()
                        .pb_1()
                        .text_xs()
                        .font_semibold()
                        .text_color(pal.muted)
                        .child(item.period.label())
                        .into_any_element(),
                );
            }
            let selected = ix == current;
            rows.push(
                div()
                    .id(("assistant-conversation", ix))
                    .px_2()
                    .py_2()
                    .rounded_md()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .cursor_pointer()
                    .when(selected, |this| this.bg(pal.active))
                    .when(!selected, |this| {
                        this.hover(move |style| style.bg(pal.hover))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.assistant_conversation = ix;
                        this.assistant_history = false;
                        cx.notify();
                    }))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_sm()
                                    .font_medium()
                                    .child(item.title),
                            )
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .text_xs()
                                    .text_color(pal.muted)
                                    .child(item.time),
                            ),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_xs()
                            .text_color(pal.muted)
                            .child(item.preview()),
                    )
                    .into_any_element(),
            );
        }

        div()
            .id("assistant-history")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px_2()
            .pb_3()
            .flex()
            .flex_col()
            .gap(px(1.0))
            .children(rows)
    }
}

/// 대화 한 줄. 내 말은 오른쪽 말풍선, 어시스턴트 말은 왼쪽 본문으로 그린다.
fn message_bubble(message: &Message, pal: Palette) -> AnyElement {
    match message.role {
        Role::User => div()
            .flex()
            .justify_end()
            .child(
                div()
                    .max_w(px(ASSISTANT_PANEL_WIDTH * 0.8))
                    .px_3()
                    .py_2()
                    .rounded_lg()
                    .bg(pal.tint)
                    .text_sm()
                    .child(message.text),
            )
            .into_any_element(),
        Role::Agent => div()
            .flex()
            .gap_2()
            .child(
                div()
                    .flex_shrink_0()
                    .pt(px(2.0))
                    .child(Icon::new(icons::assistant()).small().text_color(pal.muted)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .line_height(relative(1.5))
                    .child(message.text),
            )
            .into_any_element(),
    }
}
