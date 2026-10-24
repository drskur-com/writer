# ADR-0051: YouTube 주소는 썸네일 카드로 보이고, 내보내기에서는 링크(ePub·PDF)와 Word 온라인 비디오(DOCX)로 바꾼다

- 상태: Proposed
- 날짜: 2026-10-24
- 관련: ADR-0050(동영상·오디오) · `src/media.rs`(`youtube_id`), `src/app/editor_image.rs`, `src/app/insert_image.rs`, `src/export/epub.rs`, `src/export/pdf.rs`, `src/export/docx.rs`, `src/app/book_preview/layout.rs`

## 배경 (Context)

시트에 YouTube 동영상을 넣고 싶다. YouTube를 앱 안에서 재생하는 공식 방법은 iframe 플레이어(웹뷰)뿐이고, 영상 파일 주소를 뽑아 재생하는 것은 약관 위반이다.

- gpui는 창을 GPU로 통째로 그린다. 에디터 위에 WKWebView 같은 네이티브 뷰를 겹치면 스크롤·잘림·겹침 순서를 손으로 맞춰야 하고, 메뉴·대화상자 위로 튀어나온다.
- ePub 리더 대부분은 iframe·외부 스크립트를 막는다. ePub 3은 이미지를 책 안에 두어야 한다.
- PDF는 링크 주석으로 주소를 열 수 있다(krilla 지원). Word는 "온라인 비디오"(`wp15:webVideoPr`)로 YouTube를 문서 안에서 재생한다.

## 결정 (Decision)

**원문은 이미지 줄 문법 그대로 YouTube 주소를 적는다**: `![예고편](https://youtu.be/<ID>){width=480px}`.

- `youtu.be/ID`, `youtube.com/watch?v=ID`, `/embed/`, `/shorts/`, `/live/`, `m.`·`music.`·`youtube-nocookie.com`을 받는다. ID는 11자만 받는다.
- 동영상 삽입 대화상자의 URL 탭은 YouTube 주소만 받고, 내려받지 않고 그대로 적는다.

**에디터: 웹뷰 없이 썸네일 카드**

- `i.ytimg.com/vi/<ID>/hqdefault.jpg`를 16:9 칸에 채워 그리고(위아래 띠는 잘린다) 가운데 재생 단추와 `YouTube` 표시를 둔다.
- 재생 단추를 누르면 기본 브라우저에서 연다. 앱 안 재생은 필요해지면 별도 창의 웹뷰로 더한다(에디터 안에는 겹치지 않는다). → ADR-0052에서 새 창 재생으로 바꿨다.

**내보내기**: 썸네일은 내보낼 때 내려받는다(`maxresdefault` → `mqdefault`). 받지 못하면 링크 글로 둔다.

- ePub: 썸네일을 책 안에 넣고 그림과 캡션에 YouTube 링크를 건다. 플레이어(iframe)는 넣지 않는다.
- PDF·미리보기: 썸네일과 재생 표시를 그리고, 그 칸에 YouTube로 가는 링크 주석을 단다.
- DOCX: 썸네일 그림을 온라인 비디오로 둔다(그림 속성에 하이퍼링크와 플레이어 iframe). Word에서 누르면 문서 안에서 재생된다.

## 결과 (Consequences)

- (+): 에디터 구조(GPU로 그리는 창)를 건드리지 않는다. 원문은 Pandoc에서도 이미지 링크로 읽힌다.
- (+): 내보낸 책은 어디서든 YouTube로 갈 수 있고, Word에서는 문서 안에서 재생된다.
- (-): 쓰는 화면에서는 재생하지 않는다(브라우저로 간다).
- (-): 썸네일은 네트워크가 있어야 보이고, 내보낼 때 기다린다. 오프라인이면 링크 글만 남는다.
- (-): Pandoc 같은 다른 도구는 YouTube 주소를 깨진 이미지로 본다.

## 대안 (Alternatives)

| 대안 | 기각 사유 |
| --- | --- |
| 에디터 줄 블록 위에 WKWebView를 겹치기 | 스크롤·잘림·겹침을 직접 맞춰야 하고 메뉴·대화상자를 가린다. 영상마다 웹뷰가 생긴다 |
| 영상 스트림을 뽑아 AVPlayer로 재생 | YouTube 약관 위반이고 자주 깨진다 |
| ePub에 iframe 플레이어 | 리더 대부분이 막는다 |
| 썸네일을 웹 주소 그대로 ePub에 둔다 | ePub 3은 그림을 책 안에 두어야 한다 |
