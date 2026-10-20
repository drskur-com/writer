# ADR-0006: 에디터 제목 줄 확대는 gpui-base를 vendor 패치해 줄 단위 LineStyle로 한다

- 상태: Proposed · 확장: ADR-0010(줄 스타일이 앞뒤 줄을 받고, 가로 배치·줄 아래 여백 추가)
- 날짜: 2026-10-20
- 관련: PLAN.md §1.4·§3.4·M2, `vendor/gpui-base`, `docs/patches/gpui-base-0.8.0-line-style.patch`, `src/app/editor.rs`, `src/markdown/mod.rs`

## 배경 (Context)

에디터에서 마크다운 제목(H1~H6)을 본문보다 크게 보이게 하고 싶다. gpui-base 0.8.0 에디터는 이를 지원하지 않는다.

- `HighlightStyle`·`TextRun`에 글자 크기가 없다. 줄 하나를 `shape_line(text, font_size, runs)` 한 번으로 그리므로 문서 전체가 한 크기다.
- 행 높이는 `row * line_height` 고정 격자로, 스크롤·커서·선택·클릭 판정·줄바꿈 계산이 모두 이를 전제한다.
- 0.8.0에도 줄별 크기·높이 API는 없다.

## 결정 (Decision)

gpui-base 0.8.0을 `vendor/gpui-base`에 두고 `[patch.crates-io]`로 연결한다. **줄 단위** `LineStyle { text_scale, height_scale }`을 추가한다.

- API: `EditorState::set_line_style(Some(|line: &str| -> LineStyle), cx)`. 엔진이 줄을 (다시) 줄바꿈할 때 줄 텍스트로 즉시 판정한다.
- 줄바꿈(wrapper)은 그 줄의 확대된 글자 크기로 계산하고, 레이아웃은 그 크기로 shape한다.
- 줄마다 행 높이가 다르다. 화면 행 수와 행 의미는 그대로라 커서 이동처럼 행 단위로 동작하는 로직은 그대로 둔다. 픽셀↔행 변환만 `DisplayMap`의 `display_row_top`·`display_row_at_y`·`display_row_height`·`content_height`로 바꾼다(키 큰 행 목록 + prefix sum).
- 앱은 ATX 제목(`#`~`######`)만 확대한다. 레벨별 배율 표는 `app/editor.rs`의 `HEADING_SCALES`에 있다. 한 줄 안에서 크기를 섞는 것과 setext 제목은 범위 밖이다.

## 결과 (Consequences)

- 좋아지는 점(+):
  - 에디터에서 제목이 레벨별로 커진다.
  - 판정이 줄 텍스트만 보므로 키 입력 즉시 반영되고 깜빡임이 없다.
  - 제목이 없는 문서에서는 추가 비용이 없다(`styled_lines` 요약이 0이면 건너뜀).
  - 기존 gpui-base input 테스트 357개가 그대로 통과하고, 새 테스트 2개를 추가했다.
- 감수하는 점(-):
  - gpui-kit/gpui-base를 올릴 때마다 패치를 다시 적용해야 한다(`docs/patches/…patch`, 10파일 +406/−63).
  - writer가 `LineStyle` 때문에 `gpui-base`에 직접 의존한다. 버전은 gpui-kit과 맞춰야 한다.
  - 줄 텍스트만 보므로 코드 블록 안의 `# 주석` 줄도 커진다.
  - 터치 캐럿·인라인 토큰·ghost line 경로는 줄별 높이를 반영하지 않았다(이 앱에서는 쓰지 않음).
- 후속 작업(TODO):
  - 코드 펜스 안의 제목 오판정을 해결한다(줄 판정에 블록 문맥 전달).
  - 업스트림 PR을 시도한다.
  - 제목 위·아래 여백을 비대칭으로 둘지 검토한다(지금은 행 안에서 세로 가운데 정렬).

## 대안 (Alternatives)

| 대안 | 기각 사유 |
| --- | --- |
| 제목을 정수 개 행으로 차지(빈 행 끼워 넣기) | 빈 행이 offset을 공유해 커서 위·아래 이동과 클릭 판정에 예외 처리가 필요하다. 높이도 행 단위로만 조절된다 |
| 줄 높이 안에서만 글자 확대(행 높이 고정) | H1 약 1.25배가 한계라 원하는 크기가 안 나온다 |
| 프리뷰 팬에서만 크게 표시 | 편집 중 화면에서 제목이 구분되지 않는다 |
| 자체 에디터 구현 | 비용이 지나치게 크다 |
