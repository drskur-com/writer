# 율리시즈(Ulysses) 에디터 포팅 계획

> 작성일: 2026-10-19 · 상태: 초안(승인 대기)
> 관련 문서: [`feature-map.md`](./feature-map.md) (기능 인벤토리/추적 표)

---

## 0. 한 줄 요약

**gpui-kit 기반 Rust 데스크톱 앱에, Ulysses의 "마크다운 시트 + 라이브러리 + 쓰기 보조 + 내보내기" 경험을 이식한다.**
편집 엔진은 이미 스택 안에 있으므로(gpui-base의 Rope/DisplayMap/Decorations), 신규 개발은
① 문서·라이브러리 모델, ② 마크다운 프리젠테이션 계층, ③ 쓰기 보조 UX, ④ 내보내기/동기화에 집중한다.

---

## 1. 현황 진단

### 1.1 저장소 상태

| 항목 | 내용 |
| --- | --- |
| 크레이트 | 단일 bin crate `writer` (edition 2024), 의존성 `gpui-kit = "0.7"` |
| 코드 | `src/main.rs` 160줄 — 하드코딩된 3-pane 껍데기(타이틀바/사이드바/캔버스/상태바), 파일 I/O·에디터 없음 |
| 빌드 | `cargo check` 통과 (약 1m35s, dev + sccache 설정은 주석 처리됨) |
| 도구 | `mise.toml`에 check/dev/watch/build-fast 태스크, `cargo-watch` |
| VCS | `main` 2커밋, remote `origin/main` |

### 1.2 스택이 **이미 제공하는 것** (직접 만들지 말 것)

`gpui-kit 0.7.3` → `gpui(gpui-pre 0.3.9, Zed gpui 스냅샷 rev 93d7456)` + `gpui-base 0.7.3` + `gpui-component 0.7.3`

| 계층 | 재사용 가능 자산 | Ulysses 대응 기능 |
| --- | --- | --- |
| 텍스트 엔진 | `ropey::Rope`, `InputBaseState<EditorMode>`, `DisplayMap`(wrap_map/fold_map/inline_line), `UndoManager`, 멀티 셀렉션(`add_selection`), IME/네이티브 입력, `SearchSession`/`SearchMatcher` | 시트 편집, 소프트랩, 되돌리기, 다중 커서, 문서 내 검색 |
| 스타일/장식 | `TextDecorationCollection`(HighlightStyle), `RangeDecorationCollection`(Fill/Frame), `set_editor_style`, `set_editor_paddings` | 마크업 구문 시각화(`#` 흐리게, **굵게**, 밑줄, 하이라이트) |
| 하이라이팅 | tree-sitter `InputHighlighter`, 내장 언어에 **markdown / markdown_inline** 포함 (feature `tree-sitter-markdown*`) | 코드펜스/인라인 마크업 기본 색상 |
| 인라인 커스텀 렌더 | `InlineToken`/`InlineTokenSpan`/`InputContent` + `InlineTokenRenderer`(click/hover 이벤트, `available_width`) | 키워드 칩, 각주 마커, 링크 배지, 첨부 파일 토큰 |
| 에디터 요소 | `component::input::Editor`(상태 기반), `Textarea`, `Input`, 컨텍스트 메뉴 훅, 붙여넣기 가로채기(`on_paste` → 이미지/파일) | 본문 에디터, 이미지 붙여넣기 |
| 레이아웃/위젯 | `dock`(탭·분할 패널), `tab`, `tree`, `list`/`virtual_list`, `searchable_list`, `resizable`, `sidebar`, `title_bar`, `toolbar`, `status_bar`, `command`(커맨드 팔레트), `menu`(popup/context/dropdown/app_menu_bar), `dialog`, `notification`, `popover`, `tooltip`, `kbd`, `badge`, `tag`, `attachment`, `setting`(설정 UI), `theme`(커스텀 테마 레지스트리 + JSON 스키마) | 라이브러리 사이드바, 탭/분할, 퀵 오픈, 우클릭 메뉴, 환경설정 |
| 유틸 | `gpui::actions!` + keymap 디스패치, `history`, `theme`, focus/scroll 유틸 | 단축키 체계 |

### 1.3 **직접 만들어야 하는 것**

