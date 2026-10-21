# 율리시즈(Ulysses) 에디터 포팅 계획

> 작성일: 2026-10-19 · 개정: 2026-10-20 (현행 코드·ADR 반영, 프리뷰 전략 수정) · 상태: 초안(승인 대기)
> 관련 문서: [`feature-map.md`](./feature-map.md) (기능 인벤토리/추적 표) · [`adr/`](./adr/README.md) (확정된 결정)
>
> 이 문서와 ADR이 충돌하면 **ADR이 우선**한다. 결정이 바뀌면 ADR을 새로 쓰고 이 문서를 맞춘다.

---

## 0. 한 줄 요약

**gpui-kit 기반 Rust 데스크톱 앱에, Ulysses의 "마크다운 시트 + 라이브러리 + 쓰기 보조 + 내보내기" 경험을 이식한다.**
편집 엔진(gpui-base의 Rope/DisplayMap/Decorations)과 마크다운 프리뷰(`TextView::markdown`)가 이미 스택 안에 있으므로,
신규 개발은 ① 문서·라이브러리 모델, ② 에디터 마크업 데코레이션 계층, ③ 쓰기 보조 UX, ④ 내보내기/동기화에 집중한다.

---

## 1. 현황 진단

### 1.1 저장소 상태

| 항목 | 내용 |
| --- | --- |
| 크레이트 | 단일 bin crate `writer` (edition 2024), 의존성 `gpui-kit = "0.7"`, `dirs`, `serde`, `toml` |
| 코드 | `app/`(UI: 커스텀 타이틀바·2열 사이드바·시트 목록·온보딩), `menus.rs`(앱 메뉴), `library/`(폴더 검증/초기화/스캔, 그룹 메타, 미리보기), `watch.rs`(폴더 감시), `config.rs`(OS 설정 디렉터리 `config.toml`), `theme.rs`(JSON 테마), `icons.rs`(Lucide 매핑). **에디터·시트 열기/저장은 아직 없음** |
| 결정 기록 | ADR-0001 테마 JSON 단일 출처 · 0002 커스텀 창 크롬 · 0003 라이브러리 레이아웃/폴더 UX · 0004 Lucide 아이콘 팩 |
| 빌드 | `cargo check` 통과 (콜드 약 1m35s) |
| 도구 | `mise.toml`에 check/dev/watch/build-fast/clean-cache 태스크, `cargo-watch` |

### 1.2 스택이 **이미 제공하는 것** (직접 만들지 말 것)

`gpui-kit 0.7.3` → `gpui(gpui-pre 0.3.9, Zed gpui 스냅샷 rev 93d7456)` + `gpui-base 0.7.3` + `gpui-component 0.7.3`

| 계층 | 재사용 가능 자산 | Ulysses 대응 기능 |
| --- | --- | --- |
| 텍스트 엔진 | `ropey::Rope`, `InputBaseState<EditorMode>`, `DisplayMap`(wrap_map/fold_map/inline_line), `UndoManager`, 멀티 셀렉션(`add_selection`), IME/네이티브 입력, `SearchSession`/`SearchMatcher` | 시트 편집, 소프트랩, 되돌리기, 다중 커서, 문서 내 검색 |
| 스타일/장식 | `TextDecorationCollection`(HighlightStyle), `RangeDecorationCollection`(Fill/Frame), `set_editor_style`, `set_editor_paddings` | 마크업 구문 시각화(`#` 흐리게, **굵게**, 밑줄, 하이라이트) |
| 하이라이팅 | tree-sitter `InputHighlighter`, 내장 언어에 **markdown / markdown_inline** 포함 (feature `tree-sitter-markdown*`) | 코드펜스/인라인 마크업 기본 색상 |
| 마크다운 프리뷰 | `text::TextView::markdown` — 내부 파서 `markdown`(markdown-rs 1.0, 위치 포함 mdast), `markdown_extensions`/`markdown_mdx`, 커스텀 `markdown_block_parser`/`markdown_block_renderer`, `image_source`, `on_link_click`, `code_block_highlighter`, `selectable`/`scrollable`, frontmatter 처리 | 리치 프리뷰 팬, 키워드/각주/시트 링크 커스텀 블록 |
| 인라인 커스텀 렌더 | `InlineToken`/`InlineTokenSpan`/`InputContent` + `InlineTokenRenderer`(click/hover 이벤트, `available_width`) | 키워드 칩, 각주 마커, 링크 배지, 첨부 파일 토큰 |
| 에디터 요소 | `component::input::Editor`(상태 기반), `Textarea`, `Input`, 컨텍스트 메뉴 훅, 붙여넣기 가로채기(`on_paste` → 이미지/파일) | 본문 에디터, 이미지 붙여넣기 |
| 레이아웃/위젯 | `dock`(탭·분할 패널), `tab`, `tree`, `list`/`virtual_list`, `searchable_list`, `resizable`, `sidebar`, `title_bar`, `toolbar`, `status_bar`, `command`(커맨드 팔레트), `menu`(popup/context/dropdown/app_menu_bar), `dialog`, `notification`, `popover`, `tooltip`, `kbd`, `badge`, `tag`, `attachment`, `setting`(설정 UI), `theme`(커스텀 테마 레지스트리 + JSON 스키마) | 라이브러리 사이드바, 탭/분할, 퀵 오픈, 우클릭 메뉴, 환경설정 |
| 유틸 | `gpui::actions!` + keymap 디스패치, `history`, `theme`, focus/scroll 유틸 | 단축키 체계 |

