# ADR-0025: 에디터 링크는 보조 키(macOS ⌘, Windows Ctrl)+클릭으로 연다

- 상태: Proposed
- 날짜: 2026-10-22
- 관련: ADR-0021·0022(경로 해석) · `src/app/editor_link.rs`, `src/markdown/mod.rs`(`link_at`), `vendor/gpui-base/src/input/editor/writer.rs`(`set_secondary_click`)

## 배경 (Context)

에디터의 링크(`[글](주소)`, `<주소>`, 맨 주소)는 파란 글자와 밑줄로 보이지만 누를 수 없었다. 그냥 클릭은 링크
글을 고치려고 커서를 놓는 동작이라 바꾸면 안 된다.

- gpui-base에는 LSP "정의로 이동"용 ⌘+클릭이 있다. 하지만 마우스를 움직일 때 비동기로 찾아 둔 결과를 쓰므로,
  링크 위에 둔 채 ⌘를 누르고 바로 누르면 이동하지 않는다. 우클릭 메뉴에 "Go to Definition"도 생긴다.

## 결정 (Decision)

**보조 키(macOS ⌘, Windows·Linux Ctrl, gpui `Modifiers::secondary`)를 누른 클릭을 앱 처리기에 먼저 넘긴다.**

- gpui-base: `EditorState::set_secondary_click(handler)`. 보조 키 클릭이 캐럿을 놓기 전에
  `handler(문서, 누른 바이트 위치)`를 부르고, `true`면 그 클릭을 먹는다(캐럿이 움직이지 않는다). Alt 클릭(여러 캐럿)은
  넘기지 않는다. 처리기는 에디터를 고치는 중에 불리므로 에디터를 바꾸는 일은 `window.defer`로 미룬다.
- 앱: 누른 자리의 링크 주소를 `markdown::link_at`으로 찾는다(링크 글·기호·주소 어디든). 링크가 아니면 평소 클릭이다.
  - 스킴이 있는 주소(`https:`·`mailto:` 등, `file:` 제외) → 시스템 기본 앱(`open_url`).
  - 경로 → `%XX`를 풀고 시트 폴더 기준으로 찾는다(이미지와 같은 규칙). 라이브러리의 `.md`면 앱에서 연다(다른
    프로젝트·메모면 그 화면과 그룹으로 옮겨 간다). 그 밖의 파일은 시스템 기본 앱으로 연다. 못 찾으면 알린다.
  - `#제목`(문서 안 이동)은 아직 없다.

## 결과 (Consequences)

- (+): 클릭 순간에 판정하므로 마우스를 움직이지 않아도 연다. macOS·Windows가 각자의 관례(⌘·Ctrl)를 쓴다.
- (+): 그냥 클릭·Alt 클릭은 그대로다.
- (-): gpui-base 패치가 조금 늘었다(필드 하나, 호출 몇 줄, API 하나).
- (-): ⌘를 눌렀을 때 링크 위에서 손가락 커서로 바뀌는 표시는 없다.
- 후속(TODO): ⌘ 누를 때 손가락 커서, 키보드로 열기(커서 위치 링크), 문서 안 `#제목` 이동, 우클릭 메뉴 "링크 열기".

## 대안 (Alternatives)

| 대안 | 기각 사유 |
| --- | --- |
| LSP 정의 제공자로 링크를 넘기기 | 마우스를 움직여야 결과가 생겨 바로 누르면 놓친다. 우클릭 메뉴에 "Go to Definition"이 생긴다. `lsp-types` 의존성이 필요하다 |
| 그냥 클릭으로 열기 | 링크 글을 고칠 수 없다 |
