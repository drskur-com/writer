# ADR-0043: 확장 캔버스에 그림자·흐림·섞기와 글자 돌리기를 더하고, 효과는 모양 목록의 상태로 둔다

- 상태: Proposed
- 날짜: 2026-10-23
- 관련: ADR-0026(Lua 확장), ADR-0042(vello_cpu로 그리기) · `src/extension/canvas.rs`(`Effects`, `Blend`, `Shadow`), `src/extension/render.rs`, `src/extension/svg.rs`, `src/export/pdf.rs`, `src/export/docx.rs`, `resources/skills/extension/references/draw.md`

## 배경 (Context)

ADR-0042로 화면과 PNG가 vello_cpu로 그려지면서, GPUI 경로로는 못 하던 흐림·그림자·섞기와 글자 돌리기를 할 수 있게 됐다.
스킬은 지금까지 이것들이 없다고 적고 "반투명 사본을 여러 겹 그려라"고 권했다.

## 결정 (Decision)

**`g.setShadow(color[, blur, dx, dy])`, `g.setBlur(radius)`, `g.setBlendMode(mode)`/`g.getBlendMode()`를 더하고, 글자는 변환의
회전과 `g.print`의 `r`을 따른다. 효과는 바뀔 때마다 모양 목록에 `Shape::Effects`로 들어가 뒤 모양들에 걸린다(`Clip`과 같다).**

- 효과는 그때의 변환 배율을 따라 캔버스 좌표로 적는다(흐림·그림자 번짐은 가우스 표준편차). `push`/`pop`은 효과를 저장하지 않는다(LÖVE와 같다).
- 섞기: CSS·PDF의 16가지와 `"add"`(더하기). LÖVE의 `"alpha"`는 `"normal"`이다. `"subtract"`처럼 없는 이름은 목록과 함께 알린다.
- 그리기(`render`): 그림자가 있으면 모양을 먼저 그림자만(`DropShadowOnly`) 한 번, 흐림이 있으면 흐려서 한 번 더 그린다.
- 글자: `Shape::Text`에 `angle`(라디안, 시계 방향, `origin` 중심)을 둔다.
- 내보내기: SVG(ePub)는 `mix-blend-mode`와 `feGaussianBlur`·`feDropShadow` 필터, 글자는 `rotate`로 옮긴다. PDF는 섞기(더하기 빼고)와 돌린
  글자를 벡터로 옮기고, 흐림·그림자·더하기가 있는 그림은 통째로 4배 PNG로 넣는다. DOCX는 효과가 있으면 SVG 없이 PNG만 넣는다(Word는 SVG 효과를 그리지 않는다).

## 결과 (Consequences)

- (+): 카드 그림자, 빛·불꽃, 안개, 잉크 같은 표현을 모양 하나로 한다. 글자를 돌려 축 이름표·간판을 쓴다.
- (-): 흐림·그림자는 모양마다 필터를 돌려 비싸다(1200×800px에서 모양 하나에 약 0.2ms, 100개면 20~25ms). 스킬에 큰 모양 몇 개에만 쓰라고 적었다.
- (-): 흐림·그림자가 있는 그림은 PDF에서 벡터가 아니다. SVG 효과는 리더마다 다르게 보일 수 있다.
- (-): 섞기는 투명한 캔버스 위에서 시작하므로 배경을 먼저 그려야 보인다.
- 후속(TODO): 이미지, 3차원 깊이 버퍼.

## 대안 (Alternatives)

| 대안 | 기각 사유 |
| --- | --- |
| 모양마다 효과 필드를 둔다 | 모든 모양과 내보내기의 모든 갈래가 바뀐다. 상태 모양 하나면 모르는 쪽은 무시하면 된다 |
| 레이어(`push_filter_layer`) 단위로 효과를 건다 | 여러 모양을 한꺼번에 흐리는 데는 맞지만, LÖVE식 "상태를 바꾸고 그린다" 모델과 다르다 |
| PDF에서 흐림·그림자를 무시한다 | 화면과 결과가 크게 달라진다 |
