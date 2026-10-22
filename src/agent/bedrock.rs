//! Amazon Bedrock(rig-bedrock, Converse API)에 대화를 보내고 답을 조각으로 받는다.
//!
//! rig·AWS SDK는 tokio 위에서 돈다. 앱(gpui) 실행기와 섞지 않도록 tokio 런타임을 따로 두고,
//! 답 조각은 채널로 화면에 넘긴다. 받는 쪽이 채널을 버리면 다음 조각에서 요청을 멈춘다.

use futures::StreamExt as _;
use futures::channel::mpsc;
use rig_bedrock::client::BedrockRuntime;
use rig_core::completion::CompletionRequest;
use rig_core::message::{Message, ToolResultContent};
use rig_core::streaming::{Item, StreamEvent};

use std::sync::atomic::{AtomicU64, Ordering};

use futures::channel::oneshot;

use super::tools::Planned;
use super::{Approval, Backend, ChangeCard, ChangeState, ChatEvent, Turn, Workspace};

/// 설정에서 모델을 고르지 않았을 때 쓰는 모델(Bedrock 전역 교차 리전 추론 프로필).
pub const DEFAULT_MODEL: &str = "global.anthropic.claude-opus-5-5";

/// 설정 창에서 고를 수 있는 Bedrock 모델.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BedrockModel {
    /// 표시 이름.
    pub name: &'static str,
    /// Bedrock 기본 모델 ID. 실제로 부를 때는 이 ID로 끝나는 추론 프로필
    /// (`global.…`, `us.…` 등)을 쓴다.
    pub base: &'static str,
}

/// 고를 수 있는 모델(설정 창 순서). 기본은 Claude Opus 5.5.
pub const BEDROCK_MODELS: &[BedrockModel] = &[
    BedrockModel {
        name: "GPT-6.1 Sol",
        base: "openai.gpt-6.1-sol",
    },
    BedrockModel {
        name: "Claude Sonnet 5.5",
        base: "anthropic.claude-sonnet-5-5",
    },
    BedrockModel {
        name: "Claude Opus 5.5",
        base: "anthropic.claude-opus-5-5",
    },
];

impl BedrockModel {
    /// 추론 프로필 ID(`global.openai.gpt-6.1-sol`)가 이 모델의 것인지.
    pub fn matches(&self, id: &str) -> bool {
        id == self.base
            || id
                .strip_suffix(self.base)
                .is_some_and(|prefix| prefix.ends_with('.'))
    }

    /// 리전에서 쓸 수 있는 추론 프로필 중 이 모델의 것. 어느 리전으로든 보낼 수 있는 `global.`을
    /// 먼저 고르고, 없으면 지역 프로필(`us.`, `eu.`, `apac.` 등)을 고른다.
    fn pick<'a>(&self, ids: &'a [String]) -> Option<&'a String> {
        let mut candidates = ids.iter().filter(|id| self.matches(id) && *id != self.base);
        let first = candidates.clone().next();
        candidates.find(|id| id.starts_with("global.")).or(first)
    }
}

/// 이 리전에서 쓸 수 있는 모델과 부를 ID([`BEDROCK_MODELS`] 순서). 추론 프로필 목록으로 판단한다.
pub fn available_models(
    profile: String,
    region: Option<String>,
) -> futures::channel::oneshot::Receiver<Result<Vec<(BedrockModel, String)>, String>> {
    let (tx, rx) = futures::channel::oneshot::channel();
    runtime().spawn(async move {
        tx.send(list_models(&profile, region).await).ok();
    });
    rx
}

async fn list_models(
    profile: &str,
    region: Option<String>,
) -> Result<Vec<(BedrockModel, String)>, String> {
    let mut loader =
        aws_config::defaults(aws_config::BehaviorVersion::latest()).profile_name(profile);
    if let Some(region) = region {
        loader = loader.region(aws_config::Region::new(region));
    }
    let client = aws_sdk_bedrock::Client::new(&loader.load().await);
    let mut pages = client
        .list_inference_profiles()
        .into_paginator()
        .items()
        .send();
    let mut ids = Vec::new();
    while let Some(summary) = pages.next().await {
        let summary =
            summary.map_err(|err| aws_sdk_bedrock::error::DisplayErrorContext(err).to_string())?;
        if *summary.status() == aws_sdk_bedrock::types::InferenceProfileStatus::Active {
            ids.push(summary.inference_profile_id().to_string());
        }
    }
    Ok(models_in(&ids))
}

