# ADR (Architecture Decision Record)

이 디렉터리는 이 프로젝트의 **의사결정 기록**을 담는다. ADR 은 "무엇을 만들었나"가 아니라
**"왜 그렇게 하기로 했나"** 를 결정 시점에 남기는 짧은 문서다.

- 배경·계획은 [`../PLAN.md`](../PLAN.md), 기능 추적은 [`../feature-map.md`](../feature-map.md).
- 구현/사용법 문서가 아니다. 시간이 지나 결정이 바뀌면 기존 문서를 고치지 않고
  **새 ADR 로 대체(supersede)** 한다.
- 실험 결과는 `docs/spikes/`에 두고, 거기서 "그래서 이걸 쓴다"가 확정되면 ADR 로 승격한다.

## 목록

| 번호 | 제목 | 상태 | 날짜 |
| --- | --- | --- | --- |
| [0001](./0001-theme-colors-single-source.md) | 테마 색상은 JSON 을 단일 출처로 한다 | Accepted | 2026-10-20 |
| [0002](./0002-custom-window-chrome.md) | Windows 창 크롬은 커스텀 타이틀바로 대체하고 아이콘 에셋을 등록한다 | Accepted | 2026-10-20 |
| [0003](./0003-library-layout-and-folder-ux.md) | 라이브러리는 단일 루트 + 섹션(Inbox/Notes/Projects) 하위 폴더로 둔다 | Accepted (일부 Superseded by 0005) | 2026-10-20 |
| [0004](./0004-icon-pack.md) | 아이콘 팩은 Lucide 전체 카탈로그(AllAssets)를 쓰고 매핑을 한 곳에 둔다 | Accepted | 2026-10-20 |
| [0005](./0005-group-meta-in-group-folder.md) | 그룹 메타는 그룹 폴더의 `.writer.toml`에 둔다 | Accepted | 2026-10-20 |
| [0006](./0006-heading-line-style-gpui-base-patch.md) | 에디터 제목 줄 확대는 gpui-base를 vendor 패치해 줄 단위 LineStyle로 한다 | Proposed | 2026-10-20 |
| [0007](./0007-image-attributes-pandoc-syntax.md) | 이미지 크기·정렬·전면 배치는 Pandoc 속성 문법으로 마크다운에 적는다 | Proposed | 2026-10-20 |
| [0008](./0008-editor-image-line-blocks.md) | 에디터 이미지는 이미지 줄 아래 "줄 블록"으로 그리고, 이미지 줄은 커서가 있을 때만 보인다 | Proposed | 2026-10-20 |
| [0009](./0009-project-book-styles.md) | 책 스타일은 프로젝트 `.writer.toml`의 `[styles.<대상>]`에 두고, 다른 프로젝트에서는 값을 복사해 가져온다 | Proposed | 2026-10-20 |
| [0010](./0010-editor-book-layout.md) | 에디터는 책 스타일대로 문단을 배치하고, 블록 사이 여백은 앞 줄 아래에 둔다 | Proposed | 2026-10-20 |
| [0011](./0011-line-per-paragraph-editing.md) | 에디터에서는 줄 하나가 문단이고, 파일에는 표준 마크다운으로 변환해 저장한다 | Proposed | 2026-10-20 |
| [0012](./0012-sheet-glue-flag.md) | 시트 접합은 파일을 합치지 않고 그룹 메타 `glued`에 "앞 시트에 이어짐"으로 적는다 | Proposed (이동 규칙 일부 Superseded by 0014) | 2026-10-20 |
| [0013](./0013-book-preview-window.md) | 프로젝트 미리보기는 별도 창에서 자체 배치 엔진으로 PDF·ePub 페이지를 그린다 | Proposed | 2026-10-21 |
| [0014](./0014-pdf-export-from-preview-layout.md) | PDF는 미리보기 배치를 그대로 krilla로 찍는다(typst 대신) | Proposed | 2026-10-21 |
| [0015](./0015-docx-export-word-styles.md) | DOCX는 원고 블록을 Word 스타일 문서로 만든다(미리보기 배치를 옮기지 않는다) | Proposed | 2026-10-21 |
| [0014](./0014-individual-sheet-move-glue.md) | 시트는 개별로 옮기고 이동 위치에 따라 접합을 유지·해제·추가한다 | Proposed | 2026-10-21 |
| [0016](./0016-line-group-fence.md) | 줄 묶음은 `:::` 펜스로 감싸고, 책에서는 구간 전체를 강제 줄바꿈으로 이은 한 문단으로 그린다 | Proposed | 2026-10-21 |
| [0017](./0017-editor-table-grid.md) | 에디터의 표는 마지막 행 아래 줄 블록에 격자로 그리고, 커서가 표에 닿으면 원문 행 전체를 보인다 | Proposed | 2026-10-21 |
| [0018](./0018-unnumbered-heading-attribute.md) | 번호 없는 제목(서장·에필로그 등)은 시트 첫 줄 제목의 `{unnumbered}` 속성으로 표시한다 | Proposed | 2026-11-14 |
| [0019](./0019-cross-group-sheet-move.md) | 시트를 끌어 다른 그룹으로 옮기면 파일을 옮기고 두 그룹의 순서·접합을 함께 저장한다 | Proposed | 2026-10-22 |
| [0020](./0020-group-multiselect-move.md) | 그룹을 Shift로 여러 개 골라 다른 그룹의 하위로 끌어 옮긴다 | Proposed | 2026-10-22 |
| [0021](./0021-image-path-resolution.md) | 이미지 경로는 한 곳에서 찾고, `../`가 어긋난 경로는 위 폴더에서 다시 찾는다 | Proposed | 2026-10-22 |
| [0022](./0022-project-relative-image-paths.md) | 프로젝트 시트의 이미지 경로는 프로젝트 폴더 기준으로 적는다 | Proposed | 2026-10-22 |
| [0023](./0023-editor-task-checkbox-overlay.md) | 에디터의 할 일 항목은 기호 위에 체크 상자를 겹쳐 그리고, 누르면 원문의 `[ ]`·`[x]`만 바꾼다 | Proposed | 2026-10-22 |
| [0024](./0024-note-todos.md) | 메모의 할 일은 원문을 유일한 저장소로 두고, 할 일 화면은 메모를 훑어 모은 색인으로 만든다 | Proposed | 2026-10-22 |
| [0025](./0025-editor-link-open.md) | 에디터 링크는 보조 키(macOS ⌘, Windows Ctrl)+클릭으로 연다 | Proposed | 2026-10-22 |
| [0026](./0026-lua-extensions.md) | 확장은 라이브러리의 Lua(Luau) 스크립트로 두고, 시트의 `` ```ext `` 블록에 캔버스로 그리거나 메뉴 명령을 더한다 | Proposed | 2026-10-23 |
| [0027](./0027-ui-i18n.md) | UI 글은 rust-i18n으로 영어·한국어를 고른다 | Proposed | 2026-10-23 |

## 작성 규칙

1. 파일명: `NNNN-짧은-제목.md` (4자리, 소문자·하이픈).
2. 템플릿: [`0000-template.md`](./0000-template.md).
3. 상태: `Proposed` → `Accepted` → (`Deprecated` | `Superseded by ADR-XXXX`).
4. 한 문서 = 한 결정. "배경 → 결정 → 결과 → 대안"을 유지하고, 코드 위치를 링크한다.
5. 대체 시: 새 ADR 에 `Supersedes ADR-XXXX` 표기, 이전 ADR 상태를
   `Superseded by ADR-YYYY` 로 갱신.
