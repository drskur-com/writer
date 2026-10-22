# ADR-0026: 확장은 라이브러리의 Lua(Luau) 스크립트로 두고, 시트의 `` ```ext `` 블록에 캔버스로 그리거나 메뉴 명령을 더한다

- 상태: Proposed
- 날짜: 2026-10-23
- 관련: ADR-0008(줄 블록), ADR-0017(표: 접힌 줄 + 아래 블록), `src/extension/`, `src/app/editor_extension.rs`, `src/app/extension_commands.rs`, `src/agent/tools.rs`(`run_extension`), `resources/skills/extension/SKILL.md`

## 배경 (Context)

간단한 기능을 앱을 고치지 않고 더하고 싶다. 예를 들면 시트 안에 그림이나 애니메이션(궁수가 활을 쏜다)을
넣거나, 오늘 날짜를 넣는 명령 같은 것이다. 코드는 사용자가 아니라 어시스턴트가 스킬을 따라 쓴다.

- 외부 런타임(Node, Python)에 기대지 않고 앱에 넣을 수 있어야 한다.
- 모델이 쓴 코드가 한 번에 맞지 않아도 스스로 실행해 보고 고칠 수 있어야 한다.
- 남이 쓴 코드가 될 수 있으니 파일·네트워크에 닿지 못하고, 무한 루프에도 앱이 멈추지 않아야 한다.
- 나중에 그림을 PDF·EPUB·DOCX에도 넣을 수 있어야 한다.

## 결정 (Decision)

**확장은 `Extensions/<이름>.lua` 하나이고, mlua의 Luau sandbox에서 돈다. 파일은 표를 돌려주고, 앱은 그 안의
진입점만 쓴다.**

- 진입점(앱이 정한 자리, 내용은 확장이 정한다):
  - `draw(g, t, args)`: 시트의 `` ```ext <이름> `` 펜스(본문은 `key: value` 인자) 자리에 그린다. `size = {w, h}`
    캔버스, `animated = true`면 보이는 동안 매 프레임 부른다(`t`는 초).
  - `commands = { { title, run(ctx) } }`: `Extensions` 메뉴의 명령. 돌려준 글로 선택 영역을 바꾼다(없으면 커서 자리).
- 캔버스 `g`는 LÖVE `love.graphics`의 이름·인자를 따른다(`setColor`, `rectangle`, `circle`, `arc`, `line`,
  `polygon`, `print`, `printf`, `push`/`pop`/`translate`/`rotate`/`scale` 등). 호출은 **모양 목록**(경로·글자,
  캔버스 좌표, 원·호는 3차 베지어)으로 모으고, 에디터는 GPUI `canvas`에서 `paint_path`·`shape_line`으로 그린다.
  없는 함수를 부르면 쓸 수 있는 목록을 알린다.
- 에디터: 펜스 줄은 표처럼 접고(`collapsed`·`reveal_run`) 닫는 줄 아래 줄 블록에 캔버스를 그린다. 본문 폭보다
  크면 비율을 지켜 줄인다. 읽기·그리기 오류는 블록 자리에 보인다. 확장 파일이 바뀌면 다시 읽는다.
- sandbox: Luau 기본(`io` 없음, `os`는 시간 함수뿐) + `require` 제거 + `sandbox(true)`. 메모리 64MB, 시간은 읽기 1초·
  그리기 0.1초·명령 2초(interrupt로 끊는다). `math.random`은 그릴 때마다 같은 seed로 되돌린다.
- 어시스턴트: 빌트인 스킬 `extension`(API·규칙)과 `run_extension` 도구(그리기 결과의 모양 수·범위, 명령 결과,
  오류)를 준다. 파일은 기존 `write_file`·`edit_file`(승인 diff)로 쓴다. 라이브러리를 열 때 `Extensions/`를 만든다.

## 결과 (Consequences)

- (+): 의존성은 mlua(Luau 소스를 정적으로 빌드) 하나다. 외부 런타임이 없다.
- (+): 모양 목록은 출력 형식과 상관없어서, 같은 확장을 PDF(krilla 경로)·EPUB(SVG)·DOCX(SVG+PNG)로 옮길 수 있다.
- (+): 펜스라서 다른 마크다운 앱에서는 코드 블록으로 보이고 원고가 깨지지 않는다.
- (-): Lua 문자열은 바이트라 한글을 자르거나 셀 때 `utf8.*`를 써야 한다(스킬에 적었다).
- (-): 캔버스는 LÖVE의 일부뿐이다(이미지·셰이더·입력 없음). 글자는 돌리지 않는다.
- (-): 애니메이션 블록이 보이면 창이 매 프레임 다시 그려진다.
- 후속(TODO): 내보내기(PDF·EPUB·DOCX, `t = 0` 또는 `still:` 인자의 프레임), 미리보기 창, 인라인 값(`{{today}}`처럼
  볼 때마다 계산), 블록 클릭·입력, 확장 설정 화면.

## 대안 (Alternatives)

| 대안 | 기각 사유 |
| --- | --- |
| JS(rquickjs) | 문자열·정규식은 낫지만, 캔버스 API를 흉내 낼 기준(LÖVE)이 Lua 쪽에 있고 sandbox·시간 제한은 Luau가 더 간단하다 |
| Rhai, Lisp(Steel·Janet) | 모델이 잘 모르는 언어라 실패가 잦다 |
| WebAssembly(wasmtime·wasmi) | 확장마다 컴파일러가 필요하다. 배포용 확장 생태계에 맞고, 모델이 바로 쓰는 스크립트에는 무겁다 |
| 확장이 SVG 문자열을 돌려준다 | 매 프레임 SVG를 파싱·래스터화해 아틀라스에 올려야 한다. 그리는 코드를 직접 쓰는 캔버스보다 덜 자연스럽다 |
