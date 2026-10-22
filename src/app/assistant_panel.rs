//! 오른쪽 어시스턴트(LLM) 패널. 에디터 툴바 오른쪽 끝 버튼으로 열고 닫는다.
//!
//! 새 대화는 설정의 Bedrock 프로필로 실제 모델과 주고받고, 답을 받는 대로 조금씩 그린다
//! ([`crate::ai`]). 기록의 지난 대화는 아직 저장하지 않아 샘플을 보여 준다.

use futures::StreamExt as _;
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::component::searchable_list::{SearchableListItem, SearchableVec};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::*;
use gpui_kit::gpui::prelude::FluentBuilder;
use gpui_kit::gpui::*;

use super::WriterApp;
use super::style::*;
use crate::{ai, icons};

/// 새 대화(실제 모델과 주고받는 대화)의 상태.
pub(super) struct LiveChat {
    /// 입력창. Enter로 보내고 Shift+Enter로 줄을 바꾼다.
    input: Entity<TextareaState>,
    /// 주고받은 말. 답을 받는 중이면 마지막은 자라는 어시스턴트 답이다.
    turns: Vec<ai::Turn>,
    /// 마지막 요청이 실패한 까닭.
    error: Option<SharedString>,
    /// 답을 받는 태스크. 버리면 받기를 멈춘다.
    reply: Option<Task<()>>,
    /// 답을 받는 중인지. 태스크가 끝나도 `reply`는 남아 있으므로 따로 둔다.
    streaming: bool,
    /// 답이 자랄 때 맨 아래로 따라가는 스크롤.
    scroll: ScrollHandle,
    /// 입력창 아래의 모델 고르기. 리전에서 쓸 수 있는 모델만 담는다.
    model_picker: Entity<SelectState<SearchableVec<ModelItem>>>,
    models: ModelLookup,
    /// 모델 목록이 어느 (프로필, 리전) 것인지. 설정이 바뀌면 다시 확인한다.
    models_for: Option<(String, Option<String>)>,
    /// 모델 목록을 확인하는 태스크. 새로 시작하면 이전 것은 버린다.
    model_lookup: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

/// 리전에서 쓸 수 있는 모델 확인 상태.
enum ModelLookup {
    /// AI를 설정하지 않았다.
    Idle,
    Loading,
    /// 쓸 수 있는 모델과 부를 ID(추론 프로필).
    Ready(Vec<(ai::BedrockModel, String)>),
    Failed(SharedString),
}

/// 모델 고르기의 선택지 하나. 값은 부를 ID다.
#[derive(Clone)]
struct ModelItem {
    title: SharedString,
    id: SharedString,
}

impl SearchableListItem for ModelItem {
    type Value = SharedString;

    fn title(&self) -> SharedString {
        self.title.clone()
    }

    fn render(&self, _: &mut Window, _: &mut App) -> impl IntoElement {
        div().child(self.title.clone())
    }

    fn value(&self) -> &SharedString {
        &self.id
    }
}

impl LiveChat {
    pub(super) fn new(window: &mut Window, cx: &mut Context<WriterApp>) -> Self {
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 6)
                .submit_on_enter(true)
                .placeholder("열린 시트에 대해 물어보세요…")
        });
        let subscription =
            cx.subscribe_in(
                &input,
                window,
                |this, input, event, window, cx| match event {
                    InputEvent::PressEnter { shift: false, .. } => {
                        let text = input.read(cx).value().to_string();
                        this.send_assistant_message(text, window, cx);
                    }
                    // 보내기 버튼을 켜고 끄려고 다시 그린다.
                    InputEvent::Change => cx.notify(),
                    _ => {}
                },
            );
        let model_picker = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(Vec::<ModelItem>::new()),
                None,
                window,
                cx,
            )
        });
        let model_subscription = cx.subscribe(
            &model_picker,
            |_, _, event: &SelectEvent<SearchableVec<ModelItem>>, cx| {
                let SelectEvent::Confirm(value) = event;
                if let Some(id) = value {
                    crate::settings::Prefs::set_ai_model(Some(id.clone()), cx);
                }
            },
        );
        Self {
            input,
            turns: Vec::new(),
            error: None,
            reply: None,
            streaming: false,
            scroll: ScrollHandle::new(),
            model_picker,
            models: ModelLookup::Idle,
            models_for: None,
            model_lookup: None,
            _subscriptions: vec![subscription, model_subscription],
        }
    }
}

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

