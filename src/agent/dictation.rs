//! 받아쓰기 기록을 모델로 번역·다듬기·요약(ADR-0055).
//!
//! 모두 대화 맥락·도구 없이 지시와 글만 보내는 한 번의 요청이다(OCR의 [`super::transcribe`]와 같은 틀).
//! 문단은 번호를 붙인 태그(`<p n="1">…</p>`)로 보내고 같은 번호의 태그(번역 `<tr>`, 다듬기 `<fix>`)로 받아,
//! 개수가 어긋나면 실패로 본다.
//! 시스템 프롬프트·용어집처럼 요청마다 같은 것을 앞에 두어 제공자의 프롬프트 캐시가 들기 쉽게 한다.
//!
//! 번역·다듬기는 녹음 쓰기 스레드(gpui 밖)가 [`translate_blocking`]으로, 요약은 화면이 [`summarize`]로 부른다.

use std::sync::mpsc as std_mpsc;
use std::time::Duration;

use futures::StreamExt as _;
use futures::channel::oneshot;
use rig_core::completion::CompletionRequest;
use rig_core::message::Message;
use rig_core::streaming::{Item, StreamEvent};

use super::Backend;
use super::bedrock::{completion_model, max_tokens, runtime};

/// 번역할 수 있는 언어(코드, 그 언어로 쓴 이름, 모델에 줄 영어 이름).
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

/// 번역 언어 하나.
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

/// 용어집에 둘 최대 항목 수. 넘으면 오래된 것부터 버린다.
const MAX_TERMS: usize = 60;

/// 고유명사·전문 용어와 그 번역(또는 바른 표기). 요청마다 함께 보내 앞에서 정한 것을 이어 쓴다.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Glossary(Vec<(String, String)>);

impl Glossary {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// 새 항목을 더한다. 같은 원어는 새 것으로 바꾼다.
    pub fn merge(&mut self, terms: Vec<(String, String)>) {
        for (source, target) in terms {
            self.0.retain(|(known, _)| *known != source);
            self.0.push((source, target));
        }
        let over = self.0.len().saturating_sub(MAX_TERMS);
        self.0.drain(..over);
    }

    fn render(&self) -> String {
        self.0
            .iter()
            .map(|(source, target)| format!("{source} => {target}\n"))
            .collect()
    }
}

// ---- 번역·다듬기 ----

const TRANSLATE_SYSTEM: &str = "\
You work live on a speech-to-text transcript of a lecture, meeting, or conversation. The transcript comes \
from an on-device speech recognizer, so it has no speaker labels, contains filler words, repetitions, missing \
or wrong punctuation and spacing, and sometimes misheard words (similar-sounding words, garbled proper nouns).