### 1.3 **직접 만들어야 하는 것**

1. **라이브러리/문서 도메인** — 그룹(폴더)·시트(문서) 트리, 메타데이터, 첨부, 휴지통, 자동 저장, 파일 감시 (폴더 검증/초기화/스캔·그룹 메타는 구현됨)
2. **에디터 마크업 데코레이션 계층** — 원문 offset → mdast(markdown-rs, 프리뷰와 **동일 파서**) → 에디터 decoration 세트
3. **프리뷰 확장** — 렌더러는 `TextView`를 쓰고, Ulysses 고유 구문(키워드·각주·시트 참조·첨부)만 커스텀 블록 파서/렌더러로 추가. 스크롤 동기화·긴 문서 성능은 스파이크로 확인
4. **쓰기 보조 UX** — 단어/문자/읽기 시간 통계, 목표(goals), 포커스 모드, 타자기 모드, 아웃라인 내비게이션, 스마트 그룹
5. **검색·인덱스** — 라이브러리 전체 FTS, 필터, 최근 문서
6. **내보내기** — HTML/Markdown/RTF/PDF/DOCX/ePub + 스타일 테마
7. **동기화·버전** — WebDAV, 로컬 버전 히스토리, 충돌 해결
8. **설정/테마/키맵/패키징** — 환경설정 저장, 커스텀 테마, 설치본

### 1.4 조사로 확인한 **제약 사항** (설계에 반영)

| 제약 | 근거 | 영향 & 대응 |
| --- | --- | --- |
| `HighlightStyle`·`TextRun`에 글자 크기가 없고, 에디터는 줄 하나를 `shape_line(font_size)` 한 번으로 그리며 행 높이는 `row * line_height` 고정 격자 | `gpui-pre/src/style.rs:580`, `gpui-base/src/input/base/element.rs` | **제목만 줄 단위로 확대**: gpui-base를 vendor 패치해 `LineStyle { text_scale, height_scale }`(줄별 글자·행 높이 배율)을 추가하고, ATX `#`~`######`를 줄 텍스트로 즉시 판정한다([ADR-0006](./adr/0006-heading-line-style-gpui-base-patch.md)). setext 제목과 한 줄 안 크기 혼합은 범위 외 |
| 라인 높이가 에디터 단위(상대값)로 균일 | `input/editor.rs`의 private 상수 `EDITOR_LINE_HEIGHT = 1.5` | 블록 간 여백(문단/제목 spacing)은 에디터에서 표현 불가 → 프리뷰에서 해결 |
| 인라인 이미지 블록(변하는 행 높이) 지원 불명확 | `InlineTokenRenderer`에 `available_width`만 노출 | **M2에서 1일 스파이크 필수**. 안 되면 이미지는 "칩 + 프리뷰 렌더"로 대체 |
| gpui-pre는 Zed 스냅샷(버전 변동성 큼) | `Cargo.toml` 메타데이터 | `Cargo.lock` 커밋 유지, 업그레이드는 별도 브랜치 + 회귀 체크리스트 |
| Windows 플랫폼/IME/한중일 폰트 렌더링 검증 필요 | `gpui-pre-windows` 존재하나 미검증 | M0에서 한글 입력·폰트 폴백 스모크 테스트 |
| `TextView`의 스크롤 위치 API·블록 anchor·대용량 문서 성능 불명확 | 공개 API에 스크롤 동기화 훅 미확인 | **M4 진입 전 스파이크 C**. 부족하면 TextView 위에 블록 단위 래퍼 또는 자체 렌더러(예산 재산정) |
| 에디터(데코레이션)와 프리뷰가 다른 파서를 쓰면 해석 불일치 | TextView는 markdown-rs 사용 | 에디터도 markdown-rs mdast `position`을 사용. 별도 파서 도입 금지 |
| PDF/DOCX/ePub 파이프라인은 전부 신규 | — | M7에 예산 집중 |

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

