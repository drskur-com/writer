# gpui-base 패치

writer는 gpui-base 0.8.0을 `vendor/gpui-base`에 두고 `[patch.crates-io]`로 연결한다(ADR-0006).
에디터의 줄 스타일·줄 블록·접힌 줄·줄 오버레이·스크롤 보정이 이 패치다.

## 지금 상태: `gpui-base-0.8.0-writer.patch`

원본 0.8.0(`a/src`) → 지금 vendor(`b/src`)의 전체 차이다. 깨끗한 0.8.0에 `patch -p1`로 적용하면
vendor와 같아진다.

우리 코드는 되도록 **원본 파일 밖**에 둔다. 원본 파일에는 연결부(구조체 필드, 호출 몇 줄, `mod` 선언)만
남기고, 기능은 원본 파일의 자식 모듈(`#[path = "…_writer.rs"] mod writer;`)에 둔다. 자식 모듈은 부모의
비공개 필드에 접근할 수 있으므로 원본 코드를 고치지 않고 `impl`을 더할 수 있다.

| 원본 파일 | 우리 코드 |
| --- | --- |
| `input/base/element.rs` | `input/base/element_writer.rs`(줄 블록·오버레이 배치, 행 정렬), `input/base/tests/element_writer.rs`(테스트) |
| `input/base/state.rs` | `input/base/state_writer.rs`(입력·붙여넣기 변환, 접힌 줄 펼치기) |
| `input/editor/mod.rs` | `input/editor/writer.rs`(공개 API: `set_line_style`·`set_line_blocks`·`set_line_overlays`·`replace_range_keeping_selections`) |
| `input/editor/display_map/display_map.rs` | `…/display_map_writer.rs`(행 높이 기하) |
| `input/editor/display_map/text_wrapper.rs` | `…/text_wrapper_writer.rs`(`LineStyle` 등 타입, 줄 스타일·블록·펼치기) |
| `input/editor/display_map/wrap_map.rs` | `…/wrap_map_writer.rs` |
| `input/mod.rs` | `input/scroll_trace.rs`(임시 스크롤 추적, `WRITER_SCROLL_TRACE=1`) |

원본 파일 안에 남은 변경: `text_wrapper.rs`의 줄바꿈 클로저(줄별 글자 크기·첫 줄 들여쓰기),
`element.rs`의 레이아웃·그리기 연결부, `state.rs`의 필드와 붙여넣기·입력 변환 호출,
`overscroll.rs`·`scroll_bounce.rs`의 스크롤 보정(ADR 없음, 아래 두 패치).

## gpui-base를 올릴 때

1. 새 버전을 받아 `gpui-base-0.8.0-writer.patch`를 `patch -p1`로 적용한다. 새 파일(`*_writer.rs`)은 그대로
   들어가고, 충돌은 원본 파일의 연결부에서만 난다.
2. 충돌을 고친 뒤 `README.md`(저장소 루트, gpui-base 테스트의 `include_str!`가 읽는다)를 잠시 두고
   `cargo test --lib --features test-support`로 gpui-base 테스트를 돌린다.
3. 새 차이로 이 패치를 다시 만든다.

```sh
t=$(mktemp -d); ln -s <원본 gpui-base> $t/a; ln -s "$PWD/vendor/gpui-base" $t/b
(cd $t && diff -ruN a/src b/src) > docs/patches/gpui-base-<버전>-writer.patch
```

## 기능별 패치(기록)

아래 파일은 기능을 처음 넣을 때의 차이다. ADR이 가리키는 기록으로 남기며, 지금 코드와는 위치가 다르다
(위 구조로 옮기기 전). 업그레이드에는 쓰지 않는다.

| 패치 | ADR |
| --- | --- |
| `gpui-base-0.8.0-line-style.patch` | ADR-0006 줄 스타일(제목 확대) |
| `gpui-base-0.8.0-line-block.patch` | ADR-0008 줄 블록·접힌 줄 |
| `gpui-base-0.8.0-line-layout.patch`, `gpui-base-0.8.0-line-block-style.patch` | ADR-0010 줄 배치(정렬·양쪽 정렬·첫 줄 들여쓰기·줄 아래 여백) |
| `gpui-base-0.8.0-line-row.patch` | ADR-0016 줄 스타일의 행 번호 |
| `gpui-base-0.8.0-line-block-row.patch` | ADR-0017 표 격자 |
| `gpui-base-0.8.0-line-overlay.patch` | ADR-0023 줄 오버레이 |
| `gpui-base-0.8.0-overscroll-tolerance.patch`, `gpui-base-0.8.0-scroll-reverse.patch` | 스크롤 보정 |
