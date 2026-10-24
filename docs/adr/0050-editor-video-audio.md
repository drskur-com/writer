# ADR-0050: 동영상·오디오는 이미지 줄 문법으로 넣고, 에디터에서 재생하며, ePub에는 재생 요소로, PDF에는 포스터와 첨부파일로 내보낸다

- 상태: Proposed
- 날짜: 2026-10-24
- 관련: ADR-0007(이미지 속성), ADR-0008(이미지 줄 블록), ADR-0014(PDF), ADR-0041(메모 첨부파일) · `src/media.rs`, `src/app/media_player.rs`, `src/app/editor_image.rs`, `src/app/insert_image.rs`, `src/export/epub.rs`, `src/export/pdf.rs`, `src/export/docx.rs`, `src/app/book_preview/layout.rs`

## 배경 (Context)

시트에 동영상과 오디오를 넣고 싶다. 쓰는 화면에서 바로 재생하고, 내보낸 책에서도 가능한 한 재생되어야 한다.

- 시트는 일반 `.md`다. 다른 도구(Pandoc)에서도 읽혀야 한다.
- ePub 3은 `<video>`·`<audio>`를 지원한다(Apple Books·Thorium·Kobo가 재생, Kindle은 재생하지 않음).
- PDF 규격의 동영상(Screen·RichMedia 주석)은 사실상 Acrobat만 재생하고, krilla는 이 주석을 쓰지 못한다. 첨부파일(EmbeddedFile)은 쓴다.
- gpui에는 동영상 디코더가 없다. macOS gpui는 `CVPixelBuffer`(4:2:0 YCbCr)를 `paint_surface`로 GPU에 바로 그린다.
- rodio는 이미 확장 소리(ADR-0040)에 쓰고 있고, symphonia 디코더를 기능으로 켤 수 있다.

## 결정 (Decision)

**마크다운: 이미지 줄 문법을 그대로 쓰고 확장자로 가른다(Pandoc HTML 출력과 같은 규칙).**

```markdown
![설명](Attachments/clip.mp4){width=480px .align-left}
![주제가](Extras/Attachments/song.mp3)
```

- 동영상: `mp4`·`m4v`·`mov`. 오디오: `mp3`·`m4a`·`aac`·`wav`·`flac`·`ogg`·`oga`(`media::kind_of`).
- 파일은 이미지처럼 첨부파일 그룹에 복사한다. 웹 주소는 받지 않는다(삽입 대화상자에 URL 탭이 없다).
- 삽입 메뉴에 `동영상…`·`오디오…`를 두고 이미지 대화상자를 종류별로 쓴다. 오디오는 크기·한 페이지 가득을 고르지 않는다.

**에디터: 이미지 줄 블록 자리에 재생기를 그린다.**

- 동영상: 포스터(QuickLook 첫 장면) 위에 재생 단추와 조작 줄(재생·진행 막대·시간). 크기는 MP4 `moov`의 `tkhd`(회전 반영)에서, 길이는 `mvhd`에서 읽는다(그림 데이터는 읽지 않음).
  - macOS: AVPlayer가 풀고 소리를 내며 `AVPlayerItemVideoOutput`이 준 프레임을 `paint_surface`로 그린다. 재생 중에는 화면 프레임마다 다시 그린다.
  - 다른 OS: 포스터만 보인다.
- 오디오: 한 줄 카드(재생·제목·진행 막대·시간). rodio + symphonia로 모든 OS에서 재생한다. 출력 장치는 따로 띄운 스레드가 들고, 재생 중에는 250ms마다 다시 그린다.
- 재생기는 처음 재생할 때 만든다. 하나를 재생하면 같은 시트의 다른 재생기는 멈춘다. 단추·진행 막대는 눌러도 커서가 줄로 가지 않고, 두 번 누르면(블록 바깥) 대화상자로 고친다.

**내보내기**

- ePub: 파일을 `media/`에 넣고 `<video controls poster>`·`<audio controls>`로 둔다. 안쪽에는 재생하지 못하는 리더를 위한 포스터 그림(또는 캡션)을 둔다.
- PDF·미리보기: 동영상 자리에 포스터와 재생 표시를 그리고, 오디오는 `♪ 제목` 칸이다. 본문의 동영상·오디오 파일은 PDF 첨부파일로 넣는다.
- DOCX: 동영상은 포스터 그림, 오디오는 `♪ 제목` 문단이다.
- 포스터는 내보낼 때 QuickLook으로 만든다(`thumbnail::png`, 긴 변 1280px). macOS 밖에서는 없다.

## 결과 (Consequences)

- (+): 원문은 표준 마크다운 그대로이고, Pandoc으로 HTML을 만들면 같은 `<video>`·`<audio>`가 나온다.
- (+): 동영상 프레임은 복사·색 변환 없이 GPU로 간다. 보기만 하는 시트는 파일을 열지 않는다.
- (-): 동영상 재생은 macOS 전용이다. AVFoundation이 못 푸는 형식(WebM 등)은 받지 않는다.
- (-): 동영상 프레임은 모서리를 둥글게 자르지 않는다(`paint_surface`는 모서리를 모른다).
- (-): 줄을 지워도 그 재생기는 시트를 닫을 때까지 남는다(재생 중이면 소리가 이어진다).
- (-): PDF는 첨부 목록에서 열어야 재생된다. ePub도 Kindle 등에서는 포스터만 보인다.
- 후속: YouTube 같은 웹 동영상은 썸네일 + 링크 카드로 따로 다룬다(앱 안 재생은 웹뷰가 없어 하지 않는다).

## 대안 (Alternatives)

| 대안 | 기각 사유 |
| --- | --- |
| HTML `<video>`·`<audio>` 태그를 원문에 쓰기 | 쓰기 화면에서 읽기 어렵고 이미지와 속성 규칙이 갈린다 |
| 에디터는 포스터만, 재생은 시스템 앱 | 쓰는 화면에서 바로 확인할 수 없다 |
| 동영상 프레임을 RGBA로 바꿔 `RenderImage`로 그리기 | 프레임마다 복사·변환과 아틀라스 교체가 든다 |
| PDF에 RichMedia·Screen 주석 | Acrobat 밖에서는 재생되지 않고 krilla가 쓰지 못한다 |
| 오디오도 macOS AVPlayer로 | 다른 OS에서 재생할 수 없다. rodio가 이미 있다 |