### 3.1 코드 구조 (단계적 분리)

**현재는 단일 crate를 유지**하고, 모듈 경계만 지킨다. 워크스페이스 분리는 `markdown` 모듈이 생기고
gpui 없는 테스트/벤치가 필요해지는 시점(M2 후반)에 한다.

```
writer/
├─ Cargo.toml
├─ src/
│  ├─ main.rs                 # 앱 진입: 액션 정의, 테마/메뉴/단축키 등록, 창 열기
│  ├─ menus.rs                # 앱 메뉴 트리
│  ├─ app/                    # 앱 상태(WriterApp) + 화면별 렌더
│  │  ├─ mod.rs               #   상태, 액션 핸들러, 선택 조회, 루트 렌더
│  │  ├─ library_ops.rs       #   라이브러리 열기·감시·스캔 반영·미리보기·시트 생성/이동
│  │  ├─ style.rs             #   레이아웃 치수, Palette, 툴바 버튼 헬퍼
│  │  └─ library_panel.rs, sheet_list.rs, editor.rs, onboarding.rs
│  ├─ library/                # 도메인 (gpui 의존 금지)
│  │  ├─ mod.rs               #   모델, 스캔, 정렬
│  │  ├─ layout.rs            #   폴더 검증/초기화/레이아웃 보장
│  │  ├─ meta.rs              #   그룹 .writer.toml 읽기/쓰기, 시트 생성/순서
│  │  └─ preview.rs           #   본문 앞부분 → 제목/발췌
│  ├─ watch.rs                # 라이브러리 폴더 감시(notify)
│  ├─ config.rs               # 머신 로컬 설정 (gpui 의존 금지)
│  ├─ theme.rs, icons.rs, assets.rs  # UI 자산 매핑
│  └─ (예정) markdown/{spans, decorations, export/…}, ui/{library, editor, preview, outline, palette}
├─ themes/                    # 테마 JSON (ADR-0001)
├─ docs/                      # PLAN, feature-map, adr/, spikes/, benchmark/
└─ tests/corpus/              # (예정) 마크다운 골든 샘플
```

**분리 시 목표 형태**: `crates/{core, markdown, app}`, 의존 방향 `app → markdown → core`.

**원칙**
- `library`/`config`/`markdown`(파싱·스팬 계산)은 gpui 타입을 쓰지 않는다 → 분리 비용을 0에 가깝게 유지.
- 데코레이션 계산 결과는 UI 중립 IR(`StyleSpec`)로 내고, `HighlightStyle` 변환은 UI 쪽 얇은 어댑터에서.
- 앱 상태는 "엔티티 그래프"로: `Workspace`(root) → `LibraryView` / `DockView` → `SheetEditor`(시트별 `Entity<EditorState>` + 도메인 `Entity<Sheet>`) / `PreviewView`.

### 3.2 데이터 모델

현재 구현(`src/library.rs`)을 기준으로 하고, `(예정)` 필드는 해당 마일스톤에서 추가한다.

```
Library   { root, sections: Vec<Section> }                       # (예정) trash, index
Section   { kind: Notes|Projects, entries: Vec<Entry> }          # 고정 2섹션
Entry     = Sheet | Group
Group     { name, path, icon, sheets }                           # 메타는 <Group>/.writer.toml
            (예정) title, 하위 그룹, kind: Normal|Smart(query), collapsed
Sheet     { path(.md), modified }                         # 스캔은 메타데이터만
Preview   { title, excerpt }                              # 보이는 목록의 시트만 본문 앞 4KB에서 읽음
            (예정) keywords[], goals, created/opened_at, word_count, favorite, attachment_dir
Attachment (예정) { sheet, kind: Image|File|Note, path, caption, alt }
Config    { library_root, recent_libraries, … }                  # <config_dir>/Writer/config.toml
            (예정) theme, fonts, line_spacing, editor_width, autosave_ms, keymap_overrides
```

- **Inbox는 섹션이 아니라 Notes 아래 기본 그룹**(`DEFAULT_INBOX`)이다.
- 정렬: 기본은 최근 수정순(시간 역순). 그룹의 `.writer.toml`에 `sheets` 순서가 있으면 그것이 우선하고, 나열되지 않은 파일은 뒤에 최근 수정순으로 붙는다.
- 시트 파일명: 새 시트는 `<ULID>.md`. 파일명은 바뀌지 않는 식별자이고 표시 제목은 본문에서 얻는다(빈 시트는 "빈 시트").