/// 빈 대화에서 보여 줄 질문 예시.
const SUGGESTIONS: &[&str] = &[
    "이 시트를 세 문장으로 요약해 줘",
    "어색한 문장을 찾아 고쳐 줘",
    "맞춤법과 띄어쓰기를 봐 줘",
    "다음 장면으로 이어질 아이디어를 줘",
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
            .filter(|&ix| ix < SAMPLE_CONVERSATIONS.len());
        let conversation = current.map(|ix| &SAMPLE_CONVERSATIONS[ix]);
        let configured = ai_configured(cx);
        let live = &self.assistant_chat;

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
                SharedString::from("대화 기록")
            } else {
                conversation.map_or_else(
                    || {
                        live.turns
                            .first()
                            .map(|turn| SharedString::from(turn.text.clone()))
                            .unwrap_or_else(|| "새 대화".into())
                    },
                    |conversation| conversation.title.into(),
                )
            },
        ))
        .child(tool_group(pal).child(
            icon_button("assistant-new", icons::compose(), true, pal).on_click(cx.listener(
                |this, _, _, cx| {
                    this.assistant_conversation = None;
                    this.assistant_history = false;
                    this.reset_live_chat(cx);
                },
            )),
        ))
        .child(self.assistant_toggle(cx, pal));

        let messages: AnyElement = match conversation {
            Some(conversation) => div()
                .id("assistant-messages")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .px_4()
                .py_3()
                .flex()
                .flex_col()
                .gap_3()
                .children(conversation.messages.iter().map(|message| {
                    message_bubble(message.role == Role::User, message.text.into(), pal)
                }))
                .into_any_element(),
            None if live.turns.is_empty() && live.error.is_none() => self
                .render_assistant_empty(configured, cx, pal)
                .into_any_element(),
            None => {
                let last = live.turns.len().saturating_sub(1);
                div()
                    .id("assistant-live-messages")
                    .track_scroll(&live.scroll)
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px_4()
                    .py_3()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .children(live.turns.iter().enumerate().map(|(ix, turn)| {
                        // 첫 조각이 오기 전의 빈 답은 받는 중 표시로 그린다.
                        if !turn.user && turn.text.is_empty() && live.streaming && ix == last {
                            message_bubble(false, "…".into(), pal)
                        } else {
                            message_bubble(turn.user, turn.text.clone().into(), pal)
                        }
                    }))
                    .children(live.error.clone().map(|error| {
                        div()
                            .px_3()
                            .py_2()
                            .rounded_lg()
                            .border_1()
                            .border_color(pal.danger.opacity(0.4))
                            .text_sm()
                            .text_color(pal.danger)
                            .child(error)
                    }))
                    .into_any_element()
            }
        };

        // 입력창. 새 대화에서만 쓰고(샘플 대화는 읽기 전용), AI를 설정하지 않았으면 두지 않는다.
        // 답을 받는 중에는 보내기 대신 멈추기 버튼을 둔다.
        let can_send = !live.streaming
            && self.assistant_model_usable(cx)
            && !live.input.read(cx).value().trim().is_empty();
        let send_button = if live.streaming {
            icon_button("assistant-stop", icons::stop(), true, pal)
                .on_click(cx.listener(|this, _, _, cx| this.stop_assistant_reply(cx)))
        } else {
            icon_button("assistant-send", icons::send(), can_send, pal).when(can_send, |this| {
                this.on_click(cx.listener(|this, _, window, cx| {
                    let text = this.assistant_chat.input.read(cx).value().to_string();
                    this.send_assistant_message(text, window, cx);
                }))
            })
        };
        let composer =
            div()
                .flex_shrink_0()
                .p_3()
                .when(configured && conversation.is_none(), |this| {
                    this.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .pl_1()
                            .pr_2()
                            .py_1()
                            .rounded_lg()
                            .border_1()
                            .border_color(pal.panel_border)
                            .bg(pal.panel)
                            .child(
                                div()
                                    .text_sm()
                                    .child(Textarea::new(&live.input).appearance(false)),
                            )
                            // 아래 줄: 모델 고르기와 보내기(멈추기) 버튼.
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .child(self.render_model_picker(pal)),
                                    )
                                    .child(send_button),
                            ),
                    )
                });

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
        current: Option<usize>,
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
            let selected = Some(ix) == current;
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
                        this.assistant_conversation = Some(ix);
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

impl WriterApp {
    /// 빈 대화. AI를 설정했으면 열린 시트를 두고 물을 수 있다는 안내와 질문 예시를, 아니면
    /// 설정 창을 여는 버튼을 보인다.
    fn render_assistant_empty(
        &self,
        configured: bool,
        cx: &mut Context<Self>,
        pal: Palette,
    ) -> impl IntoElement {
        let sheet_title = self
            .editor
            .as_ref()
            .and_then(|editor| self.previews.get(editor.path()))
            .map(|(_, preview)| preview.title.trim().to_string())
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| "제목 없는 시트".to_string());

