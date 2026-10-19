# ADR-0004: 아이콘 팩은 Lucide 전체 카탈로그(AllAssets)를 쓰고 매핑을 한 곳에 둔다

- 상태: Accepted
- 날짜: 2026-10-20
- 관련: `src/icons.rs`, `src/main.rs` · gpui-kit-assets

## 배경 (Context)

사이드바(Inbox/Notes/Projects/그룹/시트)는 이모지(📥/📝/📚/📁)를 썼는데, 폰트 의존적이고
플랫폼마다 크기·색이 달라 UI 와 어울리지 않았다. 벡터 아이콘이 필요하다.

- gpui-component 는 자체 `IconName` 호환 enum 을 두는데, **기본 번들(106개)만** 담는다
  (`NotebookPen`, `Library` 등은 없음).
- `gpui-kit-assets` 는 **전체 Lucide 카탈로그**(`gpui_kit_assets::IconName`)와
  에셋 소스 `Assets`(기본 106개) / `AllAssets`(전체) 를 제공한다.
- 아이콘은 `AssetSource` 로 로드되므로, 쓰려는 아이콘이 에셋 소스에 포함돼야 한다(ADR-0002).

## 결정 (Decision)

**아이콘 팩은 Lucide 로 하고, 전체 카탈로그(`gpui_kit::assets::IconName`)를 쓰기 위해
에셋 소스를 `AllAssets` 로 전환한다. 화면 코드는 `src/icons.rs` 의 매핑 함수만 호출한다.**

- 에셋: `.with_assets(gpui_kit::assets::AllAssets)`
- 타입: `gpui_kit::assets::IconName` (컴포넌트 호환 enum 아님)
- 매핑 (`src/icons.rs`): `section(SectionKind)` → Inbox/NotebookPen/BookOpen,
  `group()` → Folder, `sheet()` → FileText, `PACK = "Lucide"`
- 도메인(`library.rs`)은 이모지/아이콘을 모른다 — 표현은 `icons.rs` 로 분리.

## 결과 (Consequences)

- (+): 시맨틱 아이콘을 자유롭게 선택(→ 향후 툴바/버튼에도 재사용).
- (+): 아이콘 교체는 `icons.rs` 만 수정. 뷰는 아이콘 이름에 직접 의존하지 않는다.
- (+): Lucide 는 일관된 스트로크/크기 → 이모지보다 룩앤필이 균일.
- (-): `AllAssets` 는 전체 Lucide 를 바이너리에 포함해 크기/빌드가 약간 증가.
- (-): Lucide 아이콘 이름 명명 규칙에 종속(없는 이름은 컴파일 에러로 조기 발견).
- 후속: 필요하면 `AllAssets` → 사용 아이콘만 담는 `icon_assets!` 선택 번들로 축소.

## 대안 (Alternatives)

| 대안 | 기각 사유 |
| --- | --- |
| 이모지 유지 | 폰트 의존·크기/색 불균일, UI 불일치 |
| 컴포넌트 `IconName`(기본 106개) | NotebookPen/Library 등 시맨틱 아이콘 부재 |
| 커스텀 SVG 직접 제작 | 제작/유지 비용, Lucide 로 충분 |
| `AllAssets` 대신 필요한 것만 개별 임베드 | 초기엔 과함 — 아이콘 목록이 안정되면 재검토 |