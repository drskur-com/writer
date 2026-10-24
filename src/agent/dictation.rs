//! 받아쓰기 기록을 모델로 요약(ADR-0055).
//!
//! 대화 맥락·도구 없이 지시와 글만 보내는 한 번의 요청이다(OCR의 [`super::transcribe`]와 같은 틀).
//! 시스템 프롬프트를 요청마다 앞에 두어 제공자의 프롬프트 캐시가 들기 쉽게 한다. 화면이 [`summarize`]로 부른다.

use futures::StreamExt as _;
use futures::channel::oneshot;
use rig_core::completion::CompletionRequest;
use rig_core::message::Message;
use rig_core::streaming::{Item, StreamEvent};

use super::Backend;
use super::bedrock::{completion_model, max_tokens, runtime};

/// 요약을 쓸 수 있는 언어(코드, 그 언어로 쓴 이름, 모델에 줄 영어 이름).
pub const LANGUAGES: &[Language] = &[
    Language::new("ko", "한국어", "Korean"),
    Language::new("en", "English", "English"),
    Language::new("ja", "日本語", "Japanese"),
    Language::new("zh-Hans", "简体中文", "Simplified Chinese"),
    Language::new("zh-Hant", "繁體中文", "Traditional Chinese"),
    Language::new("es", "Español", "Spanish"),
    Language::new("fr", "Français", "French"),
    Language::new("de", "Deutsch", "German"),
    Language::new("it", "Italiano", "Italian"),
    Language::new("pt", "Português", "Portuguese"),
    Language::new("ru", "Русский", "Russian"),
    Language::new("vi", "Tiếng Việt", "Vietnamese"),
    Language::new("th", "ไทย", "Thai"),
    Language::new("id", "Bahasa Indonesia", "Indonesian"),
    Language::new("ar", "العربية", "Arabic"),
    Language::new("hi", "हिन्दी", "Hindi"),
];

/// 요약 언어 하나.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Language {
    /// 설정에 저장하는 코드.
    pub code: &'static str,
    /// 고르기에 보이는 이름.
    pub native: &'static str,
    /// 모델에 주는 이름.
    pub english: &'static str,
}

impl Language {
    const fn new(code: &'static str, native: &'static str, english: &'static str) -> Self {
        Self {
            code,
            native,
            english,
        }
    }

    pub fn find(code: &str) -> Option<&'static Language> {
        LANGUAGES.iter().find(|language| language.code == code)
    }
}

// ---- 요약 ----

const SUMMARY_SYSTEM: &str = "\
You keep a running summary of a lecture, meeting, or conversation while it is being recorded. The input is a \
speech-to-text transcript without speaker labels; it may contain filler words and misheard words, and the \
recognizer sometimes writes a stretch of speech in the wrong language or script.

You receive the current summary (possibly empty) and the transcript recorded since then. Return the updated \
summary of everything so far.

Rules:
- Group the summary by topic, in the order the topics came up. Give each topic a short title, then concise \
bullet points under it. Keep key facts, numbers, names, decisions, questions, and action items. Leave out \
small talk.
- Merge the new transcript into the existing summary: keep earlier topics and points, update them when the \
new part changes them, add new points under the topic they belong to, and start a new topic when the \
subject changes. Do not drop earlier topics.
- Do not invent anything that was not said. When a word looks misheard, use the meaning from the context. \
Skip fragments that make no sense in context (such as a few words in an unrelated language).
- Format: each topic title on its own line starting with \"### \", followed by its points as Markdown \
bullets (\"- \"), one bullet per line. No other heading levels, no nested lists, no blank lines, and nothing \
before or after the summary.

Example:
### Topic title
- Point
- Point
### Next topic title
- Point";

/// 요약 요청.
#[derive(Clone, Debug)]
pub struct SummaryRequest {
    pub backend: Backend,
    /// 요약을 쓸 언어. 없으면 기록의 언어.
    pub language: Option<&'static Language>,
    /// 지금까지의 요약.
    pub previous: String,
    /// 그 뒤로 받아 적은 글.
    pub transcript: String,
}

fn summary_prompt(request: &SummaryRequest) -> String {
    let language = request
        .language
        .map_or("the language of the transcript", |language| {
            language.english
        });
    let mut text = format!("Write the summary in {language}.\n\n<summary>\n");
    text.push_str(request.previous.trim());
    text.push_str("\n</summary>\n\n<new_transcript>\n");
    text.push_str(request.transcript.trim());
    text.push_str("\n</new_transcript>");
    text
}

