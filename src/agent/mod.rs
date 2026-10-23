//! 어시스턴트 에이전트: 대화 상태, 시스템 프롬프트, 모델 호출. 화면(gpui)과 떨어져 있어
//! 패널은 [`Conversation`]을 들고 그리기만 한다.

mod bedrock;
mod diff;
mod edit;
mod prompt;
mod review;
pub mod skills;
mod tools;
mod web;

pub use bedrock::{BedrockModel, ChatRequest, DEFAULT_MODEL, available_models, stream};
pub use diff::{DiffLine, LineKind};
pub use prompt::{Sheet, system_prompt};
pub use tools::Workspace;

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use futures::channel::oneshot;

use crate::settings::{AiProvider, Prefs};

/// 대화의 말 한 마디.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Turn {
    /// 사용자가 한 말이면 `true`, 어시스턴트 답이면 `false`.
    pub user: bool,
    pub text: String,
    /// 어시스턴트가 이 답을 쓰며 한 일(부른 도구, 파일 변경). 화면에만 보이고 모델에는 다시
    /// 보내지 않는다.
    pub activity: Vec<Activity>,
}

/// 답 하나에서 어시스턴트가 한 일.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Activity {
    /// 도구를 불렀다. 화면에 보일 한 줄.
    Tool(String),
    /// 파일을 바꾸려 했다(승인 카드).
    Change(ChangeCard),
    /// 처음 보는 사이트의 웹 페이지를 읽으려 했다(허락 카드).
    Fetch(FetchCard),
}

/// 웹 페이지 읽기 허락 카드. 허락하면 그 사이트는 이 대화에서 다시 묻지 않는다.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchCard {
    pub id: u64,
    pub url: String,
    /// 허락하는 사이트(호스트).
    pub site: String,
    /// `Applied`면 허락했다.
    pub state: ChangeState,
}

/// 승인 카드: 바꿀 파일과 변경 내용.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangeCard {
    /// 대화 안에서 카드를 가리키는 번호.
    pub id: u64,
    /// 바꿀 파일(라이브러리 루트 기준). 새 시트면 그룹 폴더.
    pub path: String,
    /// 새 파일(새 시트 포함)을 만드는지.
    pub new_file: bool,
    pub lines: Vec<DiffLine>,
    pub state: ChangeState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeState {
    /// 사용자의 승인을 기다린다.
    Pending,
    Applied,
    Declined,
    /// 승인했지만 쓰지 못했다(그사이 파일이 바뀌었거나 쓰기 실패).
    Failed,
}

/// 승인 요청: 카드와, 사용자의 답(`true`면 적용)을 돌려보낼 곳.
#[derive(Debug)]
pub struct Approval {
    pub card: ChangeCard,
    pub reply: oneshot::Sender<bool>,
}

/// 웹 페이지를 읽어도 되는지 묻는다.
#[derive(Debug)]
pub struct FetchApproval {
    pub card: FetchCard,
    pub reply: oneshot::Sender<bool>,
}

/// 이 대화에서 읽기를 허락한 사이트. 요청과 함께 모델 호출 쪽에 넘긴다.
pub type AllowedSites = Arc<Mutex<HashSet<String>>>;

/// 답을 받는 동안 화면에 넘기는 일.
#[derive(Debug)]
pub enum ChatEvent {
    /// 답 글이 이만큼 더 왔다.
    Text(String),
    /// 도구를 불렀다. 화면에 보일 한 줄.
    Tool(String),
    /// 파일을 바꾸려 한다. 사용자가 답할 때까지 도구가 기다린다.
    Approve(Approval),
    /// 처음 보는 사이트를 읽으려 한다. 사용자가 답할 때까지 도구가 기다린다.
    ApproveFetch(FetchApproval),
    /// 승인한 변경을 쓰지 못했다(카드 번호).
    ChangeFailed(u64),
    /// 답이 끝났다.
    Done,
    /// 요청이 실패했다(자격 증명, 권한, 모델 ID 등). 사람이 읽을 설명.
    Failed(String),
}

