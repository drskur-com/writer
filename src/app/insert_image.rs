//! 이미지 삽입 대화상자: 파일·첨부파일·URL에서 이미지를 고르고 크기·정렬·캡션을 정해
//! 열린 시트의 커서 자리에 `![캡션](경로){속성}`을 넣는다(속성 문법은 ADR-0007).
//!
//! 프로젝트 시트에 고른 파일은 첨부파일 그룹에 복사하고 시트 기준 상대 경로로 잇는다.
//! 이미 첨부파일 그룹에 있는 이미지는 "첨부파일" 탭에서 골라 복사 없이 다시 쓴다.
//! 프로젝트 밖(메모)에는 첨부파일 그룹이 없어서 원본 파일의 절대 경로를 쓴다.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, FocusableExt as _, Icon, Sizable as _, WindowExt as _,
};
use gpui_kit::gpui::prelude::FluentBuilder as _;
use gpui_kit::gpui::*;
use std::cell::Cell;
use std::path::{Path, PathBuf};

use super::WriterApp;
use super::new_project::form_row;
use super::style::*;
use crate::markdown::{ImageAlign, ImageOptions};
use crate::{InsertImage, icons, library, markdown};

/// 대화상자 폭.
const DIALOG_WIDTH: f32 = 520.0;

/// 파일을 놓는 자리와 첨부파일 목록의 높이. 탭을 바꿔도 대화상자 크기가 덜 출렁이게 같게 둔다.
const SOURCE_HEIGHT: f32 = 180.0;

/// 첨부파일 목록의 썸네일 한 칸 크기.
const THUMBNAIL_SIZE: f32 = 96.0;

/// 정렬 버튼 순서.
const ALIGNS: [(ImageAlign, &str); 3] = [
    (ImageAlign::Left, "왼쪽"),
    (ImageAlign::Center, "가운데"),
    (ImageAlign::Right, "오른쪽"),
];

/// 이미지를 어디서 가져오는지.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Source {
    File,
    Attachment,
    Url,
}

/// 넣을 이미지.
enum ImageSource {
    File(PathBuf),
    Url(String),
}

/// 대화상자 안의 입력 상태.
struct InsertImageForm {
    sheet: PathBuf,
    source: Source,
    /// 보여 줄 탭. 프로젝트 시트에만 "첨부파일" 탭이 있다.
    tabs: Vec<Source>,
    file: Option<PathBuf>,
    /// 고른 파일의 수정 시각(썸네일 캐시 키).
    file_modified: u128,
    /// 파일을 고르다 생긴 문제(지원하지 않는 형식 등).
    file_error: Option<SharedString>,
    /// 프로젝트 첨부파일 그룹의 이미지(프로젝트 밖이면 비어 있다)와 그중 고른 것.
    attachments: Vec<library::FileItem>,
    attachment: Option<PathBuf>,
    url: Entity<InputState>,
    width: Entity<InputState>,
    height: Entity<InputState>,
    /// 가로세로 비율을 지킨다. 원본 크기를 알 때만 따른다.
    keep_ratio: bool,
    align: ImageAlign,
    full_page: bool,
    caption: Entity<InputState>,
    app: WeakEntity<WriterApp>,
    _subscriptions: Vec<Subscription>,
}

/// 숫자만 받는 크기 입력 칸. 비우면 자동.
fn size_input(window: &mut Window, cx: &mut App) -> Entity<InputState> {
    cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder("자동")
            .validate(|text, _| text.chars().all(|c| c.is_ascii_digit()))
    })
}

/// 크기 입력 칸의 값. 비었거나 0이면 자동(`None`).
fn size_value(input: &Entity<InputState>, cx: &App) -> Option<u32> {
    input
        .read(cx)
        .value()
        .parse()
        .ok()
        .filter(|&value| value > 0)
}

