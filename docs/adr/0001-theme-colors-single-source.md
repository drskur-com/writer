# ADR-0001: 테마 색상은 JSON 을 단일 출처로 한다

- 상태: Accepted
- 날짜: 2026-10-20
- 관련: PLAN.md §3.1, §3.5, M8 · `themes/writer.json` · `src/theme.rs`

## 배경 (Context)

초기 셸(`src/main.rs`)은 모든 색을 `rgb(0x18181b)` 처럼 하드코딩했다.

- 색을 바꾸려면 코드를 수정·재컴파일해야 하고, 같은 색이 여러 곳에 중복된다.
- gpui-component 위젯(타이틀바·메뉴 팝업·토스트·스크롤바)은 `cx.theme()` 토큰을 읽으므로,
  하드코딩 값과 테마 토큰이 어긋나 룩앤필이 깨진다(실제로 기본 Light 테마와 충돌했다).
- 라이트/다크, 사용자 테마, 설정 화면(M8)까지 고려하면 색의 "단일 출처"가 필요하다.

gpui-component 는 `ThemeRegistry` + `ThemeConfig`(JSON `ThemeSet`) 스키마와
`Theme::apply_config` 라는 이미 갖춰진 테마 체계를 제공한다.

## 결정 (Decision)

**`themes/writer.json`(ThemeSet JSON)을 색상의 단일 출처로 삼고, 앱 코드는
하드코딩 색 대신 `cx.theme()` 시맨틱 토큰만 사용한다.** 등록·전환은 `src/theme.rs` 로 캡슐화한다.

- 등록: `ThemeRegistry::load_themes_from_str(include_str!("../themes/writer.json"))`
- 적용: `Theme::update(cx, |t| t.apply_config(&config))` (이름 기반)
- 노출: `theme::register / set_theme / theme_names / active_name`

## 결과 (Consequences)

- (+): 색 변경은 JSON 수정만으로 끝난다. 라이트/다크 한 쌍이 같은 스키마로 관리된다.
- (+): 컴포넌트 위젯과 앱 UI 가 같은 토큰을 공유해 룩앤필이 일치한다.
- (+): 메뉴(`View > Theme`)가 레지스트리를 읽어 자동으로 채워진다 — 테마 추가 = JSON 배열 추가.
- (-): 모든 UI 가 토큰을 써야 하므로 하드코딩 색은 기술 부채가 된다(리뷰에서 금지).
- (-): 색상 키 이름이 gpui-component 스키마에 종속된다.
- 후속: 깨진 JSON 이 앱 시작을 막지 않도록 로그만 남기고 기본 테마로 진행(구현 완료).
- 후속: 사용자 테마 디렉터리 감시(`ThemeRegistry::watch_dir`), 선택값 영속화(설정 저장).

## 대안 (Alternatives)

| 대안 | 기각 사유 |
| --- | --- |
| Rust 상수 팔레트 (`const PALETTE: ...`) | 타입 안전하지만 변경마다 재컴파일, 사용자 테마/런타임 전환 불가 |
| 위젯별 개별 색 지정 | 중복·불일치, 컴포넌트 테마와 이중 관리 |
| 자체 테마 시스템 신규 구현 | gpui-component 가 이미 스키마·레지스트리·적용 경로를 제공 — 중복 |