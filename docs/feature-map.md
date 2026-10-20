# Ulysses 기능 → 구현 매핑 (추적 표)

우선순위: **P0** = 핵심 루프, **P1** = Ulysses다움, **P2** = 완성도/확장, **—** = 범위 외
상태: `⬜ 예정` / `🔨 진행` / `✅ 완료` / `⛔ 차단` / `🚫 미구현 결정`

## 1. 라이브러리 / 시트 관리

| 기능 | 우선 | 구현 방안 | 재사용 자산 | M | 상태 |
| --- | --- | --- | --- | --- | --- |
| 그룹(폴더) 계층 | P0 | `core::library` 트리 + 파일시스템 디렉터리 | `tree`, `list` | M3 | ⬜ |
| 시트(문서) 생성/삭제/이름변경 | P0 | frontmatter + 원자적 저장 | `dialog`, `menu::popup` | M1/M3 | ⬜ |
| 드래그로 그룹 이동/정렬 | P0 | 트리 drag&drop + `order` 필드 | `tree`, gpui DnD | M3 | ⬜ |
| 즐겨찾기 | P1 | `SheetMeta.favorite` | `badge` | M3 | ⬜ |
| 휴지통/복원 | P1 | `.trash/` + 메타 보존 | `dialog` | M3 | ⬜ |
| 스마트 그룹(필터 저장) | P1 | `Group::Smart(query)` | FTS 인덱스 | M6 | ⬜ |
| 최근 시트 목록 | P1 | `history` + 로컬 저장 | `history`, 팔레트 | M3 | ⬜ |
| 시트 접합/해제(Shift 선택 + 우클릭) | P1 | 그룹 메타 `glued`(ADR-0012). 미리보기에서 잇기 완료(ADR-0013), 내보내기 잇기는 M7 | `menu::context_menu` | M3 | 🔨 |
| 시트 복제/병합 | P2 | 코어 유틸 | — | M9 | ⬜ |
| 외부 폴더(다중 라이브러리) | P2 | `Library` 다중 루트 | — | M9 | ⬜ |
| Ulysses 라이브러리 가져오기 | P2 | 변환기(격리 모듈) | — | M9 | ⬜ |

## 2. 편집기

| 기능 | 우선 | 구현 방안 | 재사용 자산 | M | 상태 |
| --- | --- | --- | --- | --- | --- |
| 마크다운 시트 편집 | P0 | `Entity<EditorState>` + 로프 | `input::Editor`, gpui-base 엔진 | M2 | ⬜ |
| 소프트랩 / 가독 폭 | P0 | `soft_wrap(true)`, `wrapping_indent`, 최대폭 + 중앙 정렬 | DisplayMap wrap_map | M2 | ⬜ |
| 마크업 스타일링(마크어 흐림, 굵게/기울임/코드/링크) | P0 | 자체 스팬 → `TextDecorationCollection` | decorations API | M2 | ✅ |
| 커서 블록 마커 복원 | P0 | 현재 줄 range 검사 후 스타일 오버라이드 | — | M2 | ⬜ |
| 인용/리스트 좌측 레일 | P1 | `RangeDecoration(Fill)` | decorations | M2 | ⬜ |
| 코드펜스 언어 하이라이트 | P1 | tree-sitter(코드펜스 내부) | `highlighter` | M2 | ⬜ |
| 제목 크기 확대(줄 단위, H1~H6) | P1 | ATX 레벨별 글자·행 높이 배율, gpui-base 패치(ADR-0006) | display_map / wrapper | M2 | ✅ |
| 책 스타일대로 문단 배치(정렬·양쪽 정렬·첫 줄 들여쓰기·문단/제목 간격) | P1 | 줄 = 문단(ADR-0011), 줄별 가로 배치·줄 아래 여백, gpui-base 패치(ADR-0010) | display_map / wrapper | M2 | ✅ |
| 포맷 액션/단축키(굵게/기울임/제목/리스트/인용/코드/링크) | P0 | `actions!` + keymap | `gpui` 액션 | M2 | ⬜ |
| 다중 커서/선택 | P0 | 엔진 기본 | `add_selection` | M2 | ⬜ |
| 되돌리기/다시실행 | P0 | 엔진 기본(세션 내), 세션 밖 히스토리는 P2 | `UndoManager` | M2 | ⬜ |
| 문서 내 찾기/바꾸기 | P0 | `SearchSession` | `input::search` | M2 | ⬜ |
| 이미지 붙여넣기 → 첨부 | P1 | `Editor::on_paste` 훅 | attachment | M2 | ⬜ |
| 인라인 이미지 미리보기 | P2 | 이미지 줄 아래 줄 블록에 그림, gpui-base 패치(ADR-0008) | display_map / `thumbnail` | M2/M4 | ✅ |
| 맞춤법/문법 검사 | — | 범위 외(후속 LSP 연동 가능) | `Lsp` 훅 | — | 🚫 |
| 협업 편집 | — | 범위 외 | — | — | 🚫 |

