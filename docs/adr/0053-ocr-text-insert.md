# ADR-0053: 이미지의 글자는 운영체제의 인식기(macOS Vision, Windows.Media.Ocr)로 읽고(필요하면 LLM 비전으로), 고칠 수 있는 미리보기를 거쳐 커서 자리에 넣는다

- 상태: Proposed
- 날짜: 2026-10-24
- 관련: ADR-0011(줄 하나가 문단), ADR-0048(Token Plan) · `src/ocr.rs`, `src/ocr/text_recognizer.rs`, `src/agent/transcribe.rs`, `src/app/insert_ocr.rs`, `src/config.rs`(`[ocr]`), `src/menus.rs`

## 배경 (Context)

책·자료 사진이나 스크린샷의 글을 원고로 옮길 때 손으로 다시 친다. 삽입 메뉴에서 이미지를 골라 글자만 넣고 싶다.

- macOS에는 온디바이스 문자 인식이 있다. Vision `VNRecognizeTextRequest`(macOS 10.15+, 한국어는 13+)는 Objective-C API라 이미 쓰는 objc2 계열(`objc2-vision`)로 빌드 변경 없이 부른다.
- Windows에는 WinRT `Windows.Media.Ocr`(Windows 10+)가 있다. 이미 gpui가 쓰는 `windows` 크레이트(0.62)로 부르며 패키징이 필요 없다. 한국어를 읽으려면 Windows에 한국어 OCR 언어 팩이 있어야 한다.
- 더 새롭고 정확한 Windows AI `TextRecognizer`(Windows App SDK 2.5, `Microsoft.Windows.AI.Imaging`)는 WinRT라 C#·C++ 없이 winmd 바인딩으로 Rust에서 부를 수 있다. 다만 `systemAIModels` 권한이 든 패키지 ID(MSIX·sparse package), App SDK 런타임, Copilot+ PC(NPU)가 있어야 돈다. 지금 Windows 배포는 패키지 없는 exe다.
- 문단·표까지 읽는 `RecognizeDocumentsRequest`(macOS 26+)는 Swift 전용이라 Swift 빌드·링크를 끼워야 한다.
- 에디터는 줄 하나가 문단이다(ADR-0011). 인식한 줄을 그대로 넣으면 이미지의 줄바꿈마다 문단이 나뉜다.
- Vision은 언어를 여럿 주어도 맨 앞 언어 하나로 읽는다. 한글·한자 혼용문은 `ko-KR`이면 한자 줄을 버리거나 엉뚱한 한글로, `zh-Hant`·`ja-JP`면 한글을 깨뜨린다(시험: "吾等은 玆에…" → "쪽은 초에…"). PaddleOCR도 혼용문은 잘 읽지 못한다.

## 결정 (Decision)

**삽입 → OCR 텍스트…(macOS·Windows)로 대화상자를 연다.** 이미지를 끌어다 놓거나 눌러 고르면 백그라운드에서 읽고, 읽은 글을 고칠 수 있는 입력 칸에 미리 보여 준다. 삽입을 누르면 열린 시트의 커서 자리(선택이 있으면 선택 대신)에 넣는다.

- 인식(macOS): Vision 정확 모드, 언어 `ko-KR`·`en-US`, 언어 보정 켬.
- 인식(Windows, 1순위): Windows AI `TextRecognizer`. 바인딩은 NuGet `Microsoft.WindowsAppSDK.AI` 2.5.5의 winmd로 windows-bindgen 0.62.1이 만든 것을 저장소에 둔다(`src/ocr/text_recognizer/bindings.rs`, 다시 만드는 법은 파일 머리). 패키지 ID가 없는 exe는 실행 파일 옆의 `Microsoft.WindowsAppRuntime.Bootstrap.dll`을 동적으로 열어 `MddBootstrapInitialize2`(2.5, 최소 2.5.1.0, 패키지 ID가 있으면 아무것도 안 함)로 런타임을 붙인다. DLL이 없거나, 붙이지 못하거나, `GetReadyState`가 `Ready`가 아니거나, 읽다가 실패하면 조용히 다음 순위로 넘어간다. `NotReady`(모델 내려받기 필요)여도 동의 없이 내려받지 않는다.
- 인식(Windows, 2순위): `Windows.Media.Ocr`. 한국어 언어 팩이 있으면 한국어 엔진(라틴 문자도 읽는다), 없으면 사용자 프로필 언어 엔진. 하나도 없으면 언어 팩을 설치하라고 알린다. 이미지는 EXIF 방향대로 돌려 읽고, 엔진 한도(`MaxImageDimension`)를 넘으면 알린다. 줄 상자는 낱말 상자를 합쳐 만든다.
- 다른 이미지를 고르면 하던 인식은 버린다.
- 문단 묶기: 줄 사이 간격이 줄 높이의 0.8배보다 크거나, 다음 줄이 위로 올라가거나(다른 단), 줄이 글 영역보다 눈에 띄게 짧고 문장 부호로 끝나면 문단을 나누고, 아니면 공백으로 잇는다. 영어 줄 끝 하이픈은 지우고 붙인다. "줄바꿈 유지"를 켜면 줄마다 한 문단이다.
- 미리보기를 고친 뒤 "줄바꿈 유지"를 바꾸면 다시 만들어 고친 내용은 사라진다.
- 그 밖의 OS(Linux)에는 메뉴가 없다.