The reader's language is the target language. You receive numbered paragraphs and handle each one by its \
language:
- A paragraph in another language: translate it into the target language, naturally, as a fluent \
interpreter would. Drop fillers and false starts. When a word was clearly misheard, translate what the \
speaker meant, judging from the context. Output <tr n=\"N\">translation</tr>.
- A paragraph already in the target language: clean it up in that language. Remove fillers (such as \"um\", \
\"uh\", \"음\", \"어\", \"그\") and accidental repetitions and false starts, fix spacing and punctuation, and \
fix words that were clearly misheard, judging from the context and the glossary. When you are not sure, keep \
the original words. Keep the speaker's wording, meaning, and tone; do not paraphrase. Output \
<fix n=\"N\">cleaned paragraph</fix>.
- A paragraph that mixes languages counts as the language most of it is in.

Rules:
- Do not summarize, merge, split, reorder, explain, or add anything that was not said. Every input paragraph \
gets exactly one output tag with the same number.
- Keep each output paragraph on a single line, and never put one tag inside another.
- Continue naturally from the previous context. Keep terminology, names, tone, and register consistent with \
the glossary and the earlier output.
- Output only the tags described below, nothing before or after.

Output format:
<tr n=\"1\">translation of paragraph 1</tr>
<fix n=\"2\">cleaned paragraph 2</fix>
<terms>
source term => term in the target language
</terms>

In <terms>, list only proper nouns and technical terms that appear in this batch and are not yet in the \
glossary, with their rendering (or correct spelling) in the target language, one per line. Leave \
<terms></terms> empty when there are none.";

/// 번역·다듬기 요청 하나(문단 몇 개).
#[derive(Clone, Debug)]
pub struct TranslateRequest {
    pub backend: Backend,
    /// 사용 언어. 다른 언어로 말한 문단은 이 언어로 옮기고, 이 언어로 말한 문단은 다듬는다.
    pub language: &'static Language,
    /// 앞 문단(원문, 기록에 쓴 글).
    pub context: Vec<(String, String)>,
    pub glossary: Glossary,
    pub paragraphs: Vec<String>,
}

/// 문단 하나를 어떻게 쓸지.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rendered {
    /// 다른 언어였다: 번역문(원문은 아래에 붙인다).
    Translated(String),
    /// 사용 언어였다: 다듬은 글(원문 대신 쓴다).
    Cleaned(String),
    /// 그대로 둔다(모델이 비워 두었거나 형식이 깨졌다).
    Unchanged,
}

/// 번역·다듬기 결과.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Translation {
    pub paragraphs: Vec<Rendered>,
    /// 새로 나온 용어.
    pub terms: Vec<(String, String)>,
}

fn translate_prompt(request: &TranslateRequest) -> String {
    let mut text = format!("Target language: {}\n\n", request.language.english);
    push_glossary(&mut text, &request.glossary);
    if !request.context.is_empty() {
        text.push_str("Previous context (already done; do not output it again):\n<context>\n");
        for (source, output) in &request.context {
            text.push_str(&format!(
                "<source>{source}</source>\n<output>{output}</output>\n"
            ));
        }
        text.push_str("</context>\n\n");
    }
    text.push_str("Handle these paragraphs:\n");
    push_paragraphs(&mut text, &request.paragraphs);
    text
}

/// 번역·다듬기를 보내고 `timeout` 안에 답을 기다린다(쓰기 스레드에서 부른다). 문단 개수가 어긋나면 한 번
/// 다시 보낸다.
pub fn translate_blocking(
    request: &TranslateRequest,
    timeout: Duration,
) -> Result<Translation, String> {
    let mut last = String::new();
    for _ in 0..2 {
        let answer = complete_blocking(
            &request.backend,
            TRANSLATE_SYSTEM,
            translate_prompt(request),
            timeout,
        )?;
        match parse_paragraphs(&answer, request.paragraphs.len()) {
            Ok(paragraphs) => {
                return Ok(Translation {
                    paragraphs,
                    terms: parse_terms(&answer),
                });
            }
            Err(err) => {
                eprintln!("번역 답을 읽지 못했습니다({err}):\n{answer}");
                last = err;
            }
        }
    }
    Err(last)
}

// ---- 요약 ----

const SUMMARY_SYSTEM: &str = "\
You keep a running summary of a lecture, meeting, or conversation while it is being recorded. The input is a \
speech-to-text transcript without speaker labels; it may contain filler words and misheard words.

You receive the current summary (possibly empty) and the transcript recorded since then. Return the updated \
summary of everything so far.

Rules:
- Write concise bullet points grouped by topic, in the order the topics came up. Keep key facts, numbers, \
names, decisions, questions, and action items. Leave out small talk.
- Merge the new transcript into the existing summary: keep earlier points, update them when the new part \
changes them, and add new points. Do not drop earlier topics.
- Do not invent anything that was not said. When a word looks misheard, use the meaning from the context.
- Use Markdown bullets (\"- \") only: one bullet per line, no headings, no nested lists, no blank lines, and \
nothing before or after the summary.";

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
        Ok(unfence(&answer))
    })
}

// ---- 공통 ----

