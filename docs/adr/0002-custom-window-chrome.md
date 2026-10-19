# ADR-0002: Windows 창 크롬은 커스텀 타이틀바로 대체하고 아이콘 에셋을 등록한다

- 상태: Accepted
- 날짜: 2026-10-20
- 관련: ADR-0001 · `src/main.rs` · gpui-component `TitleBar`

## 배경 (Context)

Windows 에서 앱 상단이 앱의 다크 룩앤필과 어긋나고, 창 제어 아이콘이 보이지 않았다.

1. `WindowOptions.titlebar.appears_transparent = false` 라서 OS 네이티브 타이틀바(밝은 크롬)가
   그대로 그려졌다. gpui-pre 는 이 값이 `true` 일 때만 `WM_NCCALCSIZE`/hit-test 를 직접 처리해
   타이틀바를 숨긴다(`gpui-pre-windows` 의 `hide_title_bar`).
2. 아이콘(`IconName::WindowMinimize/Maximize/Close`)은 `gpui_kit_assets` 의 SVG 를
   `AssetSource` 로 로드하는데, `gpui_kit::application()` 만 쓰면 에셋 소스가 등록되지 않아
   아이콘이 렌더되지 않았다.
3. 타이틀바 색은 `cx.theme()` 를 따르는데 테마가 기본 Light 였다(→ ADR-0001).

## 결정 (Decision)

**Windows/Linux 에서 네이티브 타이틀바를 숨기고 gpui-component 의 `TitleBar` 컴포넌트로
대체하며, 그 안에 `AppMenuBar` 를 넣어 상단을 한 줄로 통합한다.** 아이콘을 위해 에셋을 등록한다.

- 창 옵션: `..TitleBar::window_options()` (즉 `appears_transparent: true`, `app_owns_titlebar_drag: true`)
- 렌더: `TitleBar::new().bg(title_bar).border_color(title_bar_border).child(menu_bar)`
- 에셋: `gpui_kit::application().with_assets(gpui_kit::assets::Assets)`
- macOS 는 `#[cfg]` 분기로 네이티브 타이틀바 + 네이티브 메뉴를 그대로 유지.

## 결과 (Consequences)

- (+): OS 밝은 크롬이 사라지고 다크 테마와 일치하는 한 줄 상단이 된다.
- (+): `TitleBar` 가 드래그·더블클릭 최대화·최소화/최대화/닫기를 처리하므로 직접 구현이 없다.
- (+): 메뉴바와 타이틀바가 합쳐져 세로 공간을 절약한다.
- (-): Windows/Linux 는 커스텀 크롬이라 gpui-component `TitleBar` 동작에 의존한다.
- (-): 에셋을 등록하지 않은 창은 아이콘이 비어 보인다(모든 창에서 `with_assets` 필요).
- 후속: OS 설정을 따르는 테마 동기화(`sync_system_appearance`)와 창 버튼 hover 색 토큰 점검.

## 대안 (Alternatives)

| 대안 | 기각 사유 |
| --- | --- |
| 네이티브 타이틀바 유지 | 다크 앱과 시각적 불일치 — 애초의 문제 |
| 타이틀바를 완전히 제거하고 메뉴 행에 창 버튼 직접 구현 | 컴포넌트가 이미 제공 — 재구현 비용만 증가(필요 시 후속) |
| `appears_transparent` 만 켜고 컨트롤 생략 | 창 이동/최소화/닫기 불가 |