### 3.3 저장 포맷 및 디렉터리 레이아웃 (ADR-0003, ADR-0005)

```
<LibraryRoot>/
├─ .writer/
│  ├─ library.toml              # 라이브러리 마커 (초기화 시 가장 마지막에 기록)
│  └─ index.sqlite              # (예정, M6) FTS5 + 메타 캐시 — 재생성 가능
├─ Notes/
│  ├─ Inbox/                    # 기본 그룹 (빠른 캡처)
│  │  ├─ .writer.toml
│  │  └─ Idea.md
│  └─ <Group>/…
└─ Projects/
   └─ <Project>/
      ├─ .writer.toml           # icon, title, sheets(순서)
      ├─ 01_Chapter.md
      └─ 01_Chapter.assets/     # (예정) 첨부 리소스
```

메타데이터는 세 곳에 두고, 기준은 **"누가 소유하고, 동기화돼야 하는가"**로 정한다.

| 위치 | 담는 것 | 성격 |
| --- | --- | --- |
| 시트 본문 YAML frontmatter (예정) | 시트 고유 메타: keywords, goal | 사용자 데이터, 시트와 함께 이동 |
| `<Group>/.writer.toml` | 그룹 메타: icon, title, 시트 순서. 프로젝트는 `[book]`(책 정보: 제목·지은이·언어·소개·표지·식별자, 내보내기용) | 사용자 데이터, 폴더와 함께 이동·동기화 |
| `<Library>/.writer/` | 마커, 인덱스, 캐시 | 재생성 가능, 지워도 데이터 손실 없음 |

> 그룹별 `.writer.toml`(커밋 `7ddb9e6`)은 ADR-0003의 "순서 등 파생 데이터는 `.writer/`에" 결정을 바꾼 것으로, ADR-0005에 기록했다.

- **본문 포맷**: 항상 **표준 마크다운**(`md`)으로 유지(다른 앱 호환 및 git 친화).
- **첨부물**: `시트이름.assets/` 폴더에 저장하고 상대 경로(`![](./시트이름.assets/img.png)`)로 참조.
  프로젝트 단위 첨부(표지 등)는 추가자료 그룹 `<Project>/Extras/Attachments/`(표시 이름 "첨부파일")에 두고 프로젝트 기준 상대 경로로 참조. 표지는 `cover.<확장자>`로 고정, 나머지는 `<ULID>.<확장자>`.
- **영역별 UX 특화**:
  - **Notes(메모)**: 최근 수정순 자동 정렬, 빠른 캡처(Inbox), 태그 중심 탐색.
  - **Projects(프로젝트)**: `.writer.toml`의 `sheets` 순서(드래그 정렬이 이 목록을 갱신), 전체 목표 단어 수(Goal), 다중 시트 일괄 병합/내보내기.

### 3.4 에디터 렌더링 파이프라인

```
Rope(원문)
  └─> [markdown] markdown-rs 파서 (mdast + position, TextView와 동일 파서·동일 옵션)
        ├─> BlockSpan[]  (heading/paragraph/list/quote/code/table/hr/image)
        └─> InlineSpan[] (strong/em/strike/code/link/keyword/footnote/marker)
              ├─(a) DecorationMapper → TextDecorationCollection / RangeDecorationCollection
              │      · 마커(`#`, `*`, `[]()`) fade_out
              │      · 제목 줄: ATX 레벨별 글자·행 높이 배율(gpui-base 패치 `set_line_style`, 줄 텍스트로 동기 판정)
              │      · strong=bold, em=italic, code=mono+bg, link=underline+색, quote=색+좌측 RangeDecoration(Fill)
              │      · 현재 커서 줄/블록은 마커를 원래 색으로 복원(Ulysses 동작)
              ├─(b) 프리뷰: TextView::markdown(원문) + 커스텀 블록 파서/렌더러(키워드·각주·시트 참조)
              └─(c) 내보내기: 같은 mdast → HTML/MD/DOCX/ePub writer