impl InsertImageForm {
    fn new(
        app: WeakEntity<WriterApp>,
        sheet: PathBuf,
        attachments: Option<Vec<library::FileItem>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let url = cx.new(|cx| InputState::new(window, cx).placeholder("https://"));
        let width = size_input(window, cx);
        let height = size_input(window, cx);
        let caption =
            cx.new(|cx| InputState::new(window, cx).placeholder("이미지 아래에 표시할 설명(선택)"));
        let mut subscriptions: Vec<Subscription> = [&url, &caption]
            .into_iter()
            .map(|input| {
                // 입력 칸에서 Enter를 누르면 바로 넣는다.
                cx.subscribe_in(input, window, |this, _, event, window, cx| {
                    if let InputEvent::PressEnter { .. } = event {
                        this.submit(window, cx);
                    }
                })
            })
            .collect();
        // 비율을 지키면 한쪽을 고칠 때 다른 쪽을 맞춘다. `set_value`는 이벤트를 내지 않아 되돌아오지 않는다.
        for (changed, other, horizontal) in [(&width, &height, true), (&height, &width, false)] {
            let other = other.clone();
            subscriptions.push(cx.subscribe_in(
                changed,
                window,
                move |this, input, event, window, cx| {
                    if let InputEvent::Change = event {
                        this.sync_ratio(input, &other, horizontal, window, cx);
                    }
                },
            ));
        }
        Self {
            sheet,
            source: Source::File,
            tabs: match attachments {
                Some(_) => vec![Source::File, Source::Attachment, Source::Url],
                None => vec![Source::File, Source::Url],
            },
            file: None,
            file_modified: 0,
            file_error: None,
            attachments: attachments.unwrap_or_default(),
            attachment: None,
            url,
            width,
            height,
            keep_ratio: true,
            align: ImageAlign::default(),
            full_page: false,
            caption,
            app,
            _subscriptions: subscriptions,
        }
    }

    /// 지금 고른 로컬 이미지 파일(파일·첨부파일 탭).
    fn local_path(&self) -> Option<&PathBuf> {
        match self.source {
            Source::File => self.file.as_ref(),
            Source::Attachment => self.attachment.as_ref(),
            Source::Url => None,
        }
    }

    /// 고른 이미지의 원본 크기(px). URL이거나 읽지 못하면 `None`.
    fn natural_size(&self) -> Option<(u32, u32)> {
        let size = imagesize::size(self.local_path()?).ok()?;
        let (width, height) = (
            u32::try_from(size.width).ok()?,
            u32::try_from(size.height).ok()?,
        );
        (width > 0 && height > 0).then_some((width, height))
    }

    /// 크기 한쪽(`changed`)이 바뀌면 원본 비율대로 다른 쪽(`other`)을 채운다. 비우면 같이 비운다.
    fn sync_ratio(
        &mut self,
        changed: &Entity<InputState>,
        other: &Entity<InputState>,
        horizontal: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.keep_ratio {
            return;
        }
        let Some((width, height)) = self.natural_size() else {
            return;
        };
        let value = match size_value(changed, cx) {
            Some(value) if horizontal => {
                (u64::from(value) * u64::from(height) + u64::from(width) / 2) / u64::from(width)
            }
            Some(value) => {
                (u64::from(value) * u64::from(width) + u64::from(height) / 2) / u64::from(height)
            }
            None => {
                other.update(cx, |state, cx| state.set_value("", window, cx));
                return;
            }
        };
        other.update(cx, |state, cx| {
            state.set_value(value.max(1).to_string(), window, cx)
        });
    }

    /// 다른 이미지를 고르면 비율을 지키는 중인 크기를 새 이미지에 맞춘다.
    fn refit_size(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let width = self.width.clone();
        let height = self.height.clone();
        if size_value(&width, cx).is_some() {
            self.sync_ratio(&width, &height, true, window, cx);
        } else if size_value(&height, cx).is_some() {
            self.sync_ratio(&height, &width, false, window, cx);
        }
    }

