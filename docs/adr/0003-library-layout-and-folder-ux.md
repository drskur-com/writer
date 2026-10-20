# ADR-0003: 라이브러리는 단일 루트 + 섹션(Inbox/Notes/Projects) 하위 폴더로 둔다

- 상태: Accepted · 부분 대체: "파생 데이터 위치" 중 그룹 메타(순서·아이콘)는 Superseded by ADR-0005
- 날짜: 2026-10-20
- 관련: PLAN.md §3.2, §3.3, M1/M3/M8 · ADR-0001 · `src/library.rs`, `src/config.rs`

## 배경 (Context)

Ulysses처럼 "메모(Notes)"와 "프로젝트(Projects)"를 지원하려면 저장 구조와 위치 정책이 필요하다.
PLAN.md §3.2/§3.3이 모델과 레이아웃을 이미 제안했고, 아래를 확정해야 했다.

- 라이브러리를 어디에 두는가(설정/데이터 분리, 기본 제안 경로)
- 프로젝트가 독립 최상위 폴더인가, 라이브러리 하위 폴더인가
- 첫 실행/이동/삭제 시 폴더 설정 UX
- 파생 데이터(인덱스/순서/즐겨찾기)를 본문과 어디에 두는가

## 결정 (Decision)

**라이브러리 루트는 사용자가 한 번 지정하는 단일 폴더이고, 그 하위에 고정 섹션 폴더
(Inbox/Notes/Projects)를 둔다. 프로젝트는 `Projects/` 아래의 폴더이며, 본문은 플레인
마크다운이다.**

- **모델**: `Library > Section(Inbox|Notes|Projects) > (Group|Sheet)`. Notes 는 평평(단일 레벨,
  `modified` 최신순), Projects 는 그룹 중첩 + 수동 순서, Inbox 는 빠른 캡처.
- **위치 정책**:
  - 라이브러리 = 사용자 선택. 기본 제안 `~/Documents/Writer` (`dirs::document_dir()`,
    Windows OneDrive 리다이렉트 자동 반영). 단일 루트(다중 라이브러리는 P2/M9 비목표).
  - 설정 = OS 설정 디렉터리 (`<config_dir>/Writer/config.toml`)에 `library_root`,
    `recent_libraries` 저장. 라이브러리와 분리(머신 로컬).
  - 파생 데이터(순서/즐겨찾기/인덱스) = `<Library>/.writer/` (재생성 가능한 캐시, 본문 미오염).
- **저장 레이아웃**:
  ```
  <LibraryRoot>/
  ├─ .writer/{library.toml, index.sqlite}
  ├─ Inbox/
  ├─ Notes/
  └─ Projects/<Project>/<Group>/<Sheet>.md + <name>.assets/
  ```
- **폴더 설정 UX** (권장안 D1~D5 채택):
  - D1 온보딩은 **메인 창 빈 상태** 카드(별도 창 아님).
  - D2 "라이브러리 만들기"는 제안 경로에 즉시 생성, [위치 바꾸기] 제공.
  - D3 비어 있지 않은 폴더(Foreign)는 하위에 `Writer/` 생성을 기본 제안.
  - D4 읽기 전용 모드는 M8로 미룸(경고만).
  - D5 iCloud/OneDrive 감지 안내는 M8.
  - 검증: `Library`(마커 존재) / `Empty` / `MarkdownOnly`(확인 후 사용) / `NonEmpty`(하위 폴더) /
    `Nested`(조상에 마커 → 거부) / `NotFound` / `NotWritable`.
  - 초기화는 `.writer/library.toml`을 **가장 마지막에** 기록(반쪽 라이브러리 방지).
  - 폴더 선택은 OS 네이티브 다이얼로그(`cx.prompt_for_paths`).

## 결과 (Consequences)

- (+): 백업/동기화/전체 검색이 단일 루트 하나로 끝난다. `.md`만으로 이식 가능.
- (+): 설정과 데이터가 분리돼 테마/창 상태가 라이브러리 동기화와 충돌하지 않는다.
- (+): 섹션별 UX 차별화(Notes 최신순 vs Projects 수동 순서/목표)의 토대.
- (-): 프로젝트를 라이브러리 밖에 단독으로 두는 용도는 지원하지 않는다(다중 라이브러리 P2).
- (-): 동기화 폴더에서 `.writer/index.sqlite` 쓰기 충돌 가능 → 재생성 가능 캐시로 취급.
- 후속: L2 파일 스캔/저장, L3 메뉴 Open/Close/Recent·복구 흐름, L4 설정 페이지 연동.

## 대안 (Alternatives)

| 대안 | 기각 사유 |
| --- | --- |
| 프로젝트마다 독립 최상위 폴더(vault-per-project) | 전체 검색·즐겨찾기·휴지통이 다중 루트를 순회, 동기화 범위 애매 |
| 설정을 라이브러리 안에 저장 | 머신별 테마/창 상태가 동기화에 섞여 충돌 |
| 라이브러리 위치를 앱 데이터 폴더로 고정 | 사용자가 위치·동기화를 통제 못함, 백업 불편 |
| 폴더 선택을 자체 구현 | OS 관례/권한/드라이브 처리 부담 |