```

- **성능 규칙 (두 경로로 분리)**:
  - **동기 경로** (매 키 입력/커서 이동, 예산 < 16ms): 직전 파싱 결과에 편집 delta만큼 offset 보정 + 커서 블록 마커 복원.
  - **지연 경로** (150ms debounce): 전체(또는 변경 블록 범위) 재파싱 → diff → `collection.set()`, 가시 블록 우선.
- **하이라이팅 우선순위**: 자체 스팬 > tree-sitter(코드펜스 내부 언어용) > 기본 스타일.
- markdown-rs는 증분 파싱이 없으므로, 5k/20k 단어 전체 파싱 시간을 M2 스파이크에서 측정하고 블록 단위 재파싱 필요 여부를 정한다.

### 3.5 액션/키맵

`gpui::actions!`로 네임스페이스 분리: `app::editor`(Bold/Italic/Link/Heading1..6/List/Quote/Code/Keyword/Footnote), `app::library`(NewSheet/NewGroup/Rename/Move/Trash), `app::view`(TogglePreview/FocusMode/TypewriterMode/ToggleSidebar), `app::export`(Html/Pdf/Docx/Epub), `app::nav`(QuickOpen/CommandPalette/GotoHeading). macOS 기본 키맵 우선, `default.json` + 사용자 오버라이드.

---

## 4. 마일스톤

각 마일스톤은 "완료 기준(acceptance)"을 통과해야 다음으로 간다. 스파이크 결과는 `docs/spikes/`에 기록하고,
채택이 확정되면 ADR로 승격한다(`adr/README.md` 규칙).

### M0 — 기반 정리 (✅ 대부분 완료, 잔여 1일)
- [x] 커스텀 타이틀바·앱 메뉴·JSON 테마 시스템 (ADR-0001, 0002)
- [x] 사이드바(Notes/Projects 섹션, 접기, 아이콘, 패널 토글, 2열 그룹/시트 선택) (ADR-0004)
- [ ] `gpui-kit` feature 활성화: `tree-sitter`, `tree-sitter-markdown`, `tree-sitter-markdown-inline` (+ 빌드 시간 측정)
- [ ] `mise` 태스크 보강: `test`, `clippy`, `fmt`, `run-release`
- [ ] 한글 IME·CJK 폰트 렌더 스모크 테스트(macOS 우선, Windows)
- ~~Cargo 워크스페이스 전환~~ → §3.1에 따라 M2 후반으로 연기
- **완료 기준**: 사이드바/캔버스가 리사이즈·토글되고, 한글 입력·복붙이 깨지지 않으며, `cargo test`/`clippy`가 통과한다.

### M1 — 문서 모델 & 저장소 (2~3일)
- [x] 폴더 검증/초기화/스캔, 섹션·그룹·시트 모델, 그룹 메타(`.writer.toml`), 설정 저장 (ADR-0003)
- [x] 시트 열기/저장: 원자적 저장(temp+rename) — `library/document.rs`
- [ ] frontmatter 파싱(keywords/goal)
- [x] 자동 저장(1초 debounce + 포커스 아웃 + 시트 전환·창 닫기·종료 시 flush), dirty 상태 추적
- [x] 파일 시스템 감시(`notify`) → 외부 수정 시 reload(저장 안 한 변경이 없을 때)
- [ ] 외부 수정 + 저장 안 한 변경 충돌 경고
- [ ] `Cmd/Ctrl+S`, `O`, `N` 최소 루프 연결
- **완료 기준**: 폴더를 열면 시트 목록이 뜨고, 편집→자동 저장→앱 재실행 후 상태(커서 제외)가 복원된다. **5,000개 시트 스캔 < 300ms**(1,000개 < 100ms).

### M2 — 에디터 코어 & 마크업 스타일링 (5~7일) ★핵심
- [x] **스파이크 A(1일)**: `EditorState` + markdown-rs mdast `position` → `TextDecorationCollection`로 `# 제목`/`**굵게**`/`[링크]()` 시각화 POC. 5k/20k 단어 전체 파싱 시간 측정 → release 9ms/35ms, 지연 경로는 백그라운드 파싱
- [ ] **스파이크 B(1일)**: `InlineTokenRenderer`로 이미지/칩 인라인 렌더 가능 여부 → 불가 시 칩 대체안 확정
- [ ] `markdown` 모듈: mdast → `BlockSpan/InlineSpan` 변환(offset = Rope 바이트/문자 단위 정합) + 골든 테스트(corpus)
- [ ] DecorationMapper(커서 블록 마커 복원, quote/list 좌측 레일, 코드펜스 배경)
- [ ] 소프트랩/래핑 인덴트, 에디터 패딩·최대 폭(가독 폭 중앙 정렬), 스크롤 여백
- [ ] 포맷 액션 + 키맵(굵게/기울임/제목/리스트/인용/코드/링크), 선택 토글 동작
- [ ] 붙여넣기 가로채기(이미지 → 첨부 저장 후 마크다운 삽입)
- [x] **제목 줄 확대 패치**: gpui-kit/gpui-base 0.8 → `vendor/gpui-base` + `[patch.crates-io]` → `set_line_style(|line| LineStyle { text_scale, height_scale })`, wrapper 줄별 크기, 줄별 행 높이(`DisplayMap::display_row_top`/`display_row_at_y`) → ADR-0006, 패치 `docs/patches/`
- [ ] (M2 후반) gpui 비의존 모듈을 `crates/{core, markdown}`로 분리 — 벤치/테스트가 gpui 없이 돌 때
- **완료 기준**: 5,000단어 문서에서 키 입력→화면 반영 < 16ms(동기 경로), 지연 경로 재계산 < 50ms. 스팬 offset 골든 테스트(CommonMark 대표 예제 + Ulysses 구문 코퍼스) 통과. Ulysses 스크린샷 대비 마크업 색상 표 완성.