1. **라이브러리/문서 도메인** — 그룹(폴더)·시트(문서) 트리, 메타데이터, 첨부, 휴지통, 자동 저장, 파일 감시
2. **마크다운 프리젠테이션 계층** — 원문 offset → 블록/인라인 AST → (a) 에디터 decoration 세트, (b) 프리뷰 렌더 IR
3. **리치 프리뷰 뷰** — gpui-component에 마크다운 렌더러가 **없음**. 자체 구현 필요
4. **쓰기 보조 UX** — 단어/문자/읽기 시간 통계, 목표(goals), 포커스 모드, 타자기 모드, 아웃라인 내비게이션, 스마트 그룹
5. **검색·인덱스** — 라이브러리 전체 FTS, 필터, 최근 문서
6. **내보내기** — HTML/Markdown/RTF/PDF/DOCX/ePub + 스타일 테마
7. **동기화·버전** — WebDAV, 로컬 버전 히스토리, 충돌 해결
8. **설정/테마/키맵/패키징** — 환경설정 저장, 커스텀 테마, 설치본

### 1.4 조사로 확인한 **제약 사항** (설계에 반영)

| 제약 | 근거 | 영향 & 대응 |
| --- | --- | --- |
| `HighlightStyle`에 `font_size` 필드가 없음 (color/font_weight/font_style/background_color/underline/strikethrough/fade_out만 존재) | `gpui-pre/src/style.rs:580` | **에디터 팬 안에서 제목을 크게 표시 불가** → Ulysses의 기본 "Markup" 모드와 동일 전략(굵기·색·마커 흐림으로 구분)을 P0으로, 큰 제목은 **프리뷰 팬**에서 구현. 필요 시 gpui-base 포킹/업스트림 패치를 M2 스파이크에서 판단 |
| 라인 높이가 에디터 단위(상대값)로 균일 | `Editor::EDITOR_LINE_HEIGHT = 1.5` | 블록 간 여백(문단/제목 spacing)은 에디터에서 표현 불가 → 프리뷰에서 해결 |
| 인라인 이미지 블록(변하는 행 높이) 지원 불명확 | `InlineTokenRenderer`에 `available_width`만 노출 | **M2에서 1일 스파이크 필수**. 안 되면 이미지는 "칩 + 프리뷰 렌더"로 대체 |
| gpui-pre는 Zed 스냅샷(버전 변동성 큼) | `Cargo.toml` 메타데이터 | `Cargo.lock` 커밋 유지, 업그레이드는 별도 브랜치 + 회귀 체크리스트 |
| Windows 플랫폼/IME/한중일 폰트 렌더링 검증 필요 | `gpui-pre-windows` 존재하나 미검증 | M0에서 한글 입력·폰트 폴백 스모크 테스트 |
| 마크다운 리치 렌더러·PDF/DOCX 파이프라인은 전부 신규 | — | M4/M7에 예산 집중 |

---

## 2. 포팅 범위 (P0/P1/P2)

상세 표는 [`feature-map.md`](./feature-map.md) 참조. 요약:

- **P0 (핵심 루프 — 없으면 앱이 성립 안 함)**
  마크다운 시트 편집, 라이브러리 트리(그룹/시트 CRUD), 자동 저장, 소프트랩, 마크업 스타일링, 탭, 단어 수, 기본 단축키/포맷 액션, 검색(문서 내), 다크/라이트 테마
- **P1 (Ulysses다움)**
  프리뷰 분할, 포커스/타자기 모드, 아웃라인 내비게이션, 퀵 오픈(커맨드 팔레트), 키워드·각주·링크·첨부, 목표(goals)·통계, 라이브러리 전체 검색, 스마트 그룹, 휴지통, 내보내기(HTML/MD/PDF), 버전 히스토리, 환경설정
- **P2 (완성도/확장)**
  DOCX·ePub 내보내기 + 스타일 테마, WebDAV 동기화, Ulysses 라이브러리 가져오기, 통계 대시보드, 다중 분할 편집, 이미지 인라인 렌더, 플러그인/매크로, 모바일 동반 앱(범위 외)
- **명시적 범위 외**
  iCloud 동기화(플랫폼 종속), Ulysses 계정/구독, 협업 편집, iOS/Android 클라이언트

---

## 3. 목표 아키텍처

### 3.1 워크스페이스 구조

