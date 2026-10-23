//! 오른쪽 어시스턴트(LLM) 패널. 에디터 툴바 오른쪽 끝 버튼으로 열고 닫는다.
//!
//! 새 대화는 설정의 Bedrock 프로필로 실제 모델과 주고받고, 답을 받는 대로 조금씩 그린다
//! ([`crate::agent`]). 대화 상태와 프롬프트는 에이전트가 맡고, 여기서는 그리기만 한다. 기록의 지난 대화는 아직 저장하지 않아 샘플을 보여 준다.

use std::cell::RefCell;
use std::collections::HashMap;

use futures::StreamExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::component::searchable_list::{SearchableListItem, SearchableVec};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::*;
use gpui_kit::gpui::prelude::FluentBuilder;
use gpui_kit::gpui::*;

use super::WriterApp;
use super::style::*;
use crate::{agent, icons};

/// 새 대화(실제 모델과 주고받는 대화)의 상태.
pub(super) struct LiveChat {
    /// 입력창. Enter로 보내고 Shift+Enter로 줄을 바꾼다.
    input: Entity<TextareaState>,
    conversation: agent::Conversation,
    /// 답을 받는 태스크. 버리면 받기를 멈춘다.
    reply: Option<Task<()>>,
    /// 답이 자랄 때 맨 아래로 따라가는 스크롤.
    scroll: ScrollHandle,
    /// 승인 카드마다 diff 칸의 스크롤. 넘칠 때만 휠을 가져가 대화가 같이 움직이지 않게 한다.
    change_scrolls: RefCell<HashMap<u64, ScrollHandle>>,
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
    Ready(Vec<(agent::BedrockModel, String)>),
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
                .placeholder(t!("assistant.input.placeholder"))
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
            conversation: agent::Conversation::default(),
            reply: None,
            scroll: ScrollHandle::new(),
            change_scrolls: RefCell::default(),
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
    fn label(self) -> std::borrow::Cow<'static, str> {
        match self {
            Period::Today => t!("assistant.history.today"),
            Period::Yesterday => t!("assistant.history.yesterday"),
            Period::ThisWeek => t!("assistant.history.last_7_days"),
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

/// 빈 대화에서 보여 줄 질문 예시(번역 키). 누르면 그 언어의 글을 보낸다.
const SUGGESTIONS: &[&str] = &[
    "assistant.suggestion.summarize",
    "assistant.suggestion.fix_awkward",
    "assistant.suggestion.spelling",
    "assistant.suggestion.next_scene",
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
                SharedString::from(t!("assistant.history.title"))
            } else {
                conversation.map_or_else(
                    || {
                        live.conversation
                            .title()
                            .map(|title| SharedString::from(title.to_string()))
                            .unwrap_or_else(|| t!("assistant.new_conversation").into())
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
            None if live.conversation.is_empty() => self
                .render_assistant_empty(configured, cx, pal)
                .into_any_element(),
            None => {
                let turns = live.conversation.turns();
                let streaming = live.conversation.is_streaming();
                let last = turns.len().saturating_sub(1);
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
                    .children(turns.iter().enumerate().map(|(ix, turn)| {
                        // 글이 오기 전의 빈 답은 받는 중 표시로 그린다.
                        let text = if !turn.user && turn.text.is_empty() && streaming && ix == last
                        {
                            "…".into()
                        } else {
                            turn.text.trim_end().to_string().into()
                        };
                        if turn.user {
                            message_bubble(true, text, pal)
                        } else {
                            self.render_reply(text, &turn.activity, cx, pal)
                        }
                    }))
                    .children(live.conversation.error().map(|error| {
                        div()
                            .px_3()
                            .py_2()
                            .rounded_lg()
                            .border_1()
                            .border_color(pal.danger.opacity(0.4))
                            .text_sm()
                            .text_color(pal.danger)
                            .child(t!("assistant.error.no_reply", error = error))
                    }))
                    .into_any_element()
            }
        };

        // 입력창. 새 대화에서만 쓰고(샘플 대화는 읽기 전용), AI를 설정하지 않았으면 두지 않는다.
        // 답을 받는 중에는 보내기 대신 멈추기 버튼을 둔다.
        let can_send = !live.conversation.is_streaming()
            && self.assistant_model_usable(cx)
            && !live.input.read(cx).value().trim().is_empty();
        let send_button = if live.conversation.is_streaming() {
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
            .unwrap_or_else(|| t!("assistant.untitled_sheet").to_string());

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

        let body = if configured {
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap_1()
                .child(
                    div()
                        .text_lg()
                        .font_semibold()
                        .child(t!("assistant.empty.title")),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(pal.muted)
                        .text_center()
                        .child(t!("assistant.empty.about_sheet", title = sheet_title)),
                )
                .child(div().w_full().mt_5().flex().flex_col().gap_2().children(
                    SUGGESTIONS.iter().enumerate().map(|(ix, key)| {
                        let text = t!(*key).to_string();
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
                            .child(text.clone())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.send_assistant_message(text.clone(), window, cx);
                            }))
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
                        .child(t!("assistant.setup.title")),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(pal.muted)
                        .text_center()
                        .child(t!("assistant.setup.description")),
                )
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
                        .child(t!("assistant.setup.open_settings")),
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
        let Some(backend) = agent::Backend::from_prefs(cx.global::<crate::settings::Prefs>())
        else {
            return;
        };
        if text.trim().is_empty() || self.assistant_chat.conversation.is_streaming() {
            return;
        }
        // 에이전트가 열린 시트를 고칠 수 있으므로 먼저 저장해 둔다. 저장하지 않은 변경이 남아 있으면
        // 에이전트가 쓴 내용을 다음 자동 저장이 덮어쓴다.
        if let Err(err) = self.save_open_sheet(cx) {
            self.notify(t!("assistant.error.save_sheet", err = err), window, cx);
            return;
        }
        let workspace = self
            .library
            .as_ref()
            .map(|library| agent::Workspace::new(&library.root));
        let system = self.assistant_system_prompt(workspace.as_ref(), cx);
        let chat = &mut self.assistant_chat;
        let Some(turns) = chat.conversation.send(&text) else {
            return;
        };

        // 샘플 대화를 보고 있었어도 새 대화로 돌아와 보낸다.
        self.assistant_conversation = None;
        self.assistant_history = false;
        let chat = &mut self.assistant_chat;
        chat.input
            .update(cx, |input, cx| input.set_value("", window, cx));
        chat.scroll.scroll_to_bottom();

        let mut events = agent::stream(agent::ChatRequest {
            backend,
            workspace,
            system,
            turns,
            allowed_sites: chat.conversation.allowed_sites(),
        });
        chat.reply = Some(cx.spawn_in(window, async move |this, cx| {
            while let Some(event) = events.next().await {
                let finished =
                    matches!(event, agent::ChatEvent::Done | agent::ChatEvent::Failed(_));
                let updated = this.update_in(cx, |this, window, cx| {
                    this.on_assistant_event(event, window, cx);
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
        let key =
            agent::Backend::from_prefs(prefs).map(|backend| (backend.profile, backend.region));
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
        let lookup = agent::available_models(profile, region);
        chat.model_lookup = Some(cx.spawn_in(window, async move |this, cx| {
            let result = lookup
                .await
                .unwrap_or_else(|_| Err(t!("assistant.model.lookup_stopped").into()));
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
        result: Result<Vec<(agent::BedrockModel, String)>, String>,
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
        let wanted = saved.as_deref().unwrap_or(agent::DEFAULT_MODEL);
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
                let wanted = prefs.ai_model.as_deref().unwrap_or(agent::DEFAULT_MODEL);
                models.iter().any(|(model, _)| model.matches(wanted))
            }
            ModelLookup::Loading => false,
            ModelLookup::Idle | ModelLookup::Failed(_) => true,
        }
    }

    /// 입력창 아래 모델 고르기. 확인 중·실패·없음이면 고르기 대신 짧은 안내를 둔다.
    fn render_model_picker(&self, pal: Palette) -> AnyElement {
        let note = |text: std::borrow::Cow<'static, str>, color: Hsla| {
            div()
                .px_2()
                .text_xs()
                .truncate()
                .text_color(color)
                .child(text)
                .into_any_element()
        };
        match &self.assistant_chat.models {
            ModelLookup::Idle => note("".into(), pal.muted),
            ModelLookup::Loading => note(t!("assistant.model.loading"), pal.muted),
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
                .child(t!("assistant.model.failed"))
                .into_any_element(),
            ModelLookup::Ready(models) if models.is_empty() => {
                note(t!("assistant.model.none_in_region"), pal.danger)
            }
            ModelLookup::Ready(_) => Select::new(&self.assistant_chat.model_picker)
                .small()
                .appearance(false)
                .focus_ring(false)
                .placeholder(t!("assistant.model.placeholder"))
                .menu_width(px(220.0))
                .into_any_element(),
        }
    }

    /// 받은 일을 대화에 반영한다. 답이 한 글자도 오지 않고 실패했으면 보낸 말을 입력창으로
    /// 되돌려 다시 보낼 수 있게 한다.
    fn on_assistant_event(
        &mut self,
        event: agent::ChatEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let chat = &mut self.assistant_chat;
        if let Some(sent) = chat.conversation.apply(event) {
            chat.input
                .update(cx, |input, cx| input.set_value(sent, window, cx));
        }
        chat.scroll.scroll_to_bottom();
        cx.notify();
    }

    /// 받는 중인 답을 멈춘다. 받은 데까지는 남긴다.
    fn stop_assistant_reply(&mut self, cx: &mut Context<Self>) {
        let chat = &mut self.assistant_chat;
        chat.reply = None;
        chat.conversation.stop();
        cx.notify();
    }

    /// 새 대화를 비운다(받는 중인 답도 멈춘다).
    fn reset_live_chat(&mut self, cx: &mut Context<Self>) {
        let chat = &mut self.assistant_chat;
        chat.reply = None;
        chat.conversation.clear();
        chat.change_scrolls.get_mut().clear();
        cx.notify();
    }

    /// 열린 시트를 넣은 시스템 프롬프트.
    fn assistant_system_prompt(&self, workspace: Option<&agent::Workspace>, cx: &App) -> String {
        let tools = workspace.is_some();
        let skills = workspace.map_or(&[][..], agent::Workspace::skills);
        let extensions = workspace.map_or(&[][..], agent::Workspace::extensions);
        let Some(editor) = &self.editor else {
            return agent::system_prompt(None, tools, skills, extensions);
        };
        let path = self.library.as_ref().and_then(|library| {
            let relative = editor.path().strip_prefix(&library.root).ok()?;
            Some(relative.to_string_lossy().into_owned())
        });
        let title = self
            .previews
            .get(editor.path())
            .map(|(_, preview)| preview.title.to_string())
            .unwrap_or_default();
        // 파일과 같은 마크다운으로 넣는다(에디터 본문은 줄 하나가 문단이라 파일과 다르다). 그래야
        // edit_file의 old를 파일에서 그대로 찾는다.
        let body = crate::text_transform::to_markdown(&editor.state().read(cx).value());
        agent::system_prompt(
            Some(agent::Sheet {
                title: &title,
                path: path.as_deref(),
                body: &body,
            }),
            tools,
            skills,
            extensions,
        )
    }
}

impl WriterApp {
    /// 어시스턴트 답: 한 일(도구 줄, 승인 카드)과 글.
    fn render_reply(
        &self,
        text: SharedString,
        activity: &[agent::Activity],
        cx: &mut Context<Self>,
        pal: Palette,
    ) -> AnyElement {
        let pending = self.assistant_chat.conversation.pending_change();
        let activity = activity
            .iter()
            .map(|activity| match activity {
                agent::Activity::Tool(summary) => div()
                    .text_xs()
                    .truncate()
                    .text_color(pal.muted)
                    .child(SharedString::from(summary.clone()))
                    .into_any_element(),
                agent::Activity::Change(card) => {
                    let scroll = self
                        .assistant_chat
                        .change_scrolls
                        .borrow_mut()
                        .entry(card.id)
                        .or_default()
                        .clone();
                    change_card(card, pending == Some(card.id), &scroll, cx, pal)
                }
                agent::Activity::Fetch(card) => fetch_card(card, pending == Some(card.id), cx, pal),
            })
            .collect();
        assistant_reply(false, text, activity, pal)
    }
}

/// 승인 카드: 바꿀 파일, 변경 내용(diff), 그리고 기다리는 중이면 [취소] [적용] 버튼.
fn change_card(
    card: &agent::ChangeCard,
    waiting: bool,
    scroll: &ScrollHandle,
    cx: &mut Context<WriterApp>,
    pal: Palette,
) -> AnyElement {
    let id = card.id as usize;
    let status = match card.state {
        agent::ChangeState::Pending if card.new_file => t!("assistant.change.pending_new"),
        agent::ChangeState::Pending => t!("assistant.change.pending_edit"),
        agent::ChangeState::Applied => t!("assistant.change.applied"),
        agent::ChangeState::Declined => t!("assistant.change.declined"),
        agent::ChangeState::Failed => t!("assistant.change.failed"),
    };
    let status_color = match card.state {
        agent::ChangeState::Applied => pal.success,
        agent::ChangeState::Failed => pal.danger,
        _ => pal.muted,
    };
    let card_id = card.id;
    let decide = |approve: bool| {
        cx.listener(move |this: &mut WriterApp, _: &ClickEvent, _, cx| {
            this.assistant_chat.conversation.decide(card_id, approve);
            cx.notify();
        })
    };
    let (decline, apply) = (decide(false), decide(true));
    div()
        .w_full()
        .rounded_lg()
        .border_1()
        .border_color(pal.panel_border)
        .overflow_hidden()
        .child(
            div()
                .px_2()
                .py_1()
                .flex()
                .items_center()
                .gap_2()
                .text_xs()
                .bg(pal.hover)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .child(SharedString::from(card.path.clone())),
                )
                .child(div().flex_shrink_0().text_color(status_color).child(status)),
        )
        .child(
            div()
                .id(("change-lines", id))
                .track_scroll(scroll)
                .max_h(px(260.0))
                .overflow_y_scroll()
                // 바깥 대화 목록도 같은 휠을 받으므로, diff가 넘쳐 스스로 스크롤할 때는 넘기지 않는다.
                .on_scroll_wheel({
                    let scroll = scroll.clone();
                    move |_, _, cx| {
                        if scroll.max_offset().y > px(0.0) {
                            cx.stop_propagation();
                        }
                    }
                })
                .py_1()
                .flex()
                .flex_col()
                .children(card.lines.iter().map(|line| diff_line(line, pal))),
        )
        .when(waiting, |this| {
            this.child(
                div()
                    .px_2()
                    .py_1()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .border_t_1()
                    .border_color(pal.panel_border)
                    .child(
                        Button::new(("change-decline", id))
                            .small()
                            .label(t!("common.cancel"))
                            .on_click(decline),
                    )
                    .child(
                        Button::new(("change-apply", id))
                            .small()
                            .primary()
                            .label(t!("assistant.change.apply"))
                            .on_click(apply),
                    ),
            )
        })
        .into_any_element()
}

/// 웹 페이지 읽기 허락 카드: 읽을 주소 전체와, 기다리는 중이면 [거절] [허용] 버튼.
/// 주소를 다 보여 주는 것은 모델이 주소에 글을 실어 내보내지 않는지 사용자가 볼 수 있게 하려는 것이다.
fn fetch_card(
    card: &agent::FetchCard,
    waiting: bool,
    cx: &mut Context<WriterApp>,
    pal: Palette,
) -> AnyElement {
    let id = card.id as usize;
    let (status, status_color) = match card.state {
        agent::ChangeState::Pending => (t!("assistant.fetch.pending"), pal.muted),
        agent::ChangeState::Applied => (t!("assistant.fetch.allowed"), pal.success),
        agent::ChangeState::Declined | agent::ChangeState::Failed => {
            (t!("assistant.fetch.declined"), pal.muted)
        }
    };
    let card_id = card.id;
    let decide = |approve: bool| {
        cx.listener(move |this: &mut WriterApp, _: &ClickEvent, _, cx| {
            this.assistant_chat.conversation.decide(card_id, approve);
            cx.notify();
        })
    };
    let (decline, allow) = (decide(false), decide(true));
    div()
        .w_full()
        .rounded_lg()
        .border_1()
        .border_color(pal.panel_border)
        .overflow_hidden()
        .child(
            div()
                .px_2()
                .py_1()
                .flex()
                .items_center()
                .gap_2()
                .text_xs()
                .bg(pal.hover)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .child(t!("assistant.fetch.title", site = card.site.clone())),
                )
                .child(div().flex_shrink_0().text_color(status_color).child(status)),
        )
        .child(
            div()
                .px_2()
                .py_1()
                .text_xs()
                .font_family("monospace")
                .text_color(pal.foreground)
                .child(SharedString::from(card.url.clone())),
        )
        .when(waiting, |this| {
            this.child(
                div()
                    .px_2()
                    .py_1()
                    .flex()
                    .items_center()
                    .gap_2()
                    .border_t_1()
                    .border_color(pal.panel_border)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_xs()
                            .text_color(pal.muted)
                            .child(t!("assistant.fetch.note", site = card.site.clone())),
                    )
                    .child(
                        Button::new(("fetch-decline", id))
                            .small()
                            .label(t!("assistant.fetch.decline"))
                            .on_click(decline),
                    )
                    .child(
                        Button::new(("fetch-allow", id))
                            .small()
                            .primary()
                            .label(t!("assistant.fetch.allow"))
                            .on_click(allow),
                    ),
            )
        })
        .into_any_element()
}

/// diff 한 줄. 지운 줄은 빨강, 더한 줄은 초록 바탕이고, 줄 안에서 바뀐 글자는 더 진하게 칠한다.
fn diff_line(line: &agent::DiffLine, pal: Palette) -> AnyElement {
    let (sign, color) = match line.kind {
        agent::LineKind::Removed => ("−", Some(pal.danger)),
        agent::LineKind::Added => ("+", Some(pal.success)),
        agent::LineKind::Context => (" ", None),
        agent::LineKind::Gap => {
            return div()
                .px_2()
                .text_xs()
                .text_color(pal.muted)
                .child("⋯")
                .into_any_element();
        }
    };
    // 빈 줄도 한 줄 높이를 차지하게 한다.
    let text: SharedString = if line.text.is_empty() {
        " ".into()
    } else {
        line.text.clone().into()
    };
    let highlights: Vec<_> = match color {
        Some(color) if !line.text.is_empty() => line
            .emphasis
            .iter()
            .map(|range| {
                (
                    range.clone(),
                    HighlightStyle {
                        background_color: Some(color.opacity(0.35)),
                        ..Default::default()
                    },
                )
            })
            .collect(),
        _ => Vec::new(),
    };
    div()
        .px_2()
        .flex()
        .gap_1()
        .text_xs()
        .line_height(relative(1.5))
        .when_some(color, |this, color| this.bg(color.opacity(0.1)))
        .when(color.is_none(), |this| this.text_color(pal.muted))
        .child(div().flex_shrink_0().w(px(10.0)).child(sign))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(StyledText::new(text).with_highlights(highlights)),
        )
        .into_any_element()
}

/// AI를 쓸 준비가 됐는지: 제공자를 골랐고, Bedrock이면 AWS 프로필도 골랐다.
fn ai_configured(cx: &App) -> bool {
    agent::Backend::from_prefs(cx.global::<crate::settings::Prefs>()).is_some()
}

/// 대화 한 줄. 내 말은 오른쪽 말풍선, 어시스턴트 말은 왼쪽 본문으로 그린다.
/// 말풍선. 어시스턴트 답이면 한 일(`activity`: 부른 도구, 승인 카드)을 글 위에 둔다.
fn message_bubble(user: bool, text: SharedString, pal: Palette) -> AnyElement {
    assistant_reply(user, text, Vec::new(), pal)
}

fn assistant_reply(
    user: bool,
    text: SharedString,
    activity: Vec<AnyElement>,
    pal: Palette,
) -> AnyElement {
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
                    .flex()
                    .flex_col()
                    .gap_1()
                    .children(activity)
                    .when(!text.is_empty(), |this| {
                        this.child(div().text_sm().line_height(relative(1.5)).child(text))
                    }),
            )
            .into_any_element()
    }
}