    /// 운영체제 파일 선택기로 이미지를 고른다.
    fn pick_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("이미지 선택".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = receiver.await else {
                return;
            };
            this.update_in(cx, |this, window, cx| this.use_file(&paths, window, cx))
                .ok();
        })
        .detach();
    }

    /// 고른(끌어다 놓은) 파일 중 첫 이미지를 쓴다. 이미지가 없으면 안내를 띄운다.
    fn use_file(&mut self, paths: &[PathBuf], window: &mut Window, cx: &mut Context<Self>) {
        let image = paths.iter().find(|path| {
            path.extension()
                .map(|ext| ext.to_string_lossy().to_lowercase())
                .is_some_and(|ext| library::COVER_EXTENSIONS.contains(&ext.as_str()))
        });
        match image {
            Some(path) => {
                self.file = Some(path.clone());
                self.file_modified = library::modified_nanos(path);
                self.file_error = None;
                self.refit_size(window, cx);
            }
            None => {
                self.file_error = Some("JPEG·PNG·GIF·WebP·SVG 이미지만 넣을 수 있습니다.".into());
            }
        }
        cx.notify();
    }

    /// 지금 탭에서 고른 이미지. 아직 없으면 `None`.
    fn image(&self, cx: &App) -> Option<ImageSource> {
        match self.source {
            Source::Url => {
                let url = self.url.read(cx).value().trim().to_string();
                (!url.is_empty()).then_some(ImageSource::Url(url))
            }
            _ => self.local_path().cloned().map(ImageSource::File),
        }
    }

    fn options(&self, cx: &App) -> ImageOptions {
        ImageOptions {
            width: size_value(&self.width, cx),
            height: size_value(&self.height, cx),
            align: self.align,
            full_page: self.full_page,
        }
    }

    /// 이미지를 넣고 대화상자를 닫는다. 실패하면 알림을 띄우고 열어 둔다.
    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(image) = self.image(cx) else {
            return;
        };
        let caption = self.caption.read(cx).value().to_string();
        let options = self.options(cx);
        let sheet = self.sheet.clone();
        let done = self.app.update(cx, |app, cx| {
            app.insert_image(&sheet, image, &caption, &options, window, cx)
        });
        if done.unwrap_or(false) {
            window.close_dialog(cx);
        }
    }
}

