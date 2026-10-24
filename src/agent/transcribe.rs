//! 이미지의 글을 모델(LLM 비전)로 받아 적는다(OCR 텍스트 삽입, ADR-0053).
//!
//! 대화 맥락·도구 없이 이미지 한 장과 지시만 보내는 한 번의 요청이다. 기기의 문자 인식(Vision)이 한자를
//! 섞은 글 등을 못 읽을 때 쓴다. 시스템 프롬프트는 원문을 고치지 않고 그대로 옮기는 데 맞춘다.
//! 답은 받는 대로 조각(`ChatEvent::Text`)으로 넘기고, 마지막은 `Done` 또는 `Failed`다.

use futures::StreamExt as _;
use futures::channel::mpsc;
use rig_core::completion::CompletionRequest;
use rig_core::message::{DocumentSourceKind, Image, ImageMediaType, Message, UserContent};
use rig_core::streaming::{Item, StreamEvent};

use super::bedrock::{completion_model, max_tokens, runtime};
use super::{Backend, ChatEvent};

/// 받아 적기의 시스템 프롬프트. 원문 왜곡(번역·교정·표기 바꾸기·요약·지어내기)을 막는 데 집중한다.
const SYSTEM: &str = "\
You are a transcription engine. You receive one image (a scanned page, a photo of a document, or a screenshot) \
and you output the text that is printed or written in it, exactly as it appears.

Fidelity rules. These are strict:
- Copy the text character for character. Do not translate, summarize, paraphrase, explain, or comment.
- Never correct anything: keep typos, archaic or nonstandard spelling, grammar, spacing, and punctuation as printed.
- Never change scripts. Keep Hanja/Kanji/Chinese characters as those characters; do not convert them to Hangul \
or kana, and do not convert Hangul to Hanja. Keep traditional and simplified forms as printed. Keep mixed-script \
text (for example Korean mixed with Hanja) mixed exactly as printed.
- Do not add anything that is not in the image: no titles, labels, notes, readings, or translations.
- If a character is illegible, write [?] in its place instead of guessing a different word. Do not invent text \
to fill gaps.
- Include all text in the image, including headings, captions, footnotes, page numbers, and running headers, \
unless the user's instructions say otherwise.

Layout rules:
- Follow the natural reading order of the page (columns in order; vertical text column by column in its own \
direction).
- Write one paragraph per line, and separate paragraphs with a single newline. Join the lines that only wrap \
inside a paragraph into one line, with a space where the original has a word break. Remove a hyphen that only \
splits a word at a line end. When a Korean, Chinese, or Japanese line wraps in the middle of a word, join it \
without adding a space.
- Output plain text only. Do not use Markdown formatting, code fences, or quotation marks around the output, and \
do not write anything before or after the transcription.
- If the image contains no text, output nothing.";

/// 줄바꿈을 그대로 둘 때 덧붙이는 지시. 위 배치 규칙의 문단 잇기를 대신한다.
const KEEP_BREAKS: &str = "Keep the line breaks of the image: write each printed line on its own line, exactly \
where it breaks in the image, and do not join wrapped lines.";

/// 기기 문자 인식 초안의 안내.
const DRAFT_NOTE: &str = "Below is a draft from an on-device OCR engine. It is often wrong: it may drop lines, \
garble words, and especially drop Hanja/Chinese characters or replace them with unrelated Hangul. Use it only as \
a hint for the reading order and to check that no line is missed. The image is the only authority: never copy \
anything from the draft that does not match the image.";

/// 받아 적기 요청.
#[derive(Clone, Debug)]
pub struct TranscribeRequest {
    /// 부를 곳과 모델.
    pub backend: Backend,
    /// 이미지(PNG 또는 JPEG).
    pub image: Vec<u8>,
    pub jpeg: bool,
    /// 기기 문자 인식 결과. 보내지 않으면 `None`.
    pub draft: Option<String>,
    /// 사용자가 덧붙인 지시. 비었으면 없다.
    pub instruction: String,
    /// 이미지의 줄바꿈을 그대로 둔다.
    pub keep_breaks: bool,
}

