# ADR-0039: 큰 확장은 `Extensions/<이름>/init.lua`와 모듈 폴더로 두고, `require`는 그 폴더 안만 부른다

- 상태: Proposed
- 날짜: 2026-10-23
- 관련: ADR-0026(Lua 확장, sandbox), ADR-0029(확장 초안), ADR-0030(검토), ADR-0038(플레이 창) · `src/extension/package.rs`(`Package`, `install_require`, `read_folder`, `modified`), `src/extension/mod.rs`(`read_source`, `scan`, `path`, `location`), `src/agent/tools.rs`(`draft_key`, `extension_source`, `take_drafts`, `Target::Files`), `resources/skills/extension/references/modules.md`

## 배경 (Context)

확장은 파일 하나(`Extensions/<이름>.lua`)이고 sandbox에서 `require`를 뺐다(ADR-0026). 레벨이 있는 게임, 도우미를 나눠 쓰는
차트 묶음처럼 큰 확장은 한 파일이 수백 줄을 넘어 모델이 고치기도(`edit_file`의 `old`가 길어진다) 사람이 읽기도 어렵다.
확장은 주로 어시스턴트가 쓰므로, 여러 파일을 초안·실행·검토·승인하는 흐름까지 함께 있어야 한다.

## 결정 (Decision)

**확장은 파일 하나 또는 폴더다. 폴더는 `init.lua`가 진입 파일(앞머리·반환 표)이고, 다른 `.lua`는 `require("ui.hud")`처럼
점으로 이은 이름의 모듈이다. 앱이 폴더의 소스를 모두 읽어 두고 `require`는 그 안에서만 찾는다. 어시스턴트의 변경은 확장
단위로 묶어 한 번 승인받는다.**

- 소스: `Package { main, modules }`. 모듈 이름은 폴더 안 경로(`ui/hud.lua` → `ui.hud`, `ui/init.lua` → `ui`), 경로 조각은 확장
  이름 규칙. 숨김·심볼릭 링크·`.lua`가 아닌 파일은 무시한다. 상한: 모듈 64개, 파일 512KB, 전체 2MB, 깊이 4.
- `require`: Rust 함수를 sandbox 전에 전역으로 둔다. 처음 부를 때 모듈을 실행해 돌려준 값(없으면 `true`)을 다시 쓰고, 서로
  부르면 `circular require: a -> b -> a`. 모듈 오류는 `<확장>/<경로>`를 가리킨다. 파일 하나 확장의 `require`는 폴더로 바꾸라고
  알린다. 메모리·시간 제한은 확장 전체에 같다.
- 같은 이름의 파일과 폴더가 함께 있으면 읽지 않고 알린다. 목록(`scan`)은 폴더를 세고, 바뀐 때는 폴더 안 파일·폴더 중 가장
  늦은 수정 시각이다(모듈만 고쳐도 다시 그린다). 확장 관리의 지우기·Finder 보기는 폴더째다.
- 어시스턴트:
  - `Extensions/<이름>/` 안의 진입 파일·모듈 쓰기는 초안이다. 폴더가 없어도 되고 승인하면 만든다. 폴더 확장 옆에 같은 이름의
    `.lua`는 쓰지 못한다.
  - `read_file`·`list_files`·`run_extension`은 디스크 위에 초안을 덮어 본다. 검토는 모든 파일을 합친 소스로 비교한다.
  - 답이 끝나면 확장마다 변경 하나(`Target::Files`): 승인 카드는 파일마다 머리줄(`LineKind::File`)과 diff다. 쓰기 전에 모든 파일이
    그대로인지 보고 하나라도 바뀌었으면 쓰지 않는다. 파일 하나 확장을 폴더로 바꾸면 옛 `<이름>.lua`는 휴지통으로 옮긴다(카드에
    지운 줄로 보인다).
- 스킬: `references/modules.md`(언제 나누는지, 모듈 모양, 쓰기·시험).

## 결과 (Consequences)

- (+): sandbox는 그대로다. Lua는 여전히 파일을 읽지 못하고, 다른 확장도 부르지 못한다.
- (+): 블록·인라인·플레이·명령·내보내기가 모두 같은 `Package`를 써서 따로 손볼 것이 없다.
- (+): 큰 확장도 파일 단위로 고치고, 사용자는 확장 단위로 한 번 승인한다.
- (-): 모듈을 지우는 도구가 없다(파일 하나 → 폴더 전환의 옛 파일만 지운다). 쓰지 않는 모듈은 남는다.
- (-): 승인 도중 일부 파일만 써지고 실패할 수 있다(원자적이지 않다). 그사이 바뀐 파일이 있으면 아무것도 쓰지 않는 것까지만 지킨다.
- (-): 폴더가 바뀌었는지 보려고 폴더를 훑는다(편집 중 확장 파일 검사마다). 상한(깊이 4, 64개)으로 비용을 묶는다.
