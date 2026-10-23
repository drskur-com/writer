//! Alibaba Cloud Model Studio의 Token Plan(구독, `sk-sp-` API 키)으로 모델을 부른다.
//!
//! Token Plan은 OpenAI 호환 Chat Completions라 rig의 OpenAI 와이어를 Token Plan 주소(국제)로 돌린다.
//! 고를 수 있는 모델은 플랜의 모델 목록(`/models`)에서 대화용이 아닌 것(그림·영상·음성)을 뺀 것이다.
//! 목록에는 ID만 있어서, 표시 이름·그림 입력·출력 상한은 아는 모델의 표([`TOKEN_PLAN_MODELS`],
//! models.dev의 `alibaba-token-plan`)에서 찾고, 표에 없으면 ID와 기본값을 쓴다.

use std::borrow::Cow;

use rig_core::providers::openai::OpenAIConfig;
use rig_core::providers::openai::wire::Dialect;

use super::ModelChoice;

/// 설정에서 모델을 고르지 않았을 때 쓰는 모델.
pub const DEFAULT_MODEL: &str = "qwen3.8-max";

/// 부를 주소(국제, 싱가포르 `ap-southeast-1`의 OpenAI 호환 모드).
const BASE_URL: &str = "https://token-plan.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1";

/// 아는 Token Plan 모델.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TokenPlanModel {
    /// 표시 이름.
    pub name: &'static str,
    /// 부를 모델 ID.
    pub id: &'static str,
    /// 그림을 볼 수 있는지.
    pub vision: bool,
    /// 한 번의 답에 쓸 수 있는 최대 출력 토큰. 넘겨 부르면 거절한다.
    pub max_output: u64,
}

/// 아는 모델(models.dev의 `alibaba-token-plan` 중 도구를 부를 수 있는 것). 고를 수 있는지는 플랜의 모델
/// 목록이 정하고, 이 표는 이름과 한계만 알려 준다.
pub const TOKEN_PLAN_MODELS: &[TokenPlanModel] = &[
    model("Qwen3.8 Max", "qwen3.8-max", true, 131_072),
    model("Qwen3.8 Flash", "qwen3.8-flash", true, 131_072),
    model("Qwen3.7 Max", "qwen3.7-max", false, 131_072),
    model("Qwen3.7 Plus", "qwen3.7-plus", true, 65_536),
    model("Qwen3.6 Plus", "qwen3.6-plus", true, 65_536),
    model("Qwen3.6 Flash", "qwen3.6-flash", true, 65_536),
    model("DeepSeek V4.1 Flash", "deepseek-v4.1-flash", true, 384_000),
    model("DeepSeek V4 Pro", "deepseek-v4-pro", false, 384_000),
    model(
        "DeepSeek V4 Pro 0813",
        "deepseek-v4-pro-0813",
        false,
        384_000,
    ),
    model("DeepSeek V4 Flash", "deepseek-v4-flash", false, 384_000),
    model(
        "DeepSeek V4 Flash 0731",
        "deepseek-v4-flash-0731",
        false,
        384_000,
    ),
    model("DeepSeek V3.2", "deepseek-v3.2", false, 65_536),
    model("GLM-5.3", "glm-5.3", false, 131_072),
    model("GLM-5.2", "glm-5.2", false, 131_072),
    model("GLM-5.1", "glm-5.1", false, 128_000),
    model("GLM-5", "glm-5", false, 16_384),
    model("Kimi K2.7 Code", "kimi-k2.7-code", true, 262_144),
    model("Kimi K2.6", "kimi-k2.6", true, 262_144),
    model("Kimi K2.5", "kimi-k2.5", true, 98_304),
    model("MiniMax-M2.5", "MiniMax-M2.5", false, 32_768),
];

const fn model(
    name: &'static str,
    id: &'static str,
    vision: bool,
    max_output: u64,
) -> TokenPlanModel {
    TokenPlanModel {
        name,
        id,
        vision,
        max_output,
    }
}

/// 모델 ID의 모델. 목록에 없으면 `None`.
pub fn find(id: &str) -> Option<&'static TokenPlanModel> {
    TOKEN_PLAN_MODELS.iter().find(|model| model.id == id)
}

/// 대화에 쓸 수 없는 모델(그림·영상·음성 생성과 인식, 자동 고르기). 플랜 목록에 함께 온다.
fn is_chat_model(id: &str) -> bool {
    let id = id.to_ascii_lowercase();
    id != "auto"
        && !id.starts_with("wan")
        && !["image", "video", "audio", "tts", "asr", "realtime"]
            .iter()
            .any(|word| id.contains(word))
}

/// 플랜에서 쓸 수 있는 대화 모델과 부를 ID(플랜 목록 순서).
pub(super) async fn list_models(api_key: &str) -> Result<Vec<(ModelChoice, String)>, String> {
    let list = OpenAIConfig::with_key(&DIALECT, api_key)
        .client()
        .list_models()
        .await
        .map_err(|err| err.to_string())?;
    Ok(choices(list.into_iter().map(|model| model.id)))
}

/// 플랜 목록의 ID에서 고를 수 있는 모델.
fn choices(ids: impl IntoIterator<Item = String>) -> Vec<(ModelChoice, String)> {
    ids.into_iter()
        .filter(|id| is_chat_model(id))
        .map(|id| {
            let name = find(&id).map_or_else(|| Cow::Owned(id.clone()), |model| model.name.into());
            let choice = ModelChoice {
                name,
                base: Cow::Owned(id.clone()),
            };
            (choice, id)
        })
        .collect()
}

/// Token Plan의 OpenAI 호환 방언. 요청 모양은 OpenAI Chat Completions 그대로다.
const DIALECT: Dialect =
    Dialect::gateway("alibaba-token-plan", BASE_URL, "ALIBABA_TOKEN_PLAN_API_KEY");

/// 이 키·모델로 부를 rig 모델.
pub fn completion_model(
    api_key: &str,
    model: &str,
) -> rig_core::driver::DynModel<rig_core::operation::Completion> {
    OpenAIConfig::with_key(&DIALECT, api_key)
        .client()
        .chat(model)
        .erase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_model_is_listed() {
        assert!(find(DEFAULT_MODEL).is_some());
        assert!(find("qwen3.8-max-preview").is_none());
    }

    #[test]
    fn lists_only_chat_models_with_known_names() {
        let ids = [
            "qwen3.8-max",
            "wan2.7-image",
            "qwen-audio-3.0-tts-plus",
            "deepseek-v4.1-flash-1017",
            "auto",
            "wan3.0-video",
            "qwen-image-3.0-pro",
            "qwen-audio-3.0-asr-flash",
        ];
        let models = choices(ids.map(String::from));
        let shown: Vec<_> = models
            .iter()
            .map(|(model, id)| (model.name.as_ref(), id.as_str()))
            .collect();
        assert_eq!(
            shown,
            [
                ("Qwen3.8 Max", "qwen3.8-max"),
                ("deepseek-v4.1-flash-1017", "deepseek-v4.1-flash-1017"),
            ]
        );
    }
}