fn push_glossary(text: &mut String, glossary: &Glossary) {
    if !glossary.is_empty() {
        text.push_str("Glossary (use these renderings consistently):\n<glossary>\n");
        text.push_str(&glossary.render());
        text.push_str("</glossary>\n\n");
    }
}

fn push_paragraphs(text: &mut String, paragraphs: &[String]) {
    for (ix, paragraph) in paragraphs.iter().enumerate() {
        text.push_str(&format!("<p n=\"{}\">{}</p>\n", ix + 1, paragraph.trim()));
    }
}

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

/// [`complete`]를 gpui·tokio 밖의 스레드에서 `timeout`까지 기다린다. 넘으면 요청을 버린다.
fn complete_blocking(
    backend: &Backend,
    system: &'static str,
    prompt: String,
    timeout: Duration,
) -> Result<String, String> {
    let (tx, rx) = std_mpsc::channel();
    let backend = backend.clone();
    let task = runtime().spawn(async move {
        tx.send(complete(&backend, system, prompt).await).ok();
    });
    match rx.recv_timeout(timeout) {
        Ok(result) => result,
        Err(_) => {
            task.abort();
            Err(format!("no answer in {} s", timeout.as_secs()))
        }
    }
}

/// 답의 문단 태그: 여는 꼴, 닫는 태그, 번역인지(아니면 다듬기). `<p>`·`<same>`은 예전 꼴이라 함께 받는다.
const TAGS: [(&str, &str, bool); 4] = [
    ("<tr n=\"", "</tr>", true),
    ("<fix n=\"", "</fix>", false),
    ("<p n=\"", "</p>", true),
    ("<same n=\"", "</same>", false),
];

/// 답에서 번호 붙은 문단(`<tr n="N">…</tr>`·`<fix n="N">…</fix>`)을 `count`개 읽는다.
/// 번호가 1..=count를 꼭 한 번씩 덮어야 한다. 문단 안의 줄바꿈은 공백으로 잇는다(줄 하나가 문단).
///
/// 모델이 형식을 조금 어겨도 읽는다: 닫는 태그가 빠지면 다음 태그나 끝까지를 문단으로 보고, 스스로 닫은 태그
/// (`<fix n="N"/>`)나 빈 태그는 그대로 두며, 태그를 다른 태그로 감쌌으면 안쪽을 읽는다.
fn parse_paragraphs(answer: &str, count: usize) -> Result<Vec<Rendered>, String> {
    /// `text`에서 가장 먼저 나오는 문단 태그(위치, 태그).
    fn next_open(text: &str) -> Option<(usize, (&'static str, &'static str, bool))> {
        TAGS.iter()
            .filter_map(|tag| text.find(tag.0).map(|at| (at, *tag)))
            .min_by_key(|(at, _)| *at)
    }
    let mut found: Vec<Option<Rendered>> = vec![None; count];
    let mut rest = answer;
    while let Some((start, (open_tag, close_tag, translated))) = next_open(rest) {
        let after = &rest[start + open_tag.len()..];
        let quote = after.find('"').ok_or("unterminated paragraph number")?;
        let n: usize = after[..quote]
            .trim()
            .parse()
            .map_err(|_| "bad paragraph number".to_string())?;
        let after = &after[quote + 1..];
        let open = after.find('>').ok_or("unterminated tag")?;
        let self_closing = after[..open].trim_end().ends_with('/');
        let body = &after[open + 1..];
        let value = if self_closing {
            rest = body;
            Rendered::Unchanged
        } else if next_open(body).is_some_and(|(at, _)| body[..at].trim().is_empty()) {
            // 태그를 다른 태그로 감쌌다: 안쪽 태그를 읽는다(바깥 닫는 태그는 건너뛰어진다).
            rest = body;
            continue;
        } else {
            let end = [next_open(body).map(|(at, _)| at), body.find("<terms>")]
                .into_iter()
                .flatten()
                .min()
                .unwrap_or(body.len());
            let (text, next) = match body[..end].find(close_tag) {
                Some(close) => (&body[..close], &body[close + close_tag.len()..]),
                None => (&body[..end], &body[end..]),
            };
            rest = next;
            let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
            match (text.is_empty(), translated) {
                (true, _) => Rendered::Unchanged,
                (false, true) => Rendered::Translated(text),
                (false, false) => Rendered::Cleaned(text),
            }
        };
        let slot = n
            .checked_sub(1)
            .and_then(|ix| found.get_mut(ix))
            .ok_or_else(|| format!("unexpected paragraph {n}"))?;
        if slot.is_some() {
            return Err(format!("paragraph {n} twice"));
        }
        *slot = Some(value);
    }
    found
        .into_iter()
        .enumerate()
        .map(|(ix, value)| value.ok_or_else(|| format!("paragraph {} missing", ix + 1)))
        .collect()
}