impl InsertImageForm {
    /// 파일 탭: 책 정보의 표지 칸처럼 끌어다 놓거나 눌러서 고른다. 고르면 미리 보여 준다.
    fn render_file(&self, pal: Palette, cx: &mut Context<Self>) -> AnyElement {
        let zone = Button::new("insert-image-drop")
            .ghost()
            .w_full()
            .h(px(SOURCE_HEIGHT))
            .p_0()
            .overflow_hidden()
            .border_1()
            .border_color(pal.panel_border)
            .bg(pal.panel)
            .drag_over::<ExternalPaths>(move |style, _, _, _| {
                style.border_color(pal.foreground).bg(pal.hover)
            })
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                this.use_file(paths.paths(), window, cx)
            }))
            .on_click(cx.listener(|this, _, window, cx| this.pick_file(window, cx)))
            .map(|this| match &self.file {
                Some(path) => this.child(
                    img(crate::thumbnail::source(
                        path.clone(),
                        self.file_modified,
                        px(DIALOG_WIDTH),
                    ))
                    .size_full()
                    .object_fit(ObjectFit::Contain)
                    .with_fallback(move || {
                        Icon::new(icons::image())
                            .text_color(pal.muted)
                            .into_any_element()
                    }),
                ),
                None => this.child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap_2()
                        .text_sm()
                        .text_color(pal.muted)
                        .child(Icon::new(icons::image()).large())
                        .child("이미지를 끌어다 놓거나 눌러서 선택"),
                ),
            });
        let status = match (&self.file_error, &self.file) {
            (Some(error), _) => Some(
                div()
                    .text_xs()
                    .text_color(pal.danger)
                    .child(error.clone())
                    .into_any_element(),
            ),
            (None, Some(path)) => Some(self.image_status(path, pal)),
            (None, None) => None,
        };
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(zone)
            .children(status)
            .into_any_element()
    }

    /// 고른 이미지의 파일 이름과 원본 크기.
    fn image_status(&self, path: &Path, pal: Palette) -> AnyElement {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        div()
            .flex()
            .gap_2()
            .text_xs()
            .text_color(pal.muted)
            .child(div().flex_1().min_w_0().truncate().child(name))
            .when_some(self.natural_size(), |this, (width, height)| {
                this.child(format!("{width} × {height}"))
            })
            .into_any_element()
    }

    /// 첨부파일 탭: 이미지 썸네일 격자. 누르면 고르고, 두 번 누르면 바로 넣는다.
    /// 고른 것은 아이콘·색 고르기처럼 진한 테두리로 표시한다.
    fn render_attachments(&self, pal: Palette, cx: &mut Context<Self>) -> AnyElement {
        let frame = div()
            .h(px(SOURCE_HEIGHT))
            .rounded_md()
            .border_1()
            .border_color(pal.panel_border)
            .bg(pal.panel);
        if self.attachments.is_empty() {
            return frame
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_2()
                .text_sm()
                .text_color(pal.muted)
                .child(Icon::new(icons::attachments()).large())
                .child("첨부파일에 이미지가 없습니다.")
                .into_any_element();
        }
        let thumbnails = self.attachments.iter().enumerate().map(|(ix, file)| {
            let selected = self.attachment.as_ref() == Some(&file.path);
            let picked = file.path.clone();
            div()
                .id(("insert-image-attachment", ix))
                .size(px(THUMBNAIL_SIZE))
                .flex_shrink_0()
                .rounded_md()
                .overflow_hidden()
                .border_2()
                .border_color(if selected {
                    pal.foreground
                } else {
                    pal.panel_border
                })
                .bg(pal.background)
                .cursor_pointer()
                .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                    this.attachment = Some(picked.clone());
                    this.refit_size(window, cx);
                    cx.notify();
                    if event.click_count() >= 2 {
                        this.submit(window, cx);
                    }
                }))
                .child(
                    img(crate::thumbnail::source(
                        file.path.clone(),
                        file.modified,
                        px(THUMBNAIL_SIZE),
                    ))
                    .size_full()
                    .object_fit(ObjectFit::Cover)
                    .with_fallback(move || {
                        Icon::new(icons::image())
                            .text_color(pal.muted)
                            .into_any_element()
                    }),
                )
        });
        let status = self
            .attachment
            .as_ref()
            .map(|path| self.image_status(path, pal));
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                frame.p_2().child(
                    div()
                        .id("insert-image-attachments")
                        .size_full()
                        .overflow_y_scroll()
                        .child(div().flex().flex_wrap().gap_2().children(thumbnails)),
                ),
            )
            .children(status)
            .into_any_element()
    }

    /// 크기: `[가로 px] × [세로 px] □ 비율 유지`. 라벨과 한 줄에 맞도록 한 줄로 둔다.
    fn render_size(&self, pal: Palette, disabled: bool, cx: &mut Context<Self>) -> AnyElement {
        let field = |input: &Entity<InputState>| {
            div().flex_1().min_w_0().child(
                Input::new(input)
                    .focus_ring(false)
                    .disabled(disabled)
                    .suffix(div().text_xs().text_color(pal.muted).child("px")),
            )
        };
        let can_keep = !disabled && self.natural_size().is_some();
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(field(&self.width))
            .child(div().text_sm().text_color(pal.muted).child("×"))
            .child(field(&self.height))
            .child(
                Checkbox::new("insert-image-keep-ratio")
                    .ml_1()
                    .label("비율 유지")
                    .checked(self.keep_ratio)
                    .disabled(!can_keep)
                    .on_click(cx.listener(|this, checked: &bool, window, cx| {
                        this.keep_ratio = *checked;
                        this.refit_size(window, cx);
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    /// 정렬(왼쪽·가운데·오른쪽).
    fn render_align(&self, disabled: bool, cx: &mut Context<Self>) -> AnyElement {
        let form = cx.entity().downgrade();
        TabBar::new("insert-image-align")
            .segmented()
            .selected_index(
                ALIGNS
                    .iter()
                    .position(|(align, _)| *align == self.align)
                    .unwrap_or(1),
            )
            .on_click(move |index: &usize, _, cx| {
                form.update(cx, |form, cx| {
                    if let Some((align, _)) = ALIGNS.get(*index) {
                        form.align = *align;
                        cx.notify();
                    }
                })
                .ok();
            })
            .children(
                ALIGNS
                    .iter()
                    .map(|(_, label)| Tab::new().flex_1().label(*label).disabled(disabled)),
            )
            .into_any_element()
    }

    /// 배치: 한 페이지 가득. 켜면 크기·정렬은 쓰지 않는다.
    fn render_full_page(&self, pal: Palette, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                Checkbox::new("insert-image-full-page")
                    .label("한 페이지 가득")
                    .checked(self.full_page)
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.full_page = *checked;
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(pal.muted)
                    .child("전용 페이지에 최대한 크게"),
            )
            .into_any_element()
    }
}

impl Render for InsertImageForm {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pal = Palette::from_theme(cx.theme());
        let form = cx.entity().downgrade();
        let tabs = TabBar::new("insert-image-source")
            .segmented()
            .selected_index(
                self.tabs
                    .iter()
                    .position(|&tab| tab == self.source)
                    .unwrap_or(0),
            )
            .on_click(move |index: &usize, window, cx| {
                form.update(cx, |form, cx| {
                    let Some(&source) = form.tabs.get(*index) else {
                        return;
                    };
                    form.source = source;
                    match source {
                        Source::Url => form.url.update(cx, |state, cx| state.focus(window, cx)),
                        _ => form.refit_size(window, cx),
                    }
                    cx.notify();
                })
                .ok();
            })
            .children(self.tabs.iter().map(|tab| {
                let (icon, label) = match tab {
                    Source::File => (icons::image(), "파일"),
                    Source::Attachment => (icons::attachments(), "첨부파일"),
                    Source::Url => (icons::link(), "URL"),
                };
                Tab::new().flex_1().icon(icon).label(label)
            }));

        let source = match self.source {
            Source::File => self.render_file(pal, cx),
            Source::Attachment => self.render_attachments(pal, cx),
            Source::Url => {
                form_row(pal, "주소", Input::new(&self.url).focus_ring(false)).into_any_element()
            }
        };

        // 한 페이지 가득이면 크기·정렬은 쓰지 않으므로 흐리게 막는다.
        let layout_disabled = self.full_page;
        let size = self.render_size(pal, layout_disabled, cx);
        let align = self.render_align(layout_disabled, cx);
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(tabs)
            .child(source)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .when(layout_disabled, |this| this.opacity(0.5))
                    .child(form_row(pal, "크기", size))
                    .child(form_row(pal, "정렬", align)),
            )
            .child(form_row(pal, "배치", self.render_full_page(pal, cx)))
            .child(form_row(
                pal,
                "캡션",
                Input::new(&self.caption).focus_ring(false),
            ))
    }
}

impl WriterApp {
    /// 이미지 삽입 대화상자를 연다. 열린 시트가 없으면 안내만 한다.
    pub(super) fn on_insert_image(
        &mut self,
        _: &InsertImage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(sheet) = self
            .editor
            .as_ref()
            .map(|editor| editor.path().to_path_buf())
        else {
            self.notify("먼저 시트를 여세요.", window, cx);
            return;
        };
        let attachments = self.project_of(&sheet).map(|project| {
            let dir = project
                .join(library::EXTRAS_DIR)
                .join(library::ATTACHMENTS_GROUP);
            self.find_group(&dir)
                .map(|group| {
                    group
                        .files
                        .into_iter()
                        .filter(|file| file.kind == library::FileKind::Image)
                        .collect()
                })
                .unwrap_or_default()
        });
        let app = cx.entity().downgrade();
        let form = cx.new(|cx| InsertImageForm::new(app, sheet, attachments, window, cx));
        // 처음 그릴 때만 캡션 칸에 포커스를 줘서 Enter로 바로 넣을 수 있게 한다.
        let initial_focus = Cell::new(true);
        window.open_dialog(cx, move |dialog, window, cx| {
            if initial_focus.replace(false) {
                let caption = form.read(cx).caption.clone();
                caption.update(cx, |state, cx| state.focus(window, cx));
            }
            let ok_form = form.clone();
            dialog
                .w(px(DIALOG_WIDTH))
                .title("이미지 삽입")
                .close_button(false)
                .child(form.clone())
                .footer(
                    DialogFooter::new()
                        .child(
                            Button::new("insert-image-cancel")
                                .label("취소")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("insert-image-ok")
                                .primary()
                                .label("삽입")
                                .on_click(move |_, window, cx| {
                                    ok_form.update(cx, |form, cx| form.submit(window, cx));
                                }),
                        ),
                )
        });
    }

    /// 열린 시트(`sheet`)의 커서 자리에 이미지 마크다운을 넣는다. 넣었으면 `true`.
    ///
    /// 이미지는 문단 하나로 둔다(캡션이 붙는 그림이 되도록). 커서 앞뒤에 글이 있으면 빈 줄로 띄운다.
    fn insert_image(
        &mut self,
        sheet: &Path,
        image: ImageSource,
        caption: &str,
        options: &ImageOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.editor.as_ref().map(|editor| editor.path()) != Some(sheet) {
            self.notify("이미지를 넣을 시트가 닫혔습니다.", window, cx);
            return false;
        }
        let destination = match image {
            ImageSource::Url(url) => url,
            ImageSource::File(path) => match self.project_of(sheet) {
                Some(project) => match library::attach_to_sheet(&project, sheet, &path) {
                    // 첨부파일 그룹의 변화는 폴더 감시가 다시 읽어 반영한다.
                    Ok(relative) => relative,
                    Err(err) => {
                        self.notify(format!("이미지를 첨부할 수 없습니다: {err}"), window, cx);
                        return false;
                    }
                },
                None => path.to_string_lossy().into_owned(),
            },
        };
        let Some(editor) = &self.editor else {
            return false;
        };
        let image = markdown::image(caption, &destination, options);
        editor.state().update(cx, |state, cx| {
            let text = state.value();
            let cursor = state.cursor().min(text.len());
            let text = paragraph_around(&text[..cursor], &text[cursor..], &image);
            state.insert(text, window, cx);
            state.focus(window, cx);
        });
        self.on_editor_changed(window, cx);
        true
    }

    /// 시트가 든 프로젝트 폴더(`Projects/<프로젝트>`). 프로젝트 밖이면 `None`.
    fn project_of(&self, sheet: &Path) -> Option<PathBuf> {
        let projects = self
            .library
            .as_ref()?
            .root
            .join(library::SectionKind::Projects.dir());
        let name = sheet.strip_prefix(&projects).ok()?.components().next()?;
        Some(projects.join(name))
    }
}

/// `before`와 `after` 사이에 `block`을 독립 문단으로 넣을 때 쓸 텍스트(앞뒤 빈 줄 포함).
fn paragraph_around(before: &str, after: &str, block: &str) -> String {
    let lead = if before.is_empty() || before.ends_with("\n\n") {
        ""
    } else if before.ends_with('\n') {
        "\n"
    } else {
        "\n\n"
    };
    let trail = if after.is_empty() || after.starts_with("\n\n") {
        ""
    } else if after.starts_with('\n') {
        "\n"
    } else {
        "\n\n"
    };
    format!("{lead}{block}{trail}")
}

#[cfg(test)]
mod tests {
    use super::paragraph_around;

    #[test]
    fn image_becomes_its_own_paragraph() {
        assert_eq!(paragraph_around("", "", "IMG"), "IMG");
        assert_eq!(paragraph_around("글", "", "IMG"), "\n\nIMG");
        assert_eq!(paragraph_around("글\n", "\n다음", "IMG"), "\nIMG\n");
        assert_eq!(paragraph_around("글\n\n", "다음", "IMG"), "IMG\n\n");
    }
}