        let badge = div()
            .size(px(48.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .bg(pal.tint)
            .child(
                Icon::new(icons::assistant())
                    .large()
                    .text_color(pal.tint_foreground),
            );

        let body =
            if configured {
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .text_lg()
                            .font_semibold()
                            .child("무엇을 도와드릴까요?"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(pal.muted)
                            .text_center()
                            .child(format!("「{sheet_title}」을 두고 물어볼 수 있어요.")),
                    )
                    .child(div().w_full().mt_5().flex().flex_col().gap_2().children(
                        SUGGESTIONS.iter().enumerate().map(|(ix, text)| {
                            div()
                                .id(("assistant-suggestion", ix))
                                .px_3()
                                .py_2()
                                .rounded_lg()
                                .border_1()
                                .border_color(pal.panel_border)
                                .text_sm()
                                .cursor_pointer()
                                .hover(move |style| style.bg(pal.hover))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.send_assistant_message(text.to_string(), window, cx);
                                }))
                                .child(*text)
                        }),
                    ))
            } else {
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .text_lg()
                            .font_semibold()
                            .child("AI가 설정되지 않았어요"),
                    )
                    .child(div().text_sm().text_color(pal.muted).text_center().child(
                        "설정 › AI에서 제공자와 AWS 프로필을 고르면 대화를 시작할 수 있어요.",
                    ))
                    .child(
                        div()
                            .id("assistant-open-settings")
                            .mt_4()
                            .px_4()
                            .py_2()
                            .rounded_full()
                            .bg(pal.tint)
                            .text_color(pal.tint_foreground)
                            .text_sm()
                            .font_medium()
                            .cursor_pointer()
                            .on_click(|_, _, cx| crate::settings::open(cx))
                            .child("AI 설정 열기"),
                    )
            };

        div()
            .id("assistant-empty")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px_6()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_4()
            .child(badge)
            .child(body)
    }
}

impl WriterApp {
    /// 새 대화에 사용자 말을 더하고 Bedrock에 보낸다. 답은 받는 대로 마지막 어시스턴트 답에 붙인다.
    /// 답을 받는 중이거나 빈 말이면 아무것도 하지 않는다.
    pub(super) fn send_assistant_message(
        &mut self,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = text.trim().to_string();
        if text.is_empty() || self.assistant_chat.streaming {
            return;
        }
        let Some(request) = self.assistant_request(&text, cx) else {
            return;
        };

        // 샘플 대화를 보고 있었어도 새 대화로 돌아와 보낸다.
        self.assistant_conversation = None;
        self.assistant_history = false;
        let chat = &mut self.assistant_chat;
        chat.error = None;
        chat.turns.push(ai::Turn {
            user: true,
            text: text.clone(),
        });
        chat.turns.push(ai::Turn {
            user: false,
            text: String::new(),
        });
        chat.streaming = true;
        chat.input
            .update(cx, |input, cx| input.set_value("", window, cx));
        chat.scroll.scroll_to_bottom();

        let mut events = ai::stream(request);
        chat.reply = Some(cx.spawn_in(window, async move |this, cx| {
            while let Some(event) = events.next().await {
                let finished = !matches!(event, ai::ChatEvent::Text(_));
                let updated = this.update_in(cx, |this, window, cx| {
                    this.on_assistant_event(event, &text, window, cx);
                });
                if updated.is_err() || finished {
                    break;
                }
            }
        }));
        cx.notify();
    }

    /// AI 설정(제공자·프로필·리전)이 바뀌었으면 그 리전에서 쓸 수 있는 모델을 다시 확인한다.
    pub(super) fn refresh_assistant_models(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let prefs = cx.global::<crate::settings::Prefs>();
        let key = match (prefs.ai_provider, &prefs.aws_profile) {
            (Some(crate::settings::AiProvider::Bedrock), Some(profile)) => Some((
                profile.to_string(),
                prefs.aws_region.as_ref().map(ToString::to_string),
            )),
            _ => None,
        };
        let chat = &mut self.assistant_chat;
        if chat.models_for == key && !matches!(chat.models, ModelLookup::Failed(_)) {
            return;
        }
        chat.models_for = key.clone();
        chat.model_picker.update(cx, |picker, cx| {
            picker.set_items(SearchableVec::new(Vec::<ModelItem>::new()), window, cx);
        });
        let Some((profile, region)) = key else {
            chat.models = ModelLookup::Idle;
            chat.model_lookup = None;
            cx.notify();
            return;
        };
        chat.models = ModelLookup::Loading;
        let lookup = ai::available_models(profile, region);
        chat.model_lookup = Some(cx.spawn_in(window, async move |this, cx| {
            let result = lookup
                .await
                .unwrap_or_else(|_| Err("확인이 중간에 멈췄습니다.".into()));
            this.update_in(cx, |this, window, cx| {
                this.apply_assistant_models(result, window, cx)
            })
            .ok();
        }));
        cx.notify();
    }