/// 모델을 부를 곳.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Backend {
    /// 자격 증명을 읽을 AWS 프로필(`~/.aws/config`).
    pub profile: String,
    /// 부를 리전. 없으면 프로필의 리전을 따른다.
    pub region: Option<String>,
    /// Bedrock 모델 ID 또는 추론 프로필 ID.
    pub model: String,
}

impl Backend {
    /// 설정에서 고른 곳. AI를 설정하지 않았으면(제공자나 AWS 프로필이 없으면) `None`.
    pub fn from_prefs(prefs: &Prefs) -> Option<Self> {
        match prefs.ai_provider {
            Some(AiProvider::Bedrock) => Some(Self {
                profile: prefs.aws_profile.as_ref()?.to_string(),
                region: prefs.aws_region.as_ref().map(ToString::to_string),
                model: prefs
                    .ai_model
                    .as_ref()
                    .map_or_else(|| DEFAULT_MODEL.to_string(), ToString::to_string),
            }),
            None => None,
        }
    }
}

/// 모델과 주고받는 대화 하나. 답을 받는 중이면 마지막 말은 자라는 어시스턴트 답이다.
#[derive(Debug, Default)]
pub struct Conversation {
    turns: Vec<Turn>,
    streaming: bool,
    /// 마지막 요청이 실패한 까닭.
    error: Option<String>,
    /// 답을 기다리는 승인 카드와 답을 돌려보낼 곳. 버리면 도구는 거절로 알고 멈춘다.
    pending: Option<(u64, oneshot::Sender<bool>)>,
    /// 읽기를 허락한 사이트.
    allowed_sites: AllowedSites,
}

impl Conversation {
    pub fn turns(&self) -> &[Turn] {
        &self.turns
    }

    /// 읽기를 허락한 사이트(모델 호출 쪽과 함께 쓴다).
    pub fn allowed_sites(&self) -> AllowedSites {
        self.allowed_sites.clone()
    }