### M3 — 라이브러리 UI (4~6일)
- [ ] 사이드바 확장: 고정 섹션(Notes, Projects) + 하위 그룹 중첩, 드래그 이동/정렬(→ `.writer.toml` `sheets` 갱신), 펼침 상태 저장
- [x] 시트 정렬: `.writer.toml` 순서 → 최근 수정순
- [ ] 컨텍스트 메뉴(새 시트/그룹, 이름 바꾸기, 이동, 복제, 즐겨찾기, 휴지통), 인라인 rename
- [ ] `dock` 탭: 다중 시트, 닫기/순서/더티 표시, 최근 문서
- [ ] 상태바: 시트 경로·저장 상태·단어 수
- [ ] 휴지통 + 복원, 삭제 확인 `dialog`
- **완료 기준**: 마우스만으로 Ulysses의 라이브러리 주요 작업 10종 완료 가능, 1,000시트 트리 스크롤 60fps(`virtual_list`).

### M4 — 프리뷰 & 분할 뷰 (3~5일, 스파이크 C 결과에 따라 재산정)
- [ ] **스파이크 C(1일, M4 진입 전)**: `TextView::markdown`으로 20k 단어 + 이미지 50개 렌더 성능, 스크롤 위치 읽기/쓰기·블록 anchor 가능 여부, 커스텀 블록 파서/렌더러로 키워드 칩 1종 구현 → `docs/spikes/`
- [ ] `TextView` 연동: 테마 스타일(제목 크기·여백), `image_source`(첨부 상대 경로 해석), `on_link_click`(시트 참조 점프), `code_block_highlighter`
- [ ] Ulysses 구문 커스텀 블록: 키워드, 각주, 시트 참조, 첨부
- [ ] 분할 토글(마크업/프리뷰/좌우 분할), 폭 리사이즈
- [ ] 스크롤 동기화(블록 anchor 매핑) — TextView API가 부족하면 블록 단위 래퍼로 대응
- **완료 기준**: 20,000단어 + 이미지 50개 문서에서 프리뷰 스크롤 60fps, 코드블록 5개 언어 하이라이트, 표/리스트 중첩 정상, 에디터와 프리뷰의 블록 해석 일치(동일 코퍼스).
- **폴백**: 스파이크 C에서 성능/스크롤 동기화가 불가로 판정되면 자체 렌더러로 전환하고 M4를 5~7일로 되돌린다.

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
- [ ] 공통: mdast(+ Ulysses 구문 확장 노드) → 렌더러별 writer (Markdown 정규화, HTML+CSS, RTF)
- [x] PDF: 미리보기 배치를 krilla로 그대로 찍는다(ADR-0014, typst 대신)
- [ ] DOCX: `docx-rs` 또는 OOXML 직접 생성(`zip`+`quick-xml`)
- [x] ePub: 직접 zip+XHTML(EPUB 3 nav + EPUB 2 ncx 목차 / 표지 / 메타 / 책 스타일 CSS)
- [ ] 스타일 테마(폰트·여백·헤더/푸터·각주 처리) + 내보내기 프리뷰 다이얼로그
- **완료 기준**: 동일 시트에서 5개 포맷 산출물 검증(Ulysses 산출물과 목차/각주/이미지 배치 비교), 한글 PDF 임베딩 정상.

