# ADR-0007: 이미지 크기·정렬·전면 배치는 Pandoc 속성 문법으로 마크다운에 적는다

- 상태: Proposed
- 날짜: 2026-10-20
- 관련: PLAN.md M7(내보내기), `src/markdown/mod.rs`(`ImageOptions`, `image`), `src/app/insert_image.rs`

## 배경 (Context)

이미지 삽입 대화상자에서 렌더링 크기(가로·세로 px), 정렬(왼쪽·가운데·오른쪽), "한 페이지 가득", 캡션을 고를 수 있게 한다.

- CommonMark 이미지(`![alt](src "title")`)에는 크기·정렬을 적을 자리가 없다.
- 시트는 사용자 데이터인 일반 `.md` 파일이다. 다른 편집기와 내보내기 도구에서도 읽혀야 한다.
- 내보내기(ePub·DOCX·PDF, M7)가 이 값을 읽어야 한다.

## 결정 (Decision)

이미지 바로 뒤에 Pandoc `link_attributes` 문법으로 붙인다. 캡션은 alt 자리에 둔다.

```markdown
![캡션](Extras/Attachments/01J….png){width=320px height=200px .align-left}
![표지](cover.jpg){.full-page}
```

- 크기: `width=…px`, `height=…px`. 비우면(자동) 적지 않는다.
- 정렬: `.align-left`, `.align-right`. 기본인 가운데는 적지 않는다.
- 한 페이지 가득: `.full-page`. 이때 크기·정렬은 쓰지 않는다.
- 옵션이 모두 기본값이면 `{…}`를 붙이지 않는다.
- 이미지는 독립 문단으로 넣는다. 그래야 Pandoc `implicit_figures`가 alt를 캡션으로 쓴다.

## 결과 (Consequences)

- 좋아지는 점(+):
  - 표준에 가장 가까운 확장 문법이다. Pandoc으로 바로 내보낼 수 있고, 우리 내보내기도 같은 규칙을 따르면 된다.
  - 옵션이 없으면 순수 CommonMark 그대로다.
  - 텍스트라서 사람이 직접 고칠 수 있다.
- 감수하는 점(-):
  - markdown-rs(에디터·프리뷰 파서)는 `{…}`를 모른다. 이미지 뒤의 일반 텍스트로 본다.
  - GitHub 같은 CommonMark 렌더러에서는 `{…}`가 글자로 보인다.
- 후속 작업(TODO):
  - 에디터 마크업에서 이미지 뒤 `{…}`를 기호처럼 흐리게 표시한다.
  - 프리뷰(`TextView`)와 내보내기에서 속성을 읽어 크기·정렬·전면 배치를 적용한다.
  - 시트의 이미지를 다시 고칠 수 있게 한다(커서 위 이미지 → 같은 대화상자).

## 대안 (Alternatives)

| 대안 | 기각 사유 |
| --- | --- |
| HTML `<img width=… style=…>`/`<figure>` | 쓰기 화면에서 읽기 어렵다. 내보내기마다 HTML을 해석해야 한다. 캡션 구조도 따로 정해야 한다. |
| 이미지 제목(`"title"`)에 옵션 문자열 | 제목의 원래 뜻(툴팁)과 섞인다. 어떤 도구도 해석하지 않는다. |
| 시트 옆 메타 파일(`.writer.toml`)에 이미지별 옵션 | 마크다운만 옮기면 옵션이 사라진다. 이미지 순서가 바뀌면 짝이 어긋난다. |
