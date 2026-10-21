# ADR-0014: PDF는 미리보기 배치를 그대로 krilla로 찍는다(typst 대신)

- 상태: Proposed
- 날짜: 2026-10-21
- 관련: ADR-0013, PLAN.md M7(PDF: typst 1순위였음) · `src/export/pdf.rs`, `src/app/book_preview/pdf.rs`, `src/app/compile_dialog.rs`

## 배경 (Context)

PDF 내보내기가 필요하다. 미리보기(ADR-0013)는 이미 판형별 쪽 나눔·양쪽 정렬·제목 배치·차례·쪽 번호를 자체 엔진으로 배치한다.
PLAN은 typst를 1순위로 두었는데, 그러면 렌더링 기능마다 미리보기와 typst 양쪽에 구현하고 결과를 맞춰야 한다.

## 결정 (Decision)

**PDF는 미리보기가 배치한 페이지(글자 조각·이미지·선의 위치, 단위 pt)를 krilla로 그대로 찍는다. 미리보기 = PDF.**

- 배치는 미리보기 엔진(`layout_book`)이 UI 스레드에서 하고, 결과를 gpui 없는 IR(`export::pdf::PdfDocument`)로 옮긴 뒤 백그라운드에서 쓴다.
- 서체: 런의 서체 이름·굵기·기울임으로 시스템 서체를 찾는다(fontdb, 이름이 같아야 함). 못 찾으면(시스템 UI 서체 등) Apple SD Gothic Neo → Helvetica Neue → Malgun Gothic. krilla가 쓴 글자만 넣는다(subset).
- 서체에 없는 글자는 대체 서체(Apple SD Gothic Neo, Hiragino Sans, PingFang SC, Noto Sans CJK KR, Malgun Gothic, Arial Unicode MS 순)로 그린다. 조각 시작 x는 배치 그대로, 조각 안은 서체 글자 폭으로 잇는다.
- 기준선은 gpui와 같이 줄 칸 안에서 ascent + descent를 세로 가운데에 둔다.
- 이미지: PNG·JPEG·GIF·WebP는 그대로, 나머지는 PNG로 바꿔 넣는다. 표지는 판형을 채우고 넘치는 부분을 자른다. 웹 이미지는 넣지 않는다.
- 내보내는 곳: 미리보기 창 PDF 보기의 내보내기 버튼(보이는 판형·목차 그대로, 선택 범위 포함), 컴파일 대화상자의 PDF(판형·목차 생성 선택).

## 결과 (Consequences)

- (+): 렌더링 기능은 미리보기에 한 번만 넣으면 PDF에도 들어간다. 화면과 결과물이 같다.
- (+): 의존성이 작다(krilla + fontdb + ttf-parser). 글자는 선택·검색할 수 있는 텍스트로 들어간다.
- (-): 조판 품질은 미리보기 엔진 수준이다(한 줄씩 채우는 줄바꿈, 각주는 시트 끝, 머리말·홀짝 여백 없음). 필요해지면 엔진에 더한다.
- (-): 조각 안 글자 간격은 krilla 셰이핑(rustybuzz)이라 CoreText로 잰 미리보기와 아주 조금 다를 수 있다. 서체에 기울임·굵은 꼴이 없으면 가장 가까운 꼴을 쓴다(합성하지 않는다).
- (-): 배치가 UI 스레드에서 돌아 긴 원고는 내보낼 때 잠깐 멈출 수 있다(ADR-0013과 같음).

## 대안 (Alternatives)

| 대안 | 기각 사유 |
| --- | --- |
| typst로 별도 조판 | 기능마다 두 번 구현, 미리보기와 결과가 어긋남, 바이너리·빌드 크기 증가. 인쇄 조판 품질이 중요해지면 다시 검토 |
| printpdf | 서체 subset·CJK 처리를 직접 해야 한다 |
| 시스템 인쇄(WebKit/HTML → PDF) | 플랫폼 종속, 미리보기와 다른 엔진 |