    pub fn is_streaming(&self) -> bool {
        self.streaming
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// 주고받은 말도, 실패도 없다.
    pub fn is_empty(&self) -> bool {
        self.turns.is_empty() && self.error.is_none()
    }

    /// 첫 사용자 말(대화 제목으로 쓴다).
    pub fn title(&self) -> Option<&str> {
        self.turns.first().map(|turn| turn.text.as_str())
    }

    /// 사용자 말을 더하고 받을 답 자리를 연다. 모델에 보낼 대화(이번 말이 마지막)를 돌려준다.
    /// 답을 받는 중이거나 빈 말이면 아무것도 하지 않고 `None`.
    pub fn send(&mut self, text: &str) -> Option<Vec<Turn>> {
        let text = text.trim();
        if text.is_empty() || self.streaming {
            return None;
        }
        self.turns.push(Turn {
            user: true,
            text: text.to_string(),
            ..Turn::default()
        });
        let turns = request_turns(&self.turns);
        self.turns.push(Turn::default());
        self.error = None;
        self.streaming = true;
        Some(turns)
    }

    /// 받은 일을 대화에 반영한다. 답이 한 글자도 오지 않고 실패했으면 보낸 말을 대화에서 빼고
    /// 돌려준다(입력창에 되돌려 다시 보낼 수 있게).
    pub fn apply(&mut self, event: ChatEvent) -> Option<String> {
        match event {
            ChatEvent::Text(piece) => {
                if let Some(turn) = self.reply_mut() {
                    turn.text.push_str(&piece);
                }
                None
            }
            ChatEvent::Tool(summary) => {
                self.push_activity(Activity::Tool(summary));
                None
            }
            ChatEvent::Approve(Approval { card, reply }) => {
                self.pending = Some((card.id, reply));
                self.push_activity(Activity::Change(card));
                None
            }
            ChatEvent::ApproveFetch(FetchApproval { card, reply }) => {
                self.pending = Some((card.id, reply));
                self.push_activity(Activity::Fetch(card));
                None
            }
            ChatEvent::ChangeFailed(id) => {
                self.set_change_state(id, ChangeState::Failed);
                None
            }
            ChatEvent::Done => {
                self.streaming = false;
                self.drop_pending();
                None
            }
            ChatEvent::Failed(message) => {
                self.streaming = false;
                self.drop_pending();
                self.error = Some(message);
                if !self.ends_with_empty_reply() {
                    return None;
                }
                self.turns.pop();
                self.turns
                    .pop()
                    .filter(|turn| turn.user)
                    .map(|turn| turn.text)
            }
        }
    }

    /// 답을 기다리는 승인 카드의 번호.
    pub fn pending_change(&self) -> Option<u64> {
        self.pending.as_ref().map(|(id, _)| *id)
    }

    /// 승인 카드에 답한다(`approve`면 적용). 기다리는 카드가 아니면 아무것도 하지 않는다.
    pub fn decide(&mut self, id: u64, approve: bool) {
        if self.pending_change() != Some(id) {
            return;
        }
        if let Some((_, reply)) = self.pending.take() {
            reply.send(approve).ok();
        }
        let state = if approve {
            ChangeState::Applied
        } else {
            ChangeState::Declined
        };
        self.set_change_state(id, state);
    }

    /// 받는 중인 답을 멈춘다. 받은 데까지는 남긴다. 기다리던 승인은 거절한 것으로 본다.
    pub fn stop(&mut self) {
        self.streaming = false;
        self.drop_pending();
        if self.ends_with_empty_reply() {
            self.turns.pop();
        }
    }

    /// 대화를 비운다.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// 받는 중인 답.
    fn reply_mut(&mut self) -> Option<&mut Turn> {
        self.turns.last_mut().filter(|turn| !turn.user)
    }

    fn push_activity(&mut self, activity: Activity) {
        if let Some(turn) = self.reply_mut() {
            // 도구 앞뒤의 글이 한 문단으로 붙지 않게 한다.
            if !turn.text.is_empty() && !turn.text.ends_with("\n\n") {
                turn.text.push_str("\n\n");
            }
            turn.activity.push(activity);
        }
    }

    fn set_change_state(&mut self, id: u64, state: ChangeState) {
        let card = self
            .turns
            .iter_mut()
            .flat_map(|turn| &mut turn.activity)
            .find_map(|activity| match activity {
                Activity::Change(card) if card.id == id => Some(&mut card.state),
                Activity::Fetch(card) if card.id == id => Some(&mut card.state),
                _ => None,
            });
        if let Some(card_state) = card {
            *card_state = state;
        }
    }

    /// 기다리던 승인을 거둔다(답할 곳을 버리면 도구는 멈춘다).
    fn drop_pending(&mut self) {
        if let Some((id, _)) = self.pending.take() {
            self.set_change_state(id, ChangeState::Declined);
        }
    }

    /// 마지막 답이 글도 한 일도 없이 비어 있다.
    fn ends_with_empty_reply(&self) -> bool {
        self.turns
            .last()
            .is_some_and(|turn| !turn.user && turn.text.is_empty() && turn.activity.is_empty())
    }
}

/// 모델에 보낼 대화. 비어 있는 답(멈춘 답)은 빼고, 그래서 같은 쪽 말이 잇달으면 한 마디로
/// 합친다(Bedrock Converse는 사용자와 어시스턴트가 번갈아 말해야 받는다).
fn request_turns(turns: &[Turn]) -> Vec<Turn> {
    let mut merged: Vec<Turn> = Vec::new();
    for turn in turns.iter() {
        // 거절했거나 쓰지 못한 변경은 다음 답에서도 알도록 답 끝에 적는다(도구 결과는 다시 보내지 않는다).
        let notes: Vec<String> = turn
            .activity
            .iter()
            .filter_map(|activity| match activity {
                Activity::Change(card) if card.state == ChangeState::Declined => Some(format!(
                    "[Not applied: the user declined the change to {}.]",
                    card.path
                )),
                Activity::Change(card) if card.state == ChangeState::Failed => {
                    Some(format!("[Not applied: writing {} failed.]", card.path))
                }
                _ => None,
            })
            .collect();
        let text = std::iter::once(turn.text.trim_end())
            .chain(notes.iter().map(String::as_str))
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
        let text = text.as_str();
        if text.is_empty() {
            continue;
        }
        match merged.last_mut() {
            Some(last) if last.user == turn.user => {
                last.text.push_str("\n\n");
                last.text.push_str(text);
            }
            _ => merged.push(Turn {
                user: turn.user,
                text: text.to_string(),
                ..Turn::default()
            }),
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(user: bool, text: &str) -> Turn {
        Turn {
            user,
            text: text.into(),
            ..Turn::default()
        }
    }

    #[test]
    fn send_opens_reply_and_returns_history() {
        let mut chat = Conversation::default();
        assert_eq!(chat.send("  안녕  "), Some(vec![turn(true, "안녕")]));
        assert_eq!(chat.turns(), [turn(true, "안녕"), turn(false, "")]);
        assert!(chat.is_streaming());
        // 받는 중이거나 빈 말이면 보내지 않는다.
        assert_eq!(chat.send("또"), None);
        chat.apply(ChatEvent::Text("반가".into()));
        chat.apply(ChatEvent::Text("워요".into()));
        chat.apply(ChatEvent::Done);
        assert_eq!(chat.send("   "), None);
        assert_eq!(
            chat.send("다음"),
            Some(vec![
                turn(true, "안녕"),
                turn(false, "반가워요"),
                turn(true, "다음")
            ])
        );
    }

    #[test]
    fn failure_before_any_text_gives_back_sent_text() {
        let mut chat = Conversation::default();
        chat.send("질문");
        assert_eq!(
            chat.apply(ChatEvent::Failed("권한 없음".into())),
            Some("질문".into())
        );
        assert!(chat.turns().is_empty());
        assert_eq!(chat.error(), Some("권한 없음"));
        assert!(!chat.is_streaming());
        // 다시 보내면 실패를 지운다.
        chat.send("질문");
        assert_eq!(chat.error(), None);
    }

    #[test]
    fn failure_after_text_keeps_partial_reply() {
        let mut chat = Conversation::default();
        chat.send("질문");
        chat.apply(ChatEvent::Text("답의 앞".into()));
        assert_eq!(chat.apply(ChatEvent::Failed("끊김".into())), None);
        assert_eq!(chat.turns(), [turn(true, "질문"), turn(false, "답의 앞")]);
    }

    #[test]
    fn tool_calls_stay_on_reply_and_split_paragraphs() {
        let mut chat = Conversation::default();
        chat.send("1장 고쳐 줘");
        chat.apply(ChatEvent::Text("읽어 볼게요.".into()));
        chat.apply(ChatEvent::Tool("읽음 · 1장.md".into()));
        chat.apply(ChatEvent::Tool("고침 · 1장.md".into()));
        chat.apply(ChatEvent::Text("고쳤어요.".into()));
        chat.apply(ChatEvent::Done);
        let reply = chat.turns().last().unwrap();
        assert_eq!(reply.text, "읽어 볼게요.\n\n고쳤어요.");
        assert_eq!(
            reply.activity,
            [
                Activity::Tool("읽음 · 1장.md".into()),
                Activity::Tool("고침 · 1장.md".into())
            ]
        );
        // 도구만 부르고 실패하면(파일은 이미 바뀌었을 수 있다) 보낸 말을 되돌리지 않는다.
        chat.send("또");
        chat.apply(ChatEvent::Tool("고침 · 2장.md".into()));
        assert_eq!(chat.apply(ChatEvent::Failed("끊김".into())), None);
        assert_eq!(
            chat.turns().last().unwrap().activity,
            [Activity::Tool("고침 · 2장.md".into())]
        );
    }

    fn card(id: u64) -> ChangeCard {
        ChangeCard {
            id,
            path: "Projects/소설/1장.md".into(),
            new_file: false,
            lines: Vec::new(),
            state: ChangeState::Pending,
        }
    }

    fn state_of(chat: &Conversation, id: u64) -> ChangeState {
        chat.turns()
            .iter()
            .flat_map(|turn| &turn.activity)
            .find_map(|activity| match activity {
                Activity::Change(card) if card.id == id => Some(card.state),
                Activity::Fetch(card) if card.id == id => Some(card.state),
                _ => None,
            })
            .unwrap()
    }

    #[test]
    fn fetch_approval_waits_and_updates_card() {
        let mut chat = Conversation::default();
        chat.send("이 링크 요약해 줘").unwrap();
        let (reply, answer) = oneshot::channel();
        chat.apply(ChatEvent::ApproveFetch(FetchApproval {
            card: FetchCard {
                id: 7,
                url: "https://example.com/a".into(),
                site: "example.com".into(),
                state: ChangeState::Pending,
            },
            reply,
        }));
        assert_eq!(chat.pending_change(), Some(7));
        chat.decide(7, true);
        assert_eq!(state_of(&chat, 7), ChangeState::Applied);
        assert_eq!(futures::executor::block_on(answer), Ok(true));
        // 허락한 사이트는 대화가 들고 있고, 비우면 잊는다.
        chat.allowed_sites()
            .lock()
            .unwrap()
            .insert("example.com".into());
        chat.clear();
        assert!(chat.allowed_sites().lock().unwrap().is_empty());
    }

    #[test]
    fn approval_sends_answer_and_updates_card() {
        let mut chat = Conversation::default();
        chat.send("고쳐 줘");
        let (reply, mut answer) = oneshot::channel();
        chat.apply(ChatEvent::Approve(Approval {
            card: card(1),
            reply,
        }));
        assert_eq!(chat.pending_change(), Some(1));
        // 다른 카드 번호에는 답하지 않는다.
        chat.decide(2, true);
        assert_eq!(answer.try_recv().unwrap(), None);
        chat.decide(1, true);
        assert_eq!(answer.try_recv().unwrap(), Some(true));
        assert_eq!(state_of(&chat, 1), ChangeState::Applied);
        assert_eq!(chat.pending_change(), None);
        // 쓰기에 실패했다고 오면 카드에 보인다.
        chat.apply(ChatEvent::ChangeFailed(1));
        assert_eq!(state_of(&chat, 1), ChangeState::Failed);

        // 멈추면 기다리던 카드는 거절한 것이 되고, 도구는 답할 곳이 사라진 것을 안다.
        let (reply, answer) = oneshot::channel();
        chat.apply(ChatEvent::Approve(Approval {
            card: card(2),
            reply,
        }));
        chat.stop();
        assert_eq!(state_of(&chat, 2), ChangeState::Declined);
        assert!(futures::executor::block_on(answer).is_err());
    }

    #[test]
    fn stop_drops_empty_reply_only() {
        let mut chat = Conversation::default();
        chat.send("하나");
        chat.stop();
        assert_eq!(chat.turns(), [turn(true, "하나")]);
        // 답 없이 멈춘 말은 다음 말과 합쳐 보낸다.
        assert_eq!(chat.send("둘"), Some(vec![turn(true, "하나\n\n둘")]));
        chat.apply(ChatEvent::Text("받은 데까지".into()));
        chat.stop();
        assert_eq!(chat.turns().last(), Some(&turn(false, "받은 데까지")));
        assert!(!chat.is_streaming());
    }
}