```
writer/
├─ Cargo.toml                 # workspace root (+ [profile] 유지)
├─ crates/
│  ├─ core/                   # UI 무관 도메인 (gpui 의존 금지)
│  │  └─ src/{library, sheet, meta, attachments, goals, stats, index, sync, settings}/
│  ├─ markdown/               # 파싱·매핑·렌더 IR·내보내기 (gpui 의존 금지)
│  │  └─ src/{ast, spans, decorations, preview_ir, export/{html,md,rtf,pdf,docx,epub}}/
│  └─ app/                    # gpui UI (bin)
│     └─ src/{app, window, panes/{library,editor,preview,outline}, dialogs, palette, theme, actions}/
├─ assets/                    # 폰트, 아이콘, 기본 테마 JSON, 내보내기 CSS/템플릿
├─ docs/                      # PLAN, feature-map, ADR, 스파이크 결과
└─ tests/corpus/              # 마크다운 골든 샘플 (CommonMark + Ulysses 구문)
```

**원칙**
- 의존 방향: `app → markdown → core`. 코어/마크다운은 gpui를 몰라야 유닛 테스트가 빠르다.
- `markdown`만 `gpui` 타입(`HighlightStyle`, `Pixels`)을 **선택적 feature**로 사용하고, 기본 출력은 UI 중립 IR(`StyleSpec`)로 둔다.
- 앱 상태는 "엔티티 그래프"로: `Workspace`(root) → `LibraryView` / `DockView` → `SheetEditor`(시트별 `Entity<EditorState>` + 도메인 `Entity<Sheet>`) / `PreviewView`.

### 3.2 데이터 모델

```
Library   { root_path, sections: LibrarySections, sheets: Map<SheetId, SheetMeta>, trash, index }
LibrarySections {
    inbox: Option<Group>,       # 빠른 임시 캡처
    notes: Group,               # 📝 메모 영역 (일반 노트, 데일리, 단일 시트 중심)
    projects: Group,            # 📚 프로젝트 영역 (장편 집필, 챕터/순서 트리, 목표 단어 관리)
}
Group     { id, name, parent, order, kind: Normal|Section|Smart(query), collapsed }
Sheet     { id, path(.md), title(=첫 H1 or 파일명), group_id, keywords[], goals,
            created/modified/opened_at, word_count, favorite, attachment_dir }
Attachment { id, sheet_id, kind: Image|File|Note, path, caption, alt, range_hint }
Settings   { library_root, theme, fonts, line_spacing, editor_width, autosave_ms, keymap_overrides, sync }
```

### 3.3 저장 포맷 및 디렉터리 레이아웃 (결정 필요 → §8 Q2)

**권장안: "단일 라이브러리 루트 + 섹션(Notes/Projects) 폴더 분할 + 플레인 마크다운 + SQLite 인덱스"**

```
MyLibrary/                      # 사용자가 지정한 라이브러리 루트 폴더
├─ .writer/                     # 앱 내부 캐시 & 메타 (git ignore 가능)
│  └─ index.sqlite              # FTS5 전문 검색 + 메타데이터 캐시
├─ Inbox/                       # (선택) 빠른 임시 캡처/받은편지함
├─ Notes/                       # 📝 메모 영역 (날짜순/태그/가벼운 단일 시트)
│  ├─ Daily/
│  └─ Ideas.md
└─ Projects/                    # 📚 프로젝트 영역 (책/원고/챕터 트리, 수동 순서 정렬, 통합 내보내기)
   └─ MyNovel/
      ├─ 01_Chapter.md
      ├─ 02_Chapter.md
      └─ characters.assets/     # 첨부 리소스
```

- **본문 포맷**: 항상 **표준 마크다운**(`md`)으로 유지(다른 앱 호환 및 git 친화).
- **메타데이터**: 키워드/목표 등은 YAML frontmatter에 기록하고, 검색·정렬용 파생 데이터는 `.writer/index.sqlite`에 캐시.
- **첨부물**: `시트이름.assets/` 폴더에 저장하고 상대 경로(`![](./시트이름.assets/img.png)`)로 참조.
- **영역별 UX 특화**:
  - **Notes(메모)**: 최근 수정순/날짜순 자동 정렬, 빠른 캡처 단축키, 태그 중심 탐색.
  - **Projects(프로젝트)**: 수동 드래그 순서(`order`), 전체 목표 단어 수(Goal), 다중 시트 일괄 병합/내보내기(PDF/DOCX/ePub).