    /// 확인한 모델로 고르기 목록을 채운다. 저장한 모델이 이 리전에서는 다른 추론 프로필
    /// (`us.` ↔ `global.` 등)이면 그 ID로 바꿔 저장한다.
    fn apply_assistant_models(
        &mut self,
        result: Result<Vec<(ai::BedrockModel, String)>, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let models = match result {
            Ok(models) => models,
            Err(err) => {
                self.assistant_chat.models = ModelLookup::Failed(err.into());
                cx.notify();
                return;
            }
        };
        let saved = cx.global::<crate::settings::Prefs>().ai_model.clone();
        // 고른 적이 없으면 기본 모델(Opus 5.5)을 고른 것으로 보인다.
        let wanted = saved.as_deref().unwrap_or(ai::DEFAULT_MODEL);
        let selected = models.iter().position(|(model, _)| model.matches(wanted));
        if let (Some(ix), Some(saved)) = (selected, &saved)
            && models[ix].1 != saved.as_ref()
        {
            crate::settings::Prefs::set_ai_model(Some(models[ix].1.clone().into()), cx);
        }
        let items: Vec<_> = models
            .iter()
            .map(|(model, id)| ModelItem {
                title: model.name.into(),
                id: id.clone().into(),
            })
            .collect();
        let chat = &mut self.assistant_chat;
        chat.model_picker.update(cx, |picker, cx| {
            picker.set_items(SearchableVec::new(items), window, cx);
            picker.set_selected_index(selected.map(IndexPath::new), window, cx);
        });
        chat.models = ModelLookup::Ready(models);
        cx.notify();
    }

    /// 지금 고른 모델로 보낼 수 있는지. 목록을 확인하지 못했으면 일단 보내 본다(실패하면 안내).
    fn assistant_model_usable(&self, cx: &App) -> bool {
        match &self.assistant_chat.models {
            ModelLookup::Ready(models) => {
                let prefs = cx.global::<crate::settings::Prefs>();
                let wanted = prefs.ai_model.as_deref().unwrap_or(ai::DEFAULT_MODEL);
                models.iter().any(|(model, _)| model.matches(wanted))
            }
            ModelLookup::Loading => false,
            ModelLookup::Idle | ModelLookup::Failed(_) => true,
        }
    }