/// 모델에 보낼 사용자 말(이미지 앞의 글).
fn prompt(request: &TranscribeRequest) -> String {
    let mut text = String::from("Transcribe the text in this image.");
    if request.keep_breaks {
        text.push_str("\n\n");
        text.push_str(KEEP_BREAKS);
    }
    let instruction = request.instruction.trim();
    if !instruction.is_empty() {
        text.push_str(
            "\n\nThe user's additional instructions follow. Apply them; they take precedence over the layout \
             rules, but still never invent text that is not in the image.\n<instructions>\n",
        );
        text.push_str(instruction);
        text.push_str("\n</instructions>");
    }
    if let Some(draft) = request.draft.as_deref().filter(|d| !d.trim().is_empty()) {
        text.push_str("\n\n");
        text.push_str(DRAFT_NOTE);
        text.push_str("\n<ocr_draft>\n");
        text.push_str(draft.trim());
        text.push_str("\n</ocr_draft>");
    }
    text
}

/// 받아 적기를 보내고, 답 조각을 받을 채널을 돌려준다. 받는 쪽이 채널을 버리면 다음 조각에서 멈춘다.
pub fn transcribe(request: TranscribeRequest) -> mpsc::UnboundedReceiver<ChatEvent> {
    let (tx, rx) = mpsc::unbounded();
    runtime().spawn(async move {
        let last = match run(request, &tx).await {
            Ok(()) => ChatEvent::Done,
            Err(err) => ChatEvent::Failed(err),
        };
        tx.unbounded_send(last).ok();
    });
    rx
}

async fn run(
    request: TranscribeRequest,
    tx: &mpsc::UnboundedSender<ChatEvent>,
) -> Result<(), String> {
    let model = completion_model(&request.backend);
    let text = prompt(&request);
    let image = Image {
        data: DocumentSourceKind::Raw(request.image),
        media_type: Some(if request.jpeg {
            ImageMediaType::JPEG
        } else {
            ImageMediaType::PNG
        }),
        ..Image::default()
    };
    let message = Message::User {
        content: vec![UserContent::Image(image), UserContent::text(text)],
    };
    let completion = CompletionRequest::new(message)
        .preamble(SYSTEM)
        .max_tokens(max_tokens(&request.backend));
    let mut stream = model.stream(completion).map_err(|err| err.to_string())?;
    while let Some(item) = stream.next().await {
        if let Item::Event(StreamEvent::Text { text, .. }) = item.map_err(|err| err.to_string())?
            && tx.unbounded_send(ChatEvent::Text(text)).is_err()
        {
            return Ok(());
        }
    }
    stream.finish().await.map_err(|err| err.to_string())?;
    Ok(())
}

/// 다 받은 답을 넣을 글로 다듬는다. 모델이 규칙을 어기고 코드 펜스로 감쌌으면 벗기고 앞뒤 공백을 지운다.
pub fn clean(text: &str) -> String {
    let text = text.trim();
    let unfenced = text
        .strip_prefix("```")
        .and_then(|rest| rest.split_once('\n'))
        .and_then(|(_, body)| body.trim_end().strip_suffix("```"));
    unfenced.unwrap_or(text).trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(draft: Option<&str>, instruction: &str, keep_breaks: bool) -> TranscribeRequest {
        TranscribeRequest {
            backend: Backend::TokenPlan {
                api_key: "sk-sp-test".into(),
                model: "qwen3.8-max".into(),
            },
            image: Vec::new(),
            jpeg: false,
            draft: draft.map(str::to_string),
            instruction: instruction.into(),
            keep_breaks,
        }
    }

    #[test]
    fn prompt_carries_options() {
        let plain = prompt(&request(None, "  ", false));
        assert_eq!(plain, "Transcribe the text in this image.");
        let full = prompt(&request(Some("初案"), "쪽 번호는 빼 줘", true));
        assert!(full.contains(KEEP_BREAKS));
        assert!(full.contains("<instructions>\n쪽 번호는 빼 줘\n</instructions>"));
        assert!(full.contains("<ocr_draft>\n初案\n</ocr_draft>"));
        assert!(!prompt(&request(Some(" \n"), "", false)).contains("ocr_draft"));
    }

    #[test]
    fn clean_strips_fences() {
        assert_eq!(clean("  吾等은 玆에\n"), "吾等은 玆에");
        assert_eq!(clean("```text\n吾等은\n玆에\n```"), "吾等은\n玆에");
        assert_eq!(clean("```\n吾等은\n```  "), "吾等은");
    }
}