### 3.4 에디터 렌더링 파이프라인

```
Rope(원문)
  └─> [markdown] 증분 파서(pulldown-cmark 기반, offset 추적)
        ├─> BlockSpan[]  (heading/paragraph/list/quote/code/table/hr/image)
        └─> InlineSpan[] (strong/em/strike/code/link/keyword/footnote/marker)
              ├─(a) DecorationMapper → TextDecorationCollection / RangeDecorationCollection
              │      · 마커(`#`, `*`, `[]()`) fade_out + 크기 축소 대신 색 약화
              │      · strong=bold, em=italic, code=mono+bg, link=underline+색, quote=색+좌측 RangeDecoration(Fill)
              │      · 현재 커서 줄/블록은 마커를 원래 색으로 복원(Ulysses 동작)
              └─(b) PreviewIr → gpui 요소 트리(리치 프리뷰) / HTML·DOCX·ePub 내보내기
```

- **성능 규칙**: 편집 이벤트 → 150ms debounce → 파싱 → diff → `collection.set()` (전체 교체는 가시 블록 범위로 제한).
- **하이라이팅 우선순위**: 자체 스팬 > tree-sitter(코드펜스 내부 언어용) > 기본 스타일.

### 3.5 액션/키맵

`gpui::actions!`로 네임스페이스 분리: `app::editor`(Bold/Italic/Link/Heading1..6/List/Quote/Code/Keyword/Footnote), `app::library`(NewSheet/NewGroup/Rename/Move/Trash), `app::view`(TogglePreview/FocusMode/TypewriterMode/ToggleSidebar), `app::export`(Html/Pdf/Docx/Epub), `app::nav`(QuickOpen/CommandPalette/GotoHeading). macOS 기본 키맵 우선, `default.json` + 사용자 오버라이드.

---

## 4. 마일스톤

각 마일스톤은 "완료 기준(acceptance)"을 통과해야 다음으로 간다. 스파이크 결과는 `docs/spikes/`에 ADR로 기록.

### M0 — 기반 정리 (1~2일)
- [ ] Cargo 워크스페이스 전환(`crates/core`, `markdown`, `app`), 루트 bin 제거 또는 `app`으로 이동
- [ ] `gpui-kit` feature 활성화: `tree-sitter`, `tree-sitter-markdown`, `tree-sitter-markdown-inline` (+ 빌드 시간 측정)
- [ ] `mise` 태스크 보강: `check-all`, `test`, `clippy`, `fmt`, `run-release`
- [ ] 앱 스켈레톤을 `dock` + `resizable` + `title_bar` + `status_bar` 기반 실제 레이아웃으로 교체
- [ ] 한글 IME·CJK 폰트 렌더 스모크 테스트(Windows/macOS)
- **완료 기준**: 3-pane 레이아웃이 리사이즈/폴딩되고, 한글 입력·복붙이 깨지지 않으며, `cargo test`가 워크스페이스 전체에서 통과한다.

### M1 — 문서 모델 & 저장소 (2~3일)
- [ ] `core`: `Library`/`Group`/`Sheet` 모델, 파일 스캔(`walkdir`), frontmatter 파싱, 원자적 저장(temp+rename)
- [ ] 자동 저장(debounce + 포커스 아웃 + 종료 시 flush), dirty 상태 추적
- [ ] 파일 시스템 감시(`notify`) → 외부 수정 시 reload/충돌 경고
- [ ] `Cmd/Ctrl+S`, `O`, `N` 최소 루프 연결
- **완료 기준**: 폴더를 열면 시트 목록이 뜨고, 편집→자동 저장→앱 재실행 후 상태(커서 제외)가 복원된다. 100개 시트 스캔 < 100ms.

### M2 — 에디터 코어 & 마크업 스타일링 (5~7일) ★핵심
- [ ] **스파이크 A(1일)**: `EditorState` + `markdown` 언어 + `TextDecorationCollection`로 `# 제목`/`**굵게**`/`[링크]()` 시각화 POC
- [ ] **스파이크 B(1일)**: `InlineTokenRenderer`로 이미지/칩 인라인 렌더 가능 여부 → 불가 시 칩 대체안 확정
- [ ] `markdown`: pulldown-cmark 기반 offset 정확 파서 + `BlockSpan/InlineSpan` + 골든 테스트(corpus)
- [ ] DecorationMapper(커서 블록 마커 복원, quote/list 좌측 레일, 코드펜스 배경)
- [ ] 소프트랩/래핑 인덴트, 에디터 패딩·최대 폭(가독 폭居中), 스크롤 여백
- [ ] 포맷 액션 + 키맵(굵게/기울임/제목/리스트/인용/코드/링크), 선택 토글 동작
- [ ] 붙여넣기 가로채기(이미지 → 첨부 저장 후 마크다운 삽입)
- **완료 기준**: 5,000단어 문서에서 타이핑 지연 < 16ms(장식 재계산 포함), CommonMark 골든 100% 통과, Ulysses 스크린샷 대비 마크업 색상 표 완성.

