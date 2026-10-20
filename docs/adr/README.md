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

## 작성 규칙

1. 파일명: `NNNN-짧은-제목.md` (4자리, 소문자·하이픈).
2. 템플릿: [`0000-template.md`](./0000-template.md).
3. 상태: `Proposed` → `Accepted` → (`Deprecated` | `Superseded by ADR-XXXX`).
4. 한 문서 = 한 결정. "배경 → 결정 → 결과 → 대안"을 유지하고, 코드 위치를 링크한다.
5. 대체 시: 새 ADR 에 `Supersedes ADR-XXXX` 표기, 이전 ADR 상태를
   `Superseded by ADR-YYYY` 로 갱신.