    /// 입력창 아래 모델 고르기. 확인 중·실패·없음이면 고르기 대신 짧은 안내를 둔다.
    fn render_model_picker(&self, pal: Palette) -> AnyElement {
        let note = |text: &'static str, color: Hsla| {
            div()
                .px_2()
                .text_xs()
                .truncate()
                .text_color(color)
                .child(text)
                .into_any_element()
        };
        match &self.assistant_chat.models {
            ModelLookup::Idle => note("", pal.muted),
            ModelLookup::Loading => note("모델 확인 중…", pal.muted),
            ModelLookup::Failed(err) => div()
                .id("assistant-model-failed")
                .px_2()
                .text_xs()
                .truncate()
                .text_color(pal.danger)
                .tooltip({
                    let err = err.clone();
                    move |window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new(err.clone()).build(window, cx)
                    }
                })
                .child("모델을 확인하지 못했습니다")
                .into_any_element(),
            ModelLookup::Ready(models) if models.is_empty() => {
                note("이 리전에 쓸 수 있는 모델이 없습니다", pal.danger)
            }
            ModelLookup::Ready(_) => Select::new(&self.assistant_chat.model_picker)
                .small()
                .appearance(false)
                .focus_ring(false)
                .placeholder("모델 선택")
                .menu_width(px(220.0))
                .into_any_element(),
        }
    }

    /// 받은 답 조각을 화면 상태에 반영한다.
    fn on_assistant_event(
        &mut self,
        event: ai::ChatEvent,
        sent: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let chat = &mut self.assistant_chat;
        match event {
            ai::ChatEvent::Text(piece) => {
                if let Some(turn) = chat.turns.last_mut().filter(|turn| !turn.user) {
                    turn.text.push_str(&piece);
                }
            }
            ai::ChatEvent::Done => chat.streaming = false,
            ai::ChatEvent::Failed(message) => {
                chat.streaming = false;
                chat.error = Some(format!("답을 받지 못했습니다: {message}").into());
                // 답이 한 글자도 오지 않았으면 보낸 말을 입력창으로 되돌려 다시 보낼 수 있게 한다.
                if chat
                    .turns
                    .last()
                    .is_some_and(|turn| !turn.user && turn.text.is_empty())
                {
                    chat.turns.pop();
                    chat.turns.pop();
                    let sent = sent.to_string();
                    chat.input
                        .update(cx, |input, cx| input.set_value(sent, window, cx));
                }
            }
        }
        chat.scroll.scroll_to_bottom();
        cx.notify();
    }

    /// 받는 중인 답을 멈춘다. 받은 데까지는 남긴다.
    fn stop_assistant_reply(&mut self, cx: &mut Context<Self>) {
        let chat = &mut self.assistant_chat;
        chat.reply = None;
        chat.streaming = false;
        if chat
            .turns
            .last()
            .is_some_and(|turn| !turn.user && turn.text.is_empty())
        {
            chat.turns.pop();
        }
        cx.notify();
    }

    /// 새 대화를 비운다(받는 중인 답도 멈춘다).
    fn reset_live_chat(&mut self, cx: &mut Context<Self>) {
        let chat = &mut self.assistant_chat;
        chat.reply = None;
        chat.streaming = false;
        chat.turns.clear();
        chat.error = None;
        cx.notify();
    }

    /// 설정과 열린 시트로 Bedrock 요청을 만든다. AI를 설정하지 않았으면 `None`.
    fn assistant_request(&self, text: &str, cx: &App) -> Option<ai::ChatRequest> {
        let prefs = cx.global::<crate::settings::Prefs>();
        let profile = match prefs.ai_provider {
            Some(crate::settings::AiProvider::Bedrock) => prefs.aws_profile.clone()?,
            None => return None,
        };
        let mut turns: Vec<ai::Turn> = self
            .assistant_chat
            .turns
            .iter()
            .filter(|turn| !turn.text.is_empty())
            .cloned()
            .collect();
        turns.push(ai::Turn {
            user: true,
            text: text.to_string(),
        });
        Some(ai::ChatRequest {
            profile: profile.to_string(),
            region: prefs.aws_region.as_ref().map(ToString::to_string),
            model: prefs
                .ai_model
                .as_ref()
                .map_or_else(|| ai::DEFAULT_MODEL.to_string(), ToString::to_string),
            system: self.assistant_system_prompt(cx),
            turns,
        })
    }

    /// 시스템 프롬프트: 앱과 역할 안내 + 열린 시트 본문.
    fn assistant_system_prompt(&self, cx: &App) -> String {
        let mut prompt = String::from(
            "당신은 글쓰기 앱 Writer 안의 글쓰기 도우미입니다. 사용자는 마크다운으로 원고(시트)를 쓰고 \
             있으며, 지금 열린 시트를 함께 보며 묻습니다. 사용자가 쓰는 말로 답하세요. 문장을 \
             고치자고 할 때는 원래 문장과 바꿀 문장을 함께 보여 주세요.",
        );
        if let Some(editor) = &self.editor {
            let title = self
                .previews
                .get(editor.path())
                .map(|(_, preview)| preview.title.trim().to_string())
                .filter(|title| !title.is_empty())
                .unwrap_or_else(|| "제목 없는 시트".to_string());
            let body = editor.state().read(cx).value();
            prompt.push_str(&format!(
                "\n\n지금 열린 시트입니다.\n<sheet title=\"{title}\">\n{body}\n</sheet>"
            ));
        }
        prompt
    }
}

/// AI를 쓸 준비가 됐는지: 제공자를 골랐고, Bedrock이면 AWS 프로필도 골랐다.
fn ai_configured(cx: &App) -> bool {
    let prefs = cx.global::<crate::settings::Prefs>();
    match prefs.ai_provider {
        Some(crate::settings::AiProvider::Bedrock) => prefs.aws_profile.is_some(),
        None => false,
    }
}

/// 대화 한 줄. 내 말은 오른쪽 말풍선, 어시스턴트 말은 왼쪽 본문으로 그린다.
fn message_bubble(user: bool, text: SharedString, pal: Palette) -> AnyElement {
    if user {
        div()
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
                    .child(text),
            )
            .into_any_element()
    } else {
        div()
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
                    .child(text),
            )
            .into_any_element()
    }
}
