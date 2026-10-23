# ADR-0048: 어시스턴트 제공자에 Alibaba Token Plan을 더하고, OpenAI 호환 Chat 와이어로 같은 도구 루프를 돈다

- 상태: Proposed
- 날짜: 2026-10-23
- 관련: `src/agent/token_plan.rs`, `src/agent/bedrock.rs`, `src/settings.rs`

## 배경 (Context)

어시스턴트는 Amazon Bedrock만 부를 수 있었다. Alibaba Cloud Model Studio의 Token Plan은 구독으로 Qwen·DeepSeek·GLM·Kimi·MiniMax 모델을
부르게 해 준다. 주소는 OpenAI 호환 Chat Completions(`token-plan.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1`)이고,
인증은 Token Plan 전용 API 키(`sk-sp-…`)다. 중국 본토(`cn-beijing`)는 주소도 키도 따로다. pi(earendil-works/pi)의
`qwen-token-plan` 제공자가 같은 주소·모델 목록(models.dev `alibaba-token-plan`)을 쓴다.

## 결정 (Decision)

**`agent::Backend`를 `Bedrock`·`TokenPlan` 두 갈래로 나누고, 둘 다 rig의 `DynModel<Completion>`으로 지워 같은 도구 루프(`bedrock::run`)에 넣는다.
Token Plan은 rig의 OpenAI Chat 와이어를 Token Plan 주소로 돌린다.**

- 고를 수 있는 모델은 플랜의 모델 목록(`/models`)에서 그림·영상·음성 모델과 `auto`를 뺀 것이다. 목록에는 ID만 있어서 표시 이름·그림 입력·최대
  출력 토큰은 models.dev의 `alibaba-token-plan`에서 옮긴 표로 찾고, 표에 없으면 ID와 기본값(그림은 넣어 보고 거절하면 뺀다, 출력 64K)을 쓴다.
  기본은 `qwen3.8-max`이고, 고른 모델이나 기본이 목록에 없으면 목록의 첫 모델을 고른다.
- 어시스턴트는 `set_model` 도구로 모델 목록을 보고 저장된 모델을 바꿀 수 있다(Bedrock도 같다). 바꾼 모델은 다음 말과 새 대화부터 쓴다.
- 국제(싱가포르) 주소만 쓴다. 중국 본토 주소는 고를 수 없다.
- API 키는 다른 AI 설정처럼 `config.toml`(`token_plan_api_key`)에 둔다.
- 고른 모델은 제공자마다 따로 저장한다(`ai_model`, `token_plan_model`).
- 생각(`enable_thinking`)은 따로 켜지 않고 모델 기본값을 따른다. 생각 글은 rig가 받은 필드(`reasoning_content`)로 대화에 되돌린다.

## 결과 (Consequences)

- (+): Bedrock과 같은 도구·승인 카드·검토가 Token Plan 모델에서도 돈다. 새 HTTP 클라이언트 코드가 없다.
- (+): 플랜에 새 모델이 생기면 표를 고치지 않아도 고를 수 있다.
- (-): 대화용이 아닌 모델은 이름으로 거른다. 새 종류의 모델이 생기면 거르는 말을 더해야 한다.
- (-): API 키가 설정 파일에 평문으로 남는다.

## 대안 (Alternatives)

| 대안 | 기각 사유 |
| --- | --- |
| API 키를 OS 키체인에 두기 | 다른 설정과 따로 놀고, 개발 빌드마다 접근 허락을 다시 묻는다 |
| Anthropic 호환 주소로 부르기 | Token Plan 문서·pi 모두 OpenAI 호환 주소를 쓴다. 모델 대부분이 Claude가 아니다 |
| 고정 목록만 쓰기(pi) | 플랜에 없는 모델도 보여서, 고르면 보낼 때 실패한다 |
| 국제·중국 지역 고르기 | 쓰는 곳은 국제뿐이다. 필요해지면 주소와 키를 지역별로 나눈다 |