/// 답의 `<terms>` 안에서 `원어 => 번역` 줄을 읽는다.
fn parse_terms(answer: &str) -> Vec<(String, String)> {
    let Some(start) = answer.find("<terms>") else {
        return Vec::new();
    };
    let body = &answer[start + 7..];
    let body = &body[..body.find("</terms>").unwrap_or(body.len())];
    body.lines()
        .filter_map(|line| {
            let (source, target) = line.split_once("=>")?;
            let (source, target) = (source.trim(), target.trim());
            (!source.is_empty() && !target.is_empty())
                .then(|| (source.to_string(), target.to_string()))
        })
        .collect()
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

    #[test]
    fn parses_translated_and_cleaned_paragraphs() {
        use Rendered::*;
        let answer = "<tr n=\"2\">Second\n line</tr>\n<fix n=\"1\">첫 문단.</fix>\n<tr n=\"3\"> Third </tr>\n\
                      <terms>\n삼성 => Samsung\nbad line\n => x\n</terms>";
        assert_eq!(
            parse_paragraphs(answer, 3).unwrap(),
            vec![
                Cleaned("첫 문단.".into()),
                Translated("Second line".into()),
                Translated("Third".into())
            ]
        );
        assert_eq!(parse_terms(answer), vec![("삼성".into(), "Samsung".into())]);
        assert!(parse_paragraphs(answer, 4).is_err());
        assert!(parse_paragraphs(answer, 2).is_err());
        assert!(parse_paragraphs("<tr n=\"1\">a</tr><fix n=\"1\">b</fix>", 1).is_err());
        // 감싼 태그는 안쪽을, 예전 꼴(<p>·<same>)도 읽는다.
        assert_eq!(
            parse_paragraphs(
                "<p n=\"1\"><same n=\"1\"/></p>\n<tr n=\"2\"> <fix n=\"2\">고침</fix></tr>\n<p n=\"3\">Old</p>",
                3
            )
            .unwrap(),
            vec![Unchanged, Cleaned("고침".into()), Translated("Old".into())]
        );
        // 형식을 조금 어긴 답: 닫는 태그 빠짐, 스스로 닫은 태그, 빈 태그.
        let loose = "<tr n=\"1\">One\n<fix n=\"2\"/>\n<fix n=\"3\"></fix>\n<tr n=\"4\">Four\n<terms>\nA => B\n</terms>";
        assert_eq!(
            parse_paragraphs(loose, 4).unwrap(),
            vec![
                Translated("One".into()),
                Unchanged,
                Unchanged,
                Translated("Four".into())
            ]
        );
        assert_eq!(parse_terms(loose), vec![("A".into(), "B".into())]);
        assert!(parse_terms("no terms").is_empty());
    }

    #[test]
    fn glossary_replaces_and_caps() {
        let mut glossary = Glossary::default();
        glossary.merge(vec![("a".into(), "1".into()), ("b".into(), "2".into())]);
        glossary.merge(vec![("a".into(), "3".into())]);
        assert_eq!(glossary.render(), "b => 2\na => 3\n");
        glossary.merge(
            (0..MAX_TERMS)
                .map(|ix| (ix.to_string(), "x".into()))
                .collect(),
        );
        assert_eq!(glossary.0.len(), MAX_TERMS);
        assert!(!glossary.render().contains("a => 3"));
    }

    /// 실제 모델로 번역·요약을 한 번씩 부른다(`bedrock::tests::live_backend`).
    /// `WRITER_TOKEN_PLAN_KEY=<키> cargo test -- --ignored live_dictation --nocapture`
    #[test]
    #[ignore]
    fn live_dictation() {
        let backend = super::super::bedrock::tests::live_backend();
        let translation = translate_blocking(
            &TranslateRequest {
                backend: backend.clone(),
                language: Language::find("en").unwrap(),
                context: vec![(
                    "오늘은 엔비디아 실적을 봅니다.".into(),
                    "Today we look at NVIDIA's earnings.".into(),
                )],
                glossary: Glossary::default(),
                paragraphs: vec![
                    "음 그러니까 지난 분기 매출이 어 크게 늘었습니다.".into(),
                    "um so this part is uh already in English".into(),
                    "데이터 센터 부문이 성장을 이끌었고요.".into(),
                ],
            },
            Duration::from_secs(60),
        )
        .unwrap();
        println!("{translation:#?}");
        assert!(matches!(
            translation.paragraphs.as_slice(),
            [
                Rendered::Translated(_),
                Rendered::Cleaned(_) | Rendered::Unchanged,
                Rendered::Translated(_)
            ]
        ));

        // 사용 언어가 한국어: 한국어 문단은 다듬고, 영어 문단은 번역한다.
        let korean = translate_blocking(
            &TranslateRequest {
                backend: backend.clone(),
                language: Language::find("ko").unwrap(),
                context: Vec::new(),
                glossary: Glossary::default(),
                paragraphs: vec![
                    "음 그러니까 오늘 회의는 어 예산 얘기 입니다".into(),
                    "Let's move on to the next topic.".into(),
                ],
            },
            Duration::from_secs(60),
        )
        .unwrap();
        println!("{korean:#?}");
        assert!(matches!(
            korean.paragraphs.as_slice(),
            [Rendered::Cleaned(_), Rendered::Translated(_)]
        ));

        let summary = runtime()
            .block_on(summarize(SummaryRequest {
                backend,
                language: None,
                previous: String::new(),
                transcript: "오늘 회의는 예산 얘기입니다.\n다음 주까지 견적을 받기로 했습니다."
                    .into(),
            }))
            .unwrap()
            .unwrap();
        println!("{summary}");
        assert!(summary.lines().all(|line| line.starts_with("- ")));
    }

    #[test]
    fn prompts_carry_context_glossary_and_numbers() {
        let mut glossary = Glossary::default();
        glossary.merge(vec![("엔비디아".into(), "NVIDIA".into())]);
        let request = TranslateRequest {
            backend: backend(),
            language: Language::find("en").unwrap(),
            context: vec![
                ("안녕하세요.".into(), "Hello.".into()),
                ("OK.".into(), "OK.".into()),
            ],
            glossary: glossary.clone(),
            paragraphs: vec![" 첫 문단 ".into(), "둘째".into()],
        };
        let text = translate_prompt(&request);
        assert!(text.starts_with("Target language: English"));
        assert!(text.contains("엔비디아 => NVIDIA"));
        assert!(text.contains("<source>안녕하세요.</source>\n<output>Hello.</output>"));
        assert!(text.contains("<p n=\"1\">첫 문단</p>\n<p n=\"2\">둘째</p>"));
        let summary = summary_prompt(&SummaryRequest {
            backend: backend(),
            language: None,
            previous: String::new(),
            transcript: "말".into(),
        });
        assert!(summary.contains("the language of the transcript"));
        assert!(summary.contains("<new_transcript>\n말\n</new_transcript>"));
    }
}