### M8 — 설정·테마·패키징 (3~5일)
- [ ] `setting` 기반 환경설정 페이지(테마/폰트/행간/편집 폭/자동 저장/키맵/언어)
- [ ] 커스텀 테마 JSON 로드(gpui-component theme registry), Ulysses풍 라이트/다크 2종 기본 제공
- [ ] i18n(한/영), 접근성(포커스 트랩·스크린리더 라벨), 온보딩(첫 실행 라이브러리 선택)
- [ ] CI: fmt/clippy/test + 플랫폼 빌드, 릴리스 번들(`cargo-bundle`/`.app`, `.exe` 설치본), 자동 업데이트 정책 결정
- **완료 기준**: 클린 머신에서 설치→라이브러리 생성→작성→내보내기까지 추가 설치 없이 동작(PDF용 CJK 폰트 번들 포함, 설치본 크기 예산 명시).

### M9 — 동기화·버전·가져오기 (P2, 5~8일)
- [ ] 로컬 버전 히스토리(스냅샷 + diff 뷰, `diff` 컴포넌트 재사용)
- [ ] WebDAV 동기화(백그라운드, ETag/mtime 기반, 충돌 시 3-way 머지 UI)
- [ ] Ulysses 라이브러리 가져오기(내보낸 패키지/폴더 → 시트+메타 변환)
- [ ] 통계 대시보드(`chart`/`plot`)
- **완료 기준**: 두 머신 WebDAV 동기화에서 충돌 10회 시나리오 전부 데이터 손실 없이 해결.

**총 예상(잔여): P0+P1 ≈ 30~43 작업일**, P2 포함 42~61일 (1인 집중 개발 + 에이전트 병행 기준).
M0 대부분·M1 일부 완료와 프리뷰의 TextView 재사용을 반영했다. 스파이크 C가 실패하면 +2일.

---

## 5. 리스크 & 대응

| # | 리스크 | 확률/영향 | 대응 |
| --- | --- | --- | --- |
| R1 | 에디터 내 폰트 크기 변경 불가(§1.4) → Ulysses식 "리치 마크업" 기대치 미달 | 중/중 | Markup 모드를 1순위로 스코프 고정, 리치는 프리뷰에서. 필요 시 gpui-base 포크 비용 견적(M2 종료 시점 결정) |
| R2 | 인라인 이미지 렌더 미지원 | 중/중 | 스파이크 B로 조기 판정 → 칩 + 프리뷰 폴백(기능 자체는 유지) |
| R3 | decoration 재계산으로 타이핑 지연 | 중/높음 | 동기/지연 경로 분리(§3.4), 가시 범위 제한, 블록 단위 재파싱, M2 벤치 게이트(16ms) |
| R3a | TextView가 스크롤 동기화·긴 문서 성능을 못 받침 | 중/중 | 스파이크 C로 M4 진입 전 판정 → 블록 래퍼 또는 자체 렌더러 폴백 |
| R3b | 에디터/프리뷰/내보내기 간 마크다운 해석 불일치 | 중/중 | markdown-rs 단일 파서·동일 옵션 강제, 공용 코퍼스로 세 경로 교차 검증 |
| R4 | gpui-pre 스냅샷 업그레이드 시 breaking change | 높음/중 | lock 커밋, 업그레이드 전용 브랜치, 스모크 테스트 스위트(M0에서 구축) |
| R5 | PDF/DOCX 품질(특히 CJK, 각주, 표), typst 도입 시 바이너리·폰트 번들 크기 수십 MB 증가 | 중/중 | typst 우선 검증 + 크기 측정, 포맷별 골든 문서 비교, 크기가 과하면 HTML→시스템 인쇄 폴백 |
| R6 | Windows GPUI 성숙도(IME, DPI, 폰트 폴백) | 중/중 | M0 스모크, 이슈 발견 시 macOS 우선 개발 + Windows는 M8 이후 집중 |
| R7 | 범위 팽창(동기화/협업 등) | 높음/높음 | P2는 M9 이후로 동결, feature-map 표의 우선순위를 유일한 기준으로 사용 |
| R8 | Ulysses 상표/디자인 카피 문제 | 낮음/중 | 독자 브랜딩·아이콘, "Ulysses 호환" 표기 금지, 파일 포맷 호환(사실 interoperability)만 제공 |

---

## 6. 검증 전략