/// 추론 프로필 ID 목록에서 고를 수 있는 모델과 부를 ID.
fn models_in(ids: &[String]) -> Vec<(BedrockModel, String)> {
    BEDROCK_MODELS
        .iter()
        .filter_map(|model| model.pick(ids).map(|id| (*model, id.clone())))
        .collect()
}

/// 한 번의 답에 쓸 수 있는 최대 출력 토큰. 스트리밍이라 넉넉히 둔다.
const MAX_TOKENS: u64 = 64_000;

/// Bedrock에 보낼 대화.
#[derive(Clone, Debug)]
pub struct ChatRequest {
    pub backend: Backend,
    /// 도구가 읽고 쓸 수 있는 폴더(라이브러리). 없으면 도구 없이 답만 받는다.
    pub workspace: Option<Workspace>,
    /// 시스템 프롬프트.
    pub system: String,
    /// 지금까지의 대화. 마지막은 이번에 보낼 사용자 말이다.
    pub turns: Vec<Turn>,
}

/// tokio 런타임. 처음 대화를 보낼 때 만든다.
fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("writer-ai")
            .enable_all()
            .build()
            .expect("tokio 런타임을 만들 수 없습니다")
    })
}

/// 대화를 보내고, 답 조각을 받을 채널을 돌려준다. 마지막 일은 언제나 `Done` 또는 `Failed`다.
pub fn stream(request: ChatRequest) -> mpsc::UnboundedReceiver<ChatEvent> {
    let (tx, rx) = mpsc::unbounded();
    runtime().spawn(async move {
        let last = match run(request, &tx).await {
            Ok(()) => ChatEvent::Done,
            Err(message) => ChatEvent::Failed(message),
        };
        tx.unbounded_send(last).ok();
    });
    rx
}

/// 사용자가 변경을 거절했을 때 모델에 돌려줄 글.
const DECLINED: &str = "The user declined this change. Do not retry it unless they ask; you may ask what they would like instead.";

/// 승인 카드 번호. 대화가 달라도 겹치지 않게 앱 전체에서 센다.
static NEXT_CHANGE: AtomicU64 = AtomicU64::new(1);

/// 도구를 부르고 다시 묻기를 이만큼까지만 되풀이한다(끝없이 도는 것을 막는다).
const MAX_TOOL_ROUNDS: usize = 24;