### M3 — 라이브러리 UI (4~6일)
- [ ] `tree`/`list` 기반 사이드바: 고정 섹션(Inbox, Notes, Projects) + 하위 그룹/시트 계층, 드래그 이동/정렬, 펼침 상태 저장
- [ ] 영역별 정렬/뷰 차별화(Notes: 최근 수정일순, Projects: 수동 순서 정렬)
- [ ] 컨텍스트 메뉴(새 시트/그룹, 이름 바꾸기, 이동, 복제, 즐겨찾기, 휴지통), 인라인 rename
- [ ] `dock` 탭: 다중 시트, 닫기/순서/더티 표시, 최근 문서
- [ ] 상태바: 시트 경로·저장 상태·단어 수
- [ ] 휴지통 + 복원, 삭제 확인 `dialog`
- **완료 기준**: 마우스만으로 Ulysses의 라이브러리 주요 작업 10종 완료 가능, 1,000시트 트리 스크롤 60fps(`virtual_list`).

### M4 — 프리뷰 & 분할 뷰 (5~7일)
- [ ] `PreviewIr` → gpui 요소 렌더러: 제목(크기·여백), 문단, 리스트, 인용, 코드블록(하이라이트), 표, 이미지, 수평선, 링크
- [ ] 분할 토글(마크업/프리뷰/좌우 분할), 폭 리사이즈
- [ ] 스크롤 동기화(블록 anchor 매핑), 프리뷰 내 링크/이미지 클릭 동작
- [ ] 가상화: 블록 단위 lazy prepaint(긴 문서 성능)
- **완료 기준**: 20,000단어 + 이미지 50개 문서에서 프리뷰 스크롤 60fps, 코드블록 5개 언어 하이라이트, 표/리스트 중첩 정상.

### M5 — 쓰기 보조 기능 (4~6일)
- [ ] 통계: 단어/문자/공백 제외/문단/읽기 시간(상태바 + 인스펙터)
- [ ] 목표(goals): 시트·그룹 단위, 진행률 배지, 달성 알림(`notification`)
- [ ] 포커스 모드(비활성 문단 fade_out — decoration으로 구현), 타자기 모드(커서 중앙 고정)
- [ ] 아웃라인 패널(제목 트리) + 클릭 이동 + `Cmd+Shift+O` 헤딩 점프
- [ ] 키워드(`#tag`)·각주(`[^1]`)·내부 링크(시트 참조)를 InlineToken 칩으로, hover 미리보기
- **완료 기준**: Ulysses의 "글쓰기 세션" 시나리오(목표 설정 → 포커스 모드 → 통계 확인 → 헤딩 점프)를 키보드만으로 수행 가능.

### M6 — 검색·인덱스 (3~4일)
- [ ] SQLite FTS5 인덱스(증분 갱신), 제목/본문/키워드/날짜 필드
- [ ] 커맨드 팔레트(`command`) 퀵 오픈: 시트·그룹·액션 통합 검색, 퍼지 매칭
- [ ] 라이브러리 필터 바(그룹/키워드/날짜/즐겨찾기), 결과 하이라이트
- [ ] 스마트 그룹(저장된 쿼리)
- **완료 기준**: 5,000시트에서 검색 < 150ms, 팔레트 열기→이동 < 50ms.