1. **유닛(코어/마크다운)**: 라이브러리 검증/스캔(기존 `library.rs` 테스트 확장), offset 매핑·frontmatter·저장 원자성·내보내기 writer. 골든 파일은 `tests/corpus/`(CommonMark 대표 예제 + Ulysses 특유 구문: 키워드, 각주, 시트 참조, 첨부). 파서 자체 정합성은 markdown-rs에 맡기고, 우리 테스트는 **스팬 offset과 데코레이션 결과**를 검증한다.
2. **UI 테스트**: `gpui_kit::test` + `test-support`로 헤드리스 렌더 → 액션 디스패치 → 상태/포커스 단정(시트 열기, 굵게 토글, 트리 이동).
3. **벤치 게이트**: `criterion` — 5k/20k 단어 문서 (a) 파싱, (b) decoration 빌드(동기/지연 경로 각각), (c) 라이브러리 스캔 1k/5k 시트. M1/M2에서 임계값 초과 시 머지 금지.
4. **수동 QA 체크리스트**: 마일스톤별 `docs/qa/M{n}.md`에 시나리오(키보드 경로 포함) 작성 후 체크.
5. **시각 회귀**: 프리뷰/테마 스냅샷(렌더 캡처) 비교 — M4 이후 도입 판단.

---

## 7. 즉시 실행 (다음 3일)

- [x] **D0**: ADR-0005 "그룹 메타는 그룹 폴더의 `.writer.toml`에 둔다" 작성, ADR-0003 상태 갱신, 메타 3계층(§3.3) 확정
- [ ] **D1**: 시트 열기/원자적 저장 + `Cmd+O/S/N` 루프, 에디터 팬에 `EditorState` 연결, 한글 IME 스모크 (M0 잔여 + M1)
- [ ] **D2**: 스파이크 A/B — markdown-rs mdast → decoration POC + 파싱 시간 측정, `InlineTokenRenderer` 이미지 확인 → `docs/spikes/001-editor-decorations.md`, R1/R2/R3 판정
- [ ] **D3**: 스파이크 C — `TextView::markdown` 프리뷰 성능·스크롤 API·커스텀 블록 확인 → `docs/spikes/002-preview-textview.md`, R3a 판정

---

## 8. 오픈 퀘스천 (결정 필요)

| # | 질문 | 선택지 | 권장 |
| --- | --- | --- | --- |
| Q1 | 우선 타깃 플랫폼 | macOS / Windows / 둘 다 | 개발기 기준 **macOS 우선**, Windows는 M8에서 동등화 |
| Q2 | 저장 포맷 | — | ✅ **결정됨**: 플레인 md + frontmatter(시트) + `.writer.toml`(그룹) + `.writer/`(캐시) — ADR-0003, ADR-0005 |
| Q3 | 편집 모드 목표 | Markup 우선 / 완전 WYSIWYG(인라인 리치) | **Markup 우선 + 프리뷰 분할** (R1 제약) |
| Q4 | 동기화 | 없음 / 로컬+git / WebDAV | P1 없음 → **P2 WebDAV** |
| Q5 | 배포 | 개인용 / 오픈소스 / 상용 | 오픈소스(라이선스 MIT/Apache) 가정 — R8 정책 영향 |
| Q6 | 내보내기 우선순위 | PDF > DOCX > ePub? | **HTML/MD → PDF → DOCX → ePub** |

---

## 9. 의존성 후보

| 용도 | crate | 비고 |
| --- | --- | --- |
| 마크다운 파싱 | `markdown` (markdown-rs 1.0, gpui-component가 이미 사용) | 프리뷰와 동일 파서로 해석 일치. 버전은 gpui-component와 맞춘다. `pulldown-cmark`/`comrak`은 도입하지 않음 |
| 인덱스/검색 | `rusqlite`(bundled, FTS5) 또는 `tantivy` | 저장소 단순성 → rusqlite |
| 파일 감시/탐색 | `notify`, `walkdir`, `dunce`(Windows 경로) | |
| 직렬화/메타 | `serde`✅, `toml`✅, `serde_json`(dev✅), frontmatter YAML은 gpui-component `text::frontmatter` 재사용 검토 후 `serde_yml`, `time`/`chrono` | |
| 설정/경로 | `dirs`✅ | |
| 내보내기 | `typst`(PDF), `docx-rs`(DOCX), `epub-builder`(ePub), `zip`, `quick-xml`, `html-escape`, `base64` | PDF 대안: `printpdf` |
| 이미지 | `image`, `fast_image_resize`(썸네일) | |
| 동기화 | `webdav-client` 또는 `reqwest`+`quick-xml` | P2 |
| 테스트/벤치 | `criterion`, `pretty_assertions`, `insta`(스냅샷) | gpui `test-support`는 dev-feature |

> 주의: gpui-kit 기본 feature에 `tree-sitter`가 꺼져 있으므로 마크다운 하이라이팅을 쓰려면 명시 활성화가 필요하다. 빌드 시간이 크게 늘면 "자체 스팬 하이라이팅만 사용하고 tree-sitter는 코드펜스 전용"으로 축소한다.
