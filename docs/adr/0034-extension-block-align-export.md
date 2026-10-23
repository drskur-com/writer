# ADR-0034: 확장 블록의 자리(`align:`)와 내보내기 제외(`export:`)는 앱이 읽는 예약 인자로 두고, 확장은 내보내기 기본값을 정한다

- 상태: Proposed
- 날짜: 2026-10-23
- 관련: ADR-0026(Lua 확장, `scale:`), ADR-0033(블록 편집 대화상자) · `src/extension/mod.rs`(`RESERVED_ARGS`, `align_of`, `export_arg`, `still`), `src/app/editor_extension.rs`, `src/app/book_preview/layout.rs`, `src/export/{epub,docx}.rs`, `src/app/insert_extension.rs`

## 배경 (Context)

확장 블록은 에디터·책 미리보기·PDF·ePub·DOCX에서 늘 가운데에 그려지고 늘 내보내진다. 그림을 본문 왼쪽이나 오른쪽에 두고
싶을 때가 있고, 버튼·게임처럼 종이에 의미가 없는 블록은 내보내기에서 빼고 싶다. 배율은 이미 앱이 읽는 예약 인자 `scale:`이다.

## 결정 (Decision)

**`scale:` 옆에 예약 인자 `align:`(`left`·`center`·`right`, 기본 가운데)과 `export:`(`true`·`false`)를 둔다. 확장은 반환 표의
`export`(기본 `true`)로 기본값을 정하고, 블록의 `export:`가 이긴다.**

- 예약 인자는 확장에 넘기지 않는다(`without_reserved`). `run_extension`은 확장이 이 이름을 인자로 쓰면 알린다.
- `align:`: 에디터 캔버스, 책 미리보기·PDF(판형 칸 안), ePub(`figure.align-*`, 이미지와 같은 클래스), DOCX(`w:jc`)에 같이 쓴다.
- `export:`: `extension::still`이 `Ok(None)`을 돌려주면 그 블록은 미리보기·내보내기에서 아무것도 내지 않는다(코드 블록으로도 쓰지
  않는다). 블록이 `export: false`면 확장 파일을 읽지 않고 뺀다(확장이 없어도 코드가 새지 않는다). `export:`가 없거나 `true`면 확장의
  `export`를 따르거나 넣고, 그리지 못하면 지금처럼 코드 블록이다. 에디터에는 늘 보인다.
- 편집 대화상자: 자리(왼쪽·가운데·오른쪽)와 내보내기(확장 기본·넣기·빼기)를 고르는 칸을 둔다. 기본값(가운데, 확장 기본)은 블록에
  적지 않는다.

## 결과 (Consequences)

- (+): 모든 확장이 따로 구현하지 않아도 자리와 내보내기를 고를 수 있다.
- (+): 확장이 자기 성격에 맞는 기본값(게임은 빼기)을 정하고, 블록마다 바꿀 수 있다.
- (-): 예약 인자가 늘어 확장이 그 이름을 인자로 쓸 수 없다.
- (-): 빼는 블록은 내보낸 원고에서 흔적이 없다(자리 표시를 남기지 않는다).