### M7 — 내보내기 (5~7일)
- [ ] 공통: `PreviewIr` → 렌더러별 writer (Markdown 정규화, HTML+CSS, RTF)
- [ ] PDF: **typst** crate(폰트 임베딩·CJK 강점) 1순위, 대안 `printpdf`/HTML→시스템 인쇄
- [ ] DOCX: `docx-rs` 또는 OOXML 직접 생성(`zip`+`quick-xml`)
- [ ] ePub: `epub` 빌더 또는 직접 zip+XHTML(목차/표지/메타)
- [ ] 스타일 테마(폰트·여백·헤더/푸터·각주 처리) + 내보내기 프리뷰 다이얼로그
- **완료 기준**: 동일 시트에서 5개 포맷 산출물 검증(Ulysses 산출물과 목차/각주/이미지 배치 비교), 한글 PDF 임베딩 정상.

### M8 — 설정·테마·패키징 (3~5일)
- [ ] `setting` 기반 환경설정 페이지(테마/폰트/행간/편집 폭/자동 저장/키맵/언어)
- [ ] 커스텀 테마 JSON 로드(gpui-component theme registry), Ulysses풍 라이트/다크 2종 기본 제공
- [ ] i18n(한/영), 접근성(포커스 트랩·스크린리더 라벨), 온보딩(첫 실행 라이브러리 선택)
- [ ] CI: fmt/clippy/test + 플랫폼 빌드, 릴리스 번들(`cargo-bundle`/`.app`, `.exe` 설치본), 자동 업데이트 정책 결정
- **완료 기준**: 클린 머신에서 설치→라이브러리 생성→작성→내보내기까지 무설치 의존성으로 동작.

### M9 — 동기화·버전·가져오기 (P2, 5~8일)
- [ ] 로컬 버전 히스토리(스냅샷 + diff 뷰, `diff` 컴포넌트 재사용)
- [ ] WebDAV 동기화(백그라운드, ETag/mtime 기반, 충돌 시 3-way 머지 UI)
- [ ] Ulysses 라이브러리 가져오기(내보낸 패키지/폴더 → 시트+메타 변환)
- [ ] 통계 대시보드(`chart`/`plot`)
- **완료 기준**: 두 머신 WebDAV 동기화에서 충돌 10회 시나리오 전부 데이터 손실 없이 해결.

**총 예상: P0+P1 ≈ 33~47 작업일**, P2 포함 45~65일 (1인 집중 개발 + 에이전트 병행 기준).

---

## 5. 리스크 & 대응

| # | 리스크 | 확률/영향 | 대응 |
| --- | --- | --- | --- |
| R1 | 에디터 내 폰트 크기 변경 불가(§1.4) → Ulysses식 "리치 마크업" 기대치 미달 | 중/중 | Markup 모드를 1순위로 스코프 고정, 리치는 프리뷰에서. 필요 시 gpui-base 포크 비용 견적(M2 종료 시점 결정) |
| R2 | 인라인 이미지 렌더 미지원 | 중/중 | 스파이크 B로 조기 판정 → 칩 + 프리뷰 폴백(기능 자체는 유지) |
| R3 | decoration 재계산으로 타이핑 지연 | 중/높음 | 증분 파싱·가시 범위 제한·debounce·M2에서 벤치 게이트(16ms) 설정 |
| R4 | gpui-pre 스냅샷 업그레이드 시 breaking change | 높음/중 | lock 커밋, 업그레이드 전용 브랜치, 스모크 테스트 스위트(M0에서 구축) |
| R5 | PDF/DOCX 품질(특히 CJK, 각주, 표) | 중/중 | typst 우선 검증, 포맷별 골든 문서 비교, M7을 프리뷰 IR과 분리 |
| R6 | Windows GPUI 성숙도(IME, DPI, 폰트 폴백) | 중/중 | M0 스모크, 이슈 발견 시 macOS 우선 개발 + Windows는 M8 이후 집중 |
| R7 | 범위 팽창(동기화/협업 등) | 높음/높음 | P2는 M9 이후로 동결, feature-map 표의 우선순위를 유일한 기준으로 사용 |
| R8 | Ulysses 상표/디자인 카피 문제 | 낮음/중 | 독자 브랜딩·아이콘, "Ulysses 호환" 표기 금지, 파일 포맷 호환(사실 interoperability)만 제공 |

---

## 6. 검증 전략

1. **유닛(코어/마크다운)**: offset 매핑·frontmatter·저장 원자성·내보내기 writer. 골든 파일은 `tests/corpus/`(CommonMark spec 예제 + Ulysses 특유 구문: 키워드, 각주, 시트 참조, 첨부).
2. **UI 테스트**: `gpui_kit::test` + `test-support`로 헤드리스 렌더 → 액션 디스패치 → 상태/포커스 단정(시트 열기, 굵게 토글, 트리 이동).
3. **벤치 게이트**: `criterion` — 5k/20k 단어 문서 (a) 파싱, (b) decoration 빌드, (c) 프리뷰 IR 생성. M2/M4에서 임계값 초과 시 머지 금지.
4. **수동 QA 체크리스트**: 마일스톤별 `docs/qa/M{n}.md`에 시나리오(키보드 경로 포함) 작성 후 체크.
5. **시각 회귀**: 프리뷰/테마 스냅샷(렌더 캡처) 비교 — M4 이후 도입 판단.

---

## 7. 즉시 실행 (다음 3일)

- [ ] **D1**: 워크스페이스 전환 + feature 활성화 + `dock` 레이아웃 스켈레톤 + 한글 IME 스모크 (M0)
- [ ] **D2**: 스파이크 A/B — `EditorState`(markdown) + decoration POC, `InlineTokenRenderer` 이미지 확인 → `docs/spikes/001-editor-decorations.md` 작성, R1/R2 판정
- [ ] **D3**: `markdown` 파서 스켈레톤 + 골든 테스트 10종, `core` 라이브러리 스캔 + `Cmd+O/S` 루프 (M1 진입)

---

## 8. 오픈 퀘스천 (결정 필요)

| # | 질문 | 선택지 | 권장 |
| --- | --- | --- | --- |
| Q1 | 우선 타깃 플랫폼 | macOS / Windows / 둘 다 | 개발기 기준 **macOS 우선**, Windows는 M8에서 동등화 |
| Q2 | 저장 포맷 | 플레인 md+frontmatter / sidecar JSON / Ulysses 컨테이너 호환 | **플레인 md+frontmatter + SQLite 인덱스** (§3.3) |
| Q3 | 편집 모드 목표 | Markup 우선 / 완전 WYSIWYG(인라인 리치) | **Markup 우선 + 프리뷰 분할** (R1 제약) |
| Q4 | 동기화 | 없음 / 로컬+git / WebDAV | P1 없음 → **P2 WebDAV** |
| Q5 | 배포 | 개인용 / 오픈소스 / 상용 | 오픈소스(라이선스 MIT/Apache) 가정 — R8 정책 영향 |
| Q6 | 내보내기 우선순위 | PDF > DOCX > ePub? | **HTML/MD → PDF → DOCX → ePub** |

---

## 9. 의존성 후보

| 용도 | crate | 비고 |
| --- | --- | --- |
| 마크다운 파싱 | `pulldown-cmark` (offset 정확) / `comrak` (GFM) | offset 매핑이 핵심 → pulldown-cmark 우선 |
| 인덱스/검색 | `rusqlite`(bundled, FTS5) 또는 `tantivy` | 저장소 단순성 → rusqlite |
| 파일 감시/탐색 | `notify`, `walkdir`, `dunce`(Windows 경로) | |
| 직렬화/메타 | `serde`, `serde_json`, `serde_yaml`(or `serde_yml`), `time`/`chrono`, `uuid` | |
| 설정/경로 | `directories`, `toml` | |
| 내보내기 | `typst`(PDF), `docx`(DOCX), `epub`, `zip`, `quick-xml`, `html-escape`, `base64` | PDF 대안: `printpdf` |
| 이미지 | `image`, `fast_image_resize`(썸네일) | |
| 동기화 | `webdav-client` 또는 `reqwest`+`quick-xml` | P2 |
| 테스트/벤치 | `criterion`, `pretty_assertions`, `insta`(스냅샷) | gpui `test-support`는 dev-feature |

> 주의: gpui-kit 기본 feature에 `tree-sitter`가 꺼져 있으므로 마크다운 하이라이팅을 쓰려면 명시 활성화가 필요하다. 빌드 시간이 크게 늘면 "자체 스팬 하이라이팅만 사용하고 tree-sitter는 코드펜스 전용"으로 축소한다.