**"AI(LLM 비전)로 읽기"를 켜면 이미지를 설정의 AI 제공자 모델에 보내 받아 적게 한다.**

- 모델은 제공자의 쓸 수 있는 모델 중 이미지를 보는 것(Token Plan은 모델 표의 `vision`)에서 고른다. 고르지 않았으면 어시스턴트의 모델이다.
- 대화 맥락·도구 없이 이미지 한 장과 지시만 보내는 한 번의 요청이고, 답은 받는 대로 미리보기에 흘려 보인다. 그동안 Vision 결과는 미리보기에 보이지 않고(초안으로만 쓴다), AI가 실패하면 Vision 결과를 보인다.
- 시스템 프롬프트는 원문 왜곡을 막는 데 맞춘다: 글자 그대로 옮기고, 번역·요약·교정(오탈자·옛 표기·띄어쓰기·문장 부호)을 하지 않으며, 문자 체계를 바꾸지 않고(한자는 한자, 혼용은 혼용 그대로), 없는 글을 더하지 않고, 읽을 수 없는 글자는 `[?]`로 둔다. 문단 잇기·줄바꿈 유지는 Vision과 같은 뜻으로 지시한다.
- Vision 결과를 초안으로 함께 보낼 수 있다(기본 켬). 초안은 자주 틀린다고(특히 한자) 알리고 줄 순서·누락 확인에만 쓰며 이미지가 우선이라고 지시한다. 엉터리 초안을 보내도 Claude Opus 5.5·GPT-6 Astra 모두 혼용 시험 문장을 그대로 읽었다.
- 사용자의 추가 지시(예: 쪽 번호 빼기)는 배치 규칙보다 앞서지만, 없는 글을 지어내지는 않게 한다.
- 이미지는 EXIF 방향대로 돌려 긴 변 2048px PNG로 보낸다. 3 MB를 넘으면 JPEG(품질 90)로 바꾼다.
- 옵션(AI 사용, 제공자별 모델, 초안 보내기, 추가 지시, 줄바꿈 유지)은 `config.toml`의 `[ocr]`에 저장하고, 대화상자에 "다음에도 그대로 쓰인다"고 알린다. AI를 켜면 이미지가 어느 제공자로 전송되는지 보인다.

## 결과 (Consequences)

- 좋아지는 점(+): 기본은 네트워크·모델 파일·앱 크기 증가 없이 인쇄물·스크린샷의 글을 바로 옮긴다. 혼용문 등은 AI로 읽는다. 넣기 전에 오인식을 고친다.
- 감수하는 점(-): Linux에는 없다. Windows는 언어 팩에 따라 읽는 언어가 달라지고 정확도가 Vision보다 낮다. 기기 인식만 쓰면 혼용문을 못 읽고, 표·다단·손글씨는 구조를 잃거나 순서가 섞일 수 있으며, 음절 단위로 줄을 바꾼 한국어 조판은 줄을 이을 때 공백이 생긴다. AI로 읽으면 이미지가 외부 제공자로 나가고 비용이 들며, 프롬프트로 막아도 모델이 글을 바꿀 가능성은 남는다(미리보기에서 확인한다).
- 후속 작업(TODO): Windows 배포에 `systemAIModels` 권한이 든 패키지 ID(MSIX 또는 sparse package)와 `Microsoft.WindowsAppRuntime.Bootstrap.dll`(또는 App SDK 런타임 의존성)을 넣어야 `TextRecognizer`가 실제로 돈다. 그 전까지는 늘 `Windows.Media.Ocr`로 읽는다. `NotReady`일 때 동의를 받아 `EnsureReadyAsync`로 모델 내려받기. macOS 26 이상에서 `RecognizeDocumentsRequest`로 문단·표 인식. 클립보드 이미지에서 읽기.

## 대안 (Alternatives)

| 대안 | 기각 사유 |
| --- | --- |
| Vision을 언어별로 두 번 돌려 글자 위치로 합치기 | `ko-KR`이 혼용 줄을 통째로 버려 줄 맞추기부터 깨진다 |
| `TextRecognizer`만 쓰기 | Copilot+ PC·패키지 ID가 있어야 해서 대부분의 PC에서 못 읽는다 |
| `RecognizeDocumentsRequest`(Swift) | macOS 26 전용, Swift 정적 라이브러리·런타임 링크가 빌드에 들어온다 |
| PaddleOCR(ONNX, CPU) | onnxruntime·모델로 수십~백 MB가 늘고 전·후처리를 직접 짜야 하며, 혼용문도 잘 읽지 못한다 |
| AI에 이미지만 보내기 | 긴 페이지에서 줄을 건너뛰거나 지어낼 수 있다. 초안은 끌 수 있게 두었다 |
| 읽은 줄을 그대로 넣기 | 이미지의 줄바꿈마다 문단이 나뉜다(ADR-0011) |