## 3. 쓰기 보조

| 기능 | 우선 | 구현 방안 | M | 상태 |
| --- | --- | --- | --- | --- |
| 단어/문자/문단/읽기 시간 | P0 | 코어 카운터(debounce) + 상태바 | M2/M5 | ⬜ |
| 목표(words/minutes, 시트·그룹) | P1 | `Goals` + 진행률 배지 + 알림 | M5 | ⬜ |
| 포커스 모드(비활성 문단 흐림) | P1 | decoration `fade_out` | M5 | ⬜ |
| 타자기 모드(커서 고정) | P1 | 스크롤 오프셋 제어(`set_scroll_offset`, `cursor_surrounding_lines`) | M5 | ⬜ |
| 아웃라인/헤딩 내비게이션 | P1 | 제목 스팬 → 사이드 패널 + 점프 | M5 | ⬜ |
| 키워드(#tag) 칩 | P1 | InlineToken + 인덱스 연동 | M5 | ⬜ |
| 각주 칩/미리보기 | P1 | InlineToken + hover | M5 | ⬜ |
| 시트 간 참조(내부 링크) | P1 | 링크 해석 → 시트 점프 | M5 | ⬜ |
| 코멘트/노트 | P2 | 마크다운 확장 구문 + 렌더 | M9 | ⬜ |
| 스니펫/텍스트 매크로 | P2 | 사용자 정의 치환 | M9 | ⬜ |
| 통계 대시보드 | P2 | `chart`/`plot` | M9 | ⬜ |

## 4. 보기 / 뷰

| 기능 | 우선 | 구현 방안 | 재사용 | M | 상태 |
| --- | --- | --- | --- | --- | --- |
| 리치 프리뷰 팬 | P1 | `TextView::markdown` + Ulysses 구문 커스텀 블록(키워드·각주·시트 참조·첨부). 스파이크 C 실패 시 자체 렌더러 | `text::TextView`(markdown-rs) | M4 | ⬜ |
| 분할(마크업/프리뷰) | P1 | `dock` + `resizable` | dock | M4 | ⬜ |
| 스크롤 동기화 | P1 | 블록 anchor 매핑 (TextView 스크롤 API는 스파이크 C에서 확인, 부족하면 블록 래퍼) | — | M4 | ⬜ |
| 탭(다중 시트) | P0 | `dock::TabPanel` | dock/tab | M3 | ⬜ |
| 사이드바 토글/폭 기억 | P0 | 설정 저장 | resizable | M3 | ⬜ |
| 다크/라이트/커스텀 테마 | P0 | theme registry + JSON | `theme` | M0/M8 | ⬜ |
| 폰트/행간/편집폭 설정 | P1 | 설정 페이지 | `setting` | M8 | ⬜ |
| 커맨드 팔레트/퀵 오픈 | P1 | `command` + FTS | command | M6 | ⬜ |
| 전체 화면/집중 레이아웃 | P2 | 윈도우 상태 + 툴바 숨김 | title_bar | M8 | ⬜ |

## 5. 검색

| 기능 | 우선 | 방안 | M | 상태 |
| --- | --- | --- | --- | --- |
| 라이브러리 전체 검색(제목/본문) | P1 | SQLite FTS5 증분 인덱스 | M6 | ⬜ |
| 필터(그룹/키워드/날짜/즐겨찾기) | P1 | 쿼리 빌더 + 결과 하이라이트 | M6 | ⬜ |
| 검색 결과 미리보기 스니펫 | P1 | offset 기반 발췌 | M6 | ⬜ |
| 정규식 검색 | P2 | `regex` 매처 | M6 | ⬜ |

## 6. 내보내기 / 공유

| 포맷 | 우선 | 방안 | M | 상태 |
| --- | --- | --- | --- | --- |
| Markdown(정규화) | P0 | AST → md writer | M7 | ⬜ |
| HTML(+CSS 테마) | P0 | AST → html, assets 인라인/폴더 | M7 | ⬜ |
| RTF | P1 | AST → rtf | M7 | ⬜ |
| PDF | P1 | typst 우선(CJK 폰트 임베딩) | M7 | ⬜ |
| DOCX | P2 | `docx` crate 또는 OOXML 직접 | M7 | ⬜ |
| ePub | P2 | zip + XHTML + 목차 | M7 | ⬜ |
| 프로젝트 미리보기(PDF·ePub, 별도 창) | P1 | 자체 배치 엔진: 판형/기기별 쪽 나눔·양쪽 정렬·제목 배치·표지·차례·쪽 번호, 폴더 감시로 자동 갱신(ADR-0013) | M7 | ✅ |
| 클립보드로 내보내기 | P1 | 포맷 선택 → clipboard | M7 | ⬜ |
| 스타일 테마(폰트/여백/헤더·푸터) | P1 | 프로젝트 `[styles.*]`(ADR-0009) → CSS·typst. 편집 화면(부/장/절 배치·쪽 나눔·접두사·굵기 포함)·가져오기·에디터 반영(쪽 나눔 표시, ¶ 버튼으로 본문/부/장/절 지정) 완료, 내보내기 변환 남음 | M7 | 🔨 |
| 공유 시트(웹 게시) | — | 범위 외 | — | 🚫 |

## 7. 저장 / 동기화 / 버전

| 기능 | 우선 | 방안 | M | 상태 |
| --- | --- | --- | --- | --- |
| 자동 저장 | P0 | debounce + 종료 flush | M1 | ⬜ |
| 외부 변경 감시/충돌 경고 | P1 | `notify` + mtime 비교 | M1 | ⬜ |
| 로컬 버전 히스토리 | P1 | 스냅샷 저장 + `diff` 뷰 | M9 | ⬜ |
| WebDAV 동기화 | P2 | 백그라운드 sync + 3-way 머지 | M9 | ⬜ |
| iCloud 동기화 | — | 플랫폼 종속, 범위 외 | — | 🚫 |
| 암호화/잠금 시트 | P2 | 미정 | M9 | ⬜ |

## 8. 앱 인프라

| 항목 | 우선 | 방안 | M | 상태 |
| --- | --- | --- | --- | --- |
| 워크스페이스 분리(core/markdown/app) | P0 | crates 3개 | M0 | ⬜ |
| 설정 저장/마이그레이션 | P0 | TOML + 스키마 버전 | M1/M8 | ⬜ |
| 키맵 커스터마이징 | P1 | JSON 오버라이드 | M8 | ⬜ |
| i18n(한/영) | P1 | 메시지 카탈로그 | M8 | ⬜ |
| 접근성(라벨/포커스) | P1 | aria_label, focus_trap | M8 | ⬜ |
| CI/릴리스 번들 | P1 | fmt/clippy/test + bundle | M8 | ⬜ |
| 로그/진단 | P0 | tracing + 로그 파일 | M0 | ⬜ |
| 첫 실행 온보딩 | P1 | 라이브러리 선택 위저드 | M8 | ⬜ |