async fn run(request: ChatRequest, tx: &mpsc::UnboundedSender<ChatEvent>) -> Result<(), String> {
    if request.turns.is_empty() {
        return Err("보낼 말이 없습니다.".into());
    }
    let backend = &request.backend;
    let mut runtime = BedrockRuntime::builder().profile_name(&backend.profile);
    if let Some(region) = &backend.region {
        runtime = runtime.region(region);
    }
    let model = runtime.build().completion(&backend.model);
    let tools = request
        .workspace
        .as_ref()
        .map(Workspace::definitions)
        .unwrap_or_default();

    // 이번 답 동안 쌓이는 대화: 지난 말들 + 도구 호출과 그 결과.
    let mut messages: Vec<Message> = request
        .turns
        .iter()
        .map(|turn| {
            if turn.user {
                Message::user(&turn.text)
            } else {
                Message::assistant(&turn.text)
            }
        })
        .collect();

    for _ in 0..MAX_TOOL_ROUNDS {
        let Some((prompt, history)) = messages.split_last() else {
            break;
        };
        let completion = CompletionRequest::new(prompt.clone())
            .preamble(&request.system)
            .messages(history.iter().cloned())
            .tools(tools.iter().cloned())
            .max_tokens(MAX_TOKENS);

        let mut stream = model.stream(completion).map_err(|err| err.to_string())?;
        while let Some(item) = stream.next().await {
            // 글 조각만 화면에 넘긴다(도구 호출은 다 받은 뒤 응답에서 읽는다).
            if let Item::Event(StreamEvent::Text { text, .. }) =
                item.map_err(|err| err.to_string())?
                // 받는 쪽(화면)이 대화를 버렸으면 그만 받는다.
                && tx.unbounded_send(ChatEvent::Text(text)).is_err()
            {
                return Ok(());
            }
        }
        let response = stream.finish().await.map_err(|err| err.to_string())?;
        let calls: Vec<_> = response.tool_calls().cloned().collect();
        let (Some(workspace), false) = (&request.workspace, calls.is_empty()) else {
            return Ok(());
        };
        messages.extend(response.message());
        let mut results = Vec::with_capacity(calls.len());
        for call in &calls {
            let text = match workspace.plan(&call.function.name, &call.function.arguments) {
                Planned::Done(output) => {
                    if tx.unbounded_send(ChatEvent::Tool(output.summary)).is_err() {
                        return Ok(());
                    }
                    output.text
                }
                // 파일을 바꾸는 도구는 승인 카드를 띄우고 사용자의 답을 기다린다.
                Planned::Change(change) => {
                    let id = NEXT_CHANGE.fetch_add(1, Ordering::Relaxed);
                    let (reply, answer) = oneshot::channel();
                    let card = ChangeCard {
                        id,
                        path: change.path.clone(),
                        new_file: change.is_new(),
                        lines: change.diff(),
                        state: ChangeState::Pending,
                    };
                    if tx
                        .unbounded_send(ChatEvent::Approve(Approval { card, reply }))
                        .is_err()
                    {
                        return Ok(());
                    }
                    match answer.await {
                        Ok(true) => {
                            let output = workspace.commit(change);
                            if output.text.starts_with("Error:") {
                                tx.unbounded_send(ChatEvent::ChangeFailed(id)).ok();
                            }
                            output.text
                        }
                        Ok(false) => DECLINED.to_string(),
                        // 사용자가 답을 멈췄다.
                        Err(_) => return Ok(()),
                    }
                }
            };
            results.push(call.result(vec![ToolResultContent::text(text)]));
        }
        messages.push(Message::tool_results(results));
    }
    Err(format!("도구를 {MAX_TOOL_ROUNDS}번 넘게 불러 멈췄습니다."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_global_profile_first_and_skips_missing_models() {
        let ids: Vec<String> = [
            "us.anthropic.claude-opus-5-5",
            "global.anthropic.claude-opus-5-5",
            "eu.anthropic.claude-sonnet-5-5",
            "global.openai.gpt-6-sol",
        ]
        .map(String::from)
        .into();
        let models = models_in(&ids);
        let picked: Vec<_> = models.iter().map(|(m, id)| (m.name, id.as_str())).collect();
        assert_eq!(
            picked,
            [
                ("Claude Sonnet 5.5", "eu.anthropic.claude-sonnet-5-5"),
                ("Claude Opus 5.5", "global.anthropic.claude-opus-5-5"),
            ]
        );
        // gpt-6-sol 프로필은 gpt-6.1-sol이 아니다.
        assert!(!BEDROCK_MODELS[0].matches("global.openai.gpt-6-sol"));
    }

    /// 실제 Bedrock에서 리전의 모델 목록을 읽는다.
    /// `WRITER_AI_PROFILE=<프로필> WRITER_AI_REGION=<리전> cargo test -- --ignored live_models`
    #[test]
    #[ignore]
    fn live_models() {
        let profile = std::env::var("WRITER_AI_PROFILE").expect("WRITER_AI_PROFILE");
        let rx = available_models(profile, std::env::var("WRITER_AI_REGION").ok());
        let models = futures::executor::block_on(rx).expect("응답 없음");
        println!("models={models:?}");
        assert!(models.is_ok());
    }

    /// 실제 Bedrock을 부른다. `WRITER_AI_PROFILE=<프로필> cargo test -- --ignored live_stream`
    #[test]
    #[ignore]
    fn live_stream() {
        let profile = std::env::var("WRITER_AI_PROFILE").expect("WRITER_AI_PROFILE");
        let mut rx = stream(ChatRequest {
            backend: Backend {
                profile,
                region: std::env::var("WRITER_AI_REGION").ok(),
                model: std::env::var("WRITER_AI_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.into()),
            },
            workspace: None,
            system: "짧게 답하세요.".into(),
            turns: vec![Turn {
                user: true,
                text: "한 문장으로 인사해 줘.".into(),
                ..Turn::default()
            }],
        });
        let mut chunks = 0;
        let mut text = String::new();
        let last = futures::executor::block_on(async {
            loop {
                match rx.next().await {
                    Some(ChatEvent::Text(piece)) => {
                        chunks += 1;
                        text.push_str(&piece);
                    }
                    Some(other) => return Some(other),
                    None => return None,
                }
            }
        });
        println!("chunks={chunks} text={text:?} last={last:?}");
        assert!(matches!(last, Some(ChatEvent::Done)), "{last:?}");
        assert!(!text.is_empty());
    }
}