/// 요약을 보내고 고친 요약을 받을 채널을 돌려준다.
pub fn summarize(request: SummaryRequest) -> oneshot::Receiver<Result<String, String>> {
    spawn(async move {
        let answer = complete(&request.backend, SUMMARY_SYSTEM, summary_prompt(&request)).await?;
        Ok(tidy_summary(&unfence(&answer)))
    })
}

/// 요약을 주제 제목(`### `)과 글머리표(`- `) 줄로 맞춘다. 모델이 다른 수준의 제목을 썼으면 `### `로 바꾸고,
/// 빈 줄은 뺀다.
fn tidy_summary(text: &str) -> String {
    text.lines()
        .map(str::trim_end)
        .filter(|line| !line.trim().is_empty())
        .map(|line| match line.trim_start().strip_prefix('#') {
            Some(title) => format!("### {}", title.trim_start_matches('#').trim()),
            None => line.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ---- 공통 ----

/// 모델이 규칙을 어기고 코드 펜스로 감쌌으면 벗기고 앞뒤 공백을 지운다.
fn unfence(text: &str) -> String {
    let text = text.trim();
    text.strip_prefix("```")
        .and_then(|rest| rest.split_once('\n'))
        .and_then(|(_, body)| body.trim_end().strip_suffix("```"))
        .unwrap_or(text)
        .trim()
        .to_string()
}

/// tokio 런타임에서 돌리고 결과를 받을 채널을 돌려준다.
fn spawn<T: Send + 'static>(
    work: impl Future<Output = Result<T, String>> + Send + 'static,
) -> oneshot::Receiver<Result<T, String>> {
    let (tx, rx) = oneshot::channel();
    runtime().spawn(async move {
        tx.send(work.await).ok();
    });
    rx
}

/// 지시와 글을 보내고 답 전체를 받는다.
async fn complete(backend: &Backend, system: &str, prompt: String) -> Result<String, String> {
    let model = completion_model(backend);
    let request = CompletionRequest::new(Message::user(prompt))
        .preamble(system)
        .max_tokens(max_tokens(backend));
    let mut stream = model.stream(request).map_err(|err| err.to_string())?;
    let mut answer = String::new();
    while let Some(item) = stream.next().await {
        if let Item::Event(StreamEvent::Text { text, .. }) = item.map_err(|err| err.to_string())? {
            answer.push_str(&text);
        }
    }
    stream.finish().await.map_err(|err| err.to_string())?;
    Ok(answer)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn backend() -> Backend {
        Backend::TokenPlan {
            api_key: "sk-sp-test".into(),
            model: "qwen3.8-max".into(),
        }
    }

    /// 실제 모델로 요약을 한 번 부른다(`bedrock::tests::live_backend`).
    /// `WRITER_TOKEN_PLAN_KEY=<키> cargo test -- --ignored live_dictation --nocapture`
    #[test]
    #[ignore]
    fn live_dictation() {
        let summary = runtime()
            .block_on(summarize(SummaryRequest {
                backend: super::super::bedrock::tests::live_backend(),
                language: Language::find("en"),
                previous: String::new(),
                transcript: "오늘 회의는 예산 얘기입니다.\n다음 주까지 견적을 받기로 했습니다."
                    .into(),
            }))
            .unwrap()
            .unwrap();
        println!("{summary}");
        assert!(summary.starts_with("### "));
        assert!(
            summary
                .lines()
                .all(|line| line.starts_with("### ") || line.starts_with("- "))
        );
    }

    #[test]
    fn summary_prompt_names_language_and_carries_transcript() {
        let request = |language| SummaryRequest {
            backend: backend(),
            language,
            previous: String::new(),
            transcript: "말".into(),
        };
        let summary = summary_prompt(&request(None));
        assert!(summary.contains("the language of the transcript"));
        assert!(summary.contains("<new_transcript>\n말\n</new_transcript>"));
        assert!(
            summary_prompt(&request(Language::find("ko")))
                .starts_with("Write the summary in Korean.")
        );
    }

    #[test]
    fn tidies_summary_into_titles_and_bullets() {
        assert_eq!(
            tidy_summary("## 예산\n\n- 견적은 다음 주\n#### 일정 \n- 금요일 마감"),
            "### 예산\n- 견적은 다음 주\n### 일정\n- 금요일 마감"
        );
    }
}
