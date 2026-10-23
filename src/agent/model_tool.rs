//! `set_model`: 어시스턴트가 부를 모델을 보고 바꾼다. 바꾼 모델은 설정(모델 고르기)에 저장돼 다음 말과
//! 새 대화부터 쓴다. 이번 답은 지금 모델로 끝낸다.

use serde_json::{Map, Value, json};

use super::{Backend, ModelChoice};

pub const SET_MODEL: &str = "set_model";

pub const DESCRIPTION: &str = "Show or change the AI model the assistant uses (the model picker under the input, also used \
by new conversations). Call it without model to list the available models and the current one. Change it only when the \
user asks; the new model answers from the user's next message on.";

pub fn parameters() -> Value {
    json!({
        "type": "object",
        "properties": {
            "model": {
                "type": "string",
                "description": "Model ID or name from the list. Omit to list the models.",
            },
        },
    })
}

/// 도구의 결과.
pub struct Outcome {
    /// 모델에 돌려줄 글.
    pub text: String,
    /// 바꿀 모델 ID(설정에 저장할 것). 목록만 보였거나 실패했으면 `None`.
    pub model: Option<String>,
}

/// `set_model`을 처리한다. 쓸 수 있는 모델 목록을 읽어, 모델을 주지 않았으면 목록을, 주었으면 그 모델로
/// 바꾼다.
pub async fn run(backend: &Backend, arguments: &Map<String, Value>) -> Outcome {
    let text = |text: String| Outcome { text, model: None };
    let models = match backend.list_models().await {
        Ok(models) => models,
        Err(err) => return text(format!("Error: the model list could not be read: {err}")),
    };
    let current = backend.model();
    let wanted = arguments
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|wanted| !wanted.is_empty());
    let Some(wanted) = wanted else {
        return text(listing(&models, current));
    };
    match find(&models, wanted) {
        Some((choice, id)) if choice.matches(current) => text(format!(
            "The assistant already uses {} ({id}).",
            choice.name
        )),
        Some((choice, id)) => Outcome {
            text: format!(
                "Changed the model to {} ({id}). It answers from the user's next message and in new \
                 conversations; this reply continues with the current model.",
                choice.name
            ),
            model: Some(id.clone()),
        },
        None => text(format!(
            "Error: {wanted} is not available. {}",
            listing(&models, current)
        )),
    }
}

/// 이름이나 ID로 고른 모델. ID(Bedrock은 기본 ID나 추론 프로필)가 먼저고, 이름은 대소문자를 가리지 않는다.
fn find<'a>(
    models: &'a [(ModelChoice, String)],
    wanted: &str,
) -> Option<&'a (ModelChoice, String)> {
    models
        .iter()
        .find(|(choice, id)| id == wanted || choice.matches(wanted))
        .or_else(|| {
            models
                .iter()
                .find(|(choice, _)| choice.name.eq_ignore_ascii_case(wanted))
        })
}

/// 모델에 보여 줄 목록.
fn listing(models: &[(ModelChoice, String)], current: &str) -> String {
    if models.is_empty() {
        return "No models are available.".into();
    }
    let lines: Vec<String> = models
        .iter()
        .map(|(choice, id)| {
            let mark = if choice.matches(current) {
                " (current)"
            } else {
                ""
            };
            format!("- {} — {id}{mark}", choice.name)
        })
        .collect();
    format!("Available models:\n{}", lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use super::*;

    fn models() -> Vec<(ModelChoice, String)> {
        [
            (
                "Claude Opus 5.5",
                "anthropic.claude-opus-5-5",
                "global.anthropic.claude-opus-5-5",
            ),
            ("GPT-6 Astra", "openai.gpt-6-astra", "us.openai.gpt-6-astra"),
        ]
        .map(|(name, base, id)| {
            let choice = ModelChoice {
                name: Cow::Borrowed(name),
                base: Cow::Borrowed(base),
            };
            (choice, id.to_string())
        })
        .into()
    }

    #[test]
    fn finds_models_by_id_base_or_name() {
        let models = models();
        let id = |wanted| find(&models, wanted).map(|(_, id)| id.as_str());
        assert_eq!(id("us.openai.gpt-6-astra"), Some("us.openai.gpt-6-astra"));
        assert_eq!(id("openai.gpt-6-astra"), Some("us.openai.gpt-6-astra"));
        assert_eq!(
            id("claude opus 5.5"),
            Some("global.anthropic.claude-opus-5-5")
        );
        assert_eq!(id("gpt-6.1-sol"), None);
        let list = listing(&models, "global.anthropic.claude-opus-5-5");
        assert!(list.contains("- Claude Opus 5.5 — global.anthropic.claude-opus-5-5 (current)"));
        assert!(
            list.contains("- GPT-6 Astra — us.openai.gpt-6-astra\n") || list.ends_with("astra")
        );
    }
}
