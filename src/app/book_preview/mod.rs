//! 프로젝트 미리보기 창(ADR-0013): 원고를 PDF(인쇄 판형) 또는 ePub(리더 화면) 기준으로 배치해 보여 준다.
//!
//! - 원고: 프로젝트 바로 아래 시트 → 원고 그룹(깊이 우선)의 시트. 접합하지 않은 시트는 새 페이지에서 시작한다.
//! - 책 스타일(ADR-0009)·책 정보(표지·제목·지은이)를 그대로 쓴다.
//! - 프로젝트 폴더를 감시해 시트·스타일이 바뀌면(자동 저장 포함) 다시 읽는다.
//! - 프로젝트마다 창 하나. 이미 열려 있으면 앞으로 가져온다.

mod layout;
mod page;
mod pdf;

pub(crate) use pdf::export_pdf;

#[cfg(not(target_os = "macos"))]
use gpui_kit::component::TitleBar;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, StyledExt as _, WindowExt as _};
use gpui_kit::gpui::prelude::FluentBuilder as _;
use gpui_kit::gpui::*;
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use super::style::*;
use crate::markdown::Block;
use crate::settings::Prefs;
use crate::{icons, library, markdown, watch};
use layout::{BookLayout, Fonts, PageSetup};

/// 창 크기.
const WINDOW_SIZE: Size<Pixels> = Size {
    width: px(980.0),
    height: px(860.0),
};

/// PDF 페이지 사이 간격(화면 px).
const PAGE_GAP: f32 = 24.0;

/// PDF 확대 단계. 1.0 = 실제 크기(1pt = 4/3px).
const ZOOMS: [f32; 7] = [0.5, 0.67, 0.75, 1.0, 1.25, 1.5, 2.0];
const DEFAULT_ZOOM: usize = 3;

/// pt → 화면 px.
const PX_PER_PT: f32 = 96.0 / 72.0;

/// 파일이 바뀐 뒤 다시 읽기까지 기다리는 시간. 저장은 이벤트가 연달아 온다.
const RELOAD_DEBOUNCE_MS: u64 = 300;

/// 미리보기에 쓰는 원고(디스크에서 읽은 것). 백그라운드에서 만든다.
pub(super) struct BookSource {
    pub title: String,
    pub authors: Vec<String>,
    pub cover: Option<PathBuf>,
    pub styles: library::Styles,
    pub chunks: Vec<Chunk>,
}

/// 시트 하나의 블록.
pub(super) struct Chunk {
    /// 상대 경로 이미지의 기준(시트 폴더).
    pub dir: PathBuf,
    /// 앞 시트에 이어지는지. 아니면 새 페이지에서 시작한다.
    pub glued: bool,
    pub blocks: Vec<Block>,
}

/// 프로젝트 원고·책 정보·스타일을 읽는다.
fn load_source(project_dir: &Path) -> io::Result<BookSource> {
    let root = project_dir
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "프로젝트 폴더가 아닙니다"))?;
    let library = library::scan(root)?;
    let project = library
        .find_project(project_dir)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "프로젝트를 찾을 수 없습니다"))?;
    let book = library::read_book(project_dir);
    let styles = library::read_styles(project_dir);
    let preserve_line_breaks = styles
        .resolve(library::StyleTarget::Body)
        .preserve_line_breaks;
    let chunks = library::manuscript_sheets(project)
        .into_iter()
        .map(|sheet| {
            // 못 읽는 시트는 비운 채로 둔다(나머지 원고는 보인다).
            let text = library::read_sheet(&sheet.path).unwrap_or_default();
            Chunk {
                dir: sheet
                    .path
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_default(),
                glued: sheet.glued,
                blocks: markdown::manuscript_blocks(&text, preserve_line_breaks),
            }
        })
        .collect();
    Ok(BookSource {
        title: if book.title.trim().is_empty() {
            project.name.clone()
        } else {
            book.title.clone()
        },
        authors: book.authors.clone(),
        cover: book.cover_path(project_dir).filter(|path| path.is_file()),
        styles,
        chunks,
    })
}

/// 선택한 그룹 또는 시트의 접합 묶음만 읽는다.
fn load_selection(root: &Path, selected: &[PathBuf]) -> io::Result<BookSource> {
    let project = crate::export::load_selection(root, selected).map_err(io::Error::other)?;
    let preserve_line_breaks = project
        .styles
        .resolve(library::StyleTarget::Body)
        .preserve_line_breaks;
    let chunks = project
        .sheets
        .into_iter()
        .map(|sheet| Chunk {
            dir: sheet
                .path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_default(),
            glued: sheet.glued,
            blocks: markdown::manuscript_blocks(&sheet.content, preserve_line_breaks),
        })
        .collect();
    Ok(BookSource {
        title: project.name,
        authors: Vec::new(),
        cover: None,
        styles: project.styles,
        chunks,
    })
}

/// DOCX 판형·서체(컴파일 대화상자). PDF와 같은 판형 값(여백·본문 크기)을 쓴다.
pub(crate) fn docx_options(
    paper: Paper,
    generate_toc: bool,
    cx: &App,
) -> crate::export::docx::DocxOptions {
    let setup = paper.setup();
    let fonts = layout_fonts(cx);
    // 테마 서체 같은 시스템 내부 이름(`.`으로 시작)은 Word가 모르므로 적지 않는다.
    let named = |family: SharedString| (!family.starts_with('.')).then(|| family.to_string());
    crate::export::docx::DocxOptions {
        page_width: f32::from(setup.width),
        page_height: f32::from(setup.height),
        margin_top: f32::from(setup.margin.top),
        margin_right: f32::from(setup.margin.right),
        margin_bottom: f32::from(setup.margin.bottom),
        margin_left: f32::from(setup.margin.left),
        font_size: f32::from(setup.font_size),
        default_font: named(fonts.editor),
        mono_font: named(fonts.mono).unwrap_or_else(|| "Menlo".into()),
        generate_toc,
    }
}

/// 배치에 쓰는 서체: 설정의 에디터 서체, 리더 기본 서체(테마 서체), 고정폭 서체.
fn layout_fonts(cx: &App) -> Fonts {
    let theme = cx.theme();
    Fonts {
        editor: Prefs::editor_font_family(cx),
        reader: theme.font_family.clone(),
        mono: theme.mono_font_family.clone(),
    }
}

/// 미리보기 기준.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Format {
    Pdf,
    Epub,
}

impl Format {
    fn title(self) -> &'static str {
        match self {
            Format::Pdf => "PDF",
            Format::Epub => "ePub",
        }
    }
}

/// PDF 판형.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Paper {
    Shinguk,
    A5,
    SixFour,
    A4,
    Letter,
}

impl Paper {
    pub(crate) const ALL: [Paper; 5] = [
        Paper::Shinguk,
        Paper::A5,
        Paper::SixFour,
        Paper::A4,
        Paper::Letter,
    ];

    pub(crate) fn title(self) -> &'static str {
        match self {
            Paper::Shinguk => "신국판 (152×225mm)",
            Paper::A5 => "A5 (148×210mm)",
            Paper::SixFour => "4·6판 (128×188mm)",
            Paper::A4 => "A4 (210×297mm)",
            Paper::Letter => "US Letter (8.5×11in)",
        }
    }

    pub(crate) fn short_title(self) -> &'static str {
        match self {
            Paper::Shinguk => "신국판",
            Paper::A5 => "A5",
            Paper::SixFour => "4·6판",
            Paper::A4 => "A4",
            Paper::Letter => "Letter",
        }
    }

    /// 판형(pt), 여백, 본문 글자 크기. 책 판형은 10pt 안팎, 문서 판형은 11pt.
    fn setup(self) -> PageSetup {
        let mm = |value: f32| px(value * 72.0 / 25.4);
        let (width, height, top, side, bottom, font_size) = match self {
            Paper::Shinguk => (mm(152.), mm(225.), mm(20.), mm(18.), mm(22.), 10.),
            Paper::A5 => (mm(148.), mm(210.), mm(19.), mm(17.), mm(21.), 10.),
            Paper::SixFour => (mm(128.), mm(188.), mm(16.), mm(14.), mm(18.), 9.5),
            Paper::A4 => (mm(210.), mm(297.), mm(25.4), mm(25.4), mm(25.4), 11.),
            Paper::Letter => (px(612.), px(792.), px(72.), px(72.), px(72.), 11.),
        };
        PageSetup {
            width,
            height,
            margin: Edges {
                top,
                right: side,
                bottom,
                left: side,
            },
            font_size: px(font_size),
            px: 0.75,
            print: true,
        }
    }
}

/// ePub 리더 화면.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Device {
    Phone,
    Tablet,
}

impl Device {
    const ALL: [Device; 2] = [Device::Phone, Device::Tablet];

    fn title(self) -> &'static str {
        match self {
            Device::Phone => "휴대폰",
            Device::Tablet => "태블릿",
        }
    }

    /// 화면 크기(CSS px), 여백, 리더 기본 글자 크기.
    fn setup(self) -> PageSetup {
        let (width, height, vertical, side, font_size) = match self {
            Device::Phone => (375., 667., 36., 22., 17.),
            Device::Tablet => (600., 860., 48., 48., 18.),
        };
        PageSetup {
            width: px(width),
            height: px(height),
            margin: Edges {
                top: px(vertical),
                right: px(side),
                bottom: px(vertical),
                left: px(side),
            },
            font_size: px(font_size),
            px: 1.0,
            print: false,
        }
    }
}

/// 배치를 다시 할지 정하는 값. 하나라도 바뀌면 다시 배치한다.
#[derive(Clone, PartialEq, Debug)]
struct LayoutKey {
    revision: usize,
    generate_toc: bool,
    setup: PageSetup,
    fonts: Fonts,
}

/// 열린 미리보기 창(프로젝트 폴더 → 창).
#[derive(Default)]
struct OpenPreviews(HashMap<(PathBuf, Option<Vec<PathBuf>>), AnyWindowHandle>);

impl Global for OpenPreviews {}

/// 프로젝트 미리보기 창을 연다(이미 열려 있으면 앞으로 가져온다).
pub(crate) fn open(project: &Path, cx: &mut App) {
    open_preview(project, None, cx);
}

pub(super) fn open_selection(root: &Path, selected: &[PathBuf], cx: &mut App) {
    open_preview(root, Some(selected.to_vec()), cx);
}

fn open_preview(project: &Path, selected: Option<Vec<PathBuf>>, cx: &mut App) {
    let key = (project.to_path_buf(), selected.clone());
    if let Some(handle) = cx
        .try_global::<OpenPreviews>()
        .and_then(|open| open.0.get(&key))
        .copied()
        && handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
    {
        return;
    }

    let window_bounds = Some(WindowBounds::Windowed(Bounds::centered(
        None,
        WINDOW_SIZE,
        cx,
    )));
    #[cfg(not(target_os = "macos"))]
    let options = WindowOptions {
        window_bounds,
        window_min_size: Some(size(px(560.0), px(480.0))),
        ..TitleBar::window_options()
    };
    #[cfg(target_os = "macos")]
    let options = WindowOptions {
        window_bounds,
        window_min_size: Some(size(px(560.0), px(480.0))),
        titlebar: Some(TitlebarOptions {
            title: Some("미리보기".into()),
            appears_transparent: true,
            traffic_light_position: Some(point(
                px(PANEL_INSET + 12.0),
                px(TOOLBAR_HEIGHT / 2.0 - 7.0),
            )),
        }),
        ..Default::default()
    };

    let dir = project.to_path_buf();
    match gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| BookPreview::new(dir, selected, window, cx))
    }) {
        Ok((handle, _)) => {
            if !cx.has_global::<OpenPreviews>() {
                cx.set_global(OpenPreviews::default());
            }
            cx.global_mut::<OpenPreviews>().0.insert(key, handle);
        }
        Err(err) => eprintln!("미리보기 창을 열 수 없습니다: {err}"),
    }
}

/// 미리보기 창 화면.
struct BookPreview {
    project: PathBuf,
    selected: Option<Vec<PathBuf>>,
    source: Option<Rc<BookSource>>,
    /// 원고를 다시 읽을 때마다 올린다(배치 다시 하기).
    revision: usize,
    error: Option<SharedString>,
    format: Format,
    paper: Paper,
    device: Device,
    zoom: usize,
    /// PDF 본문 앞에 목차 페이지를 생성할지 여부.
    generate_toc: bool,
    layout: Option<(LayoutKey, Rc<BookLayout>)>,
    /// ePub에서 보고 있는 페이지.
    epub_page: usize,
    pdf_scroll: UniformListScrollHandle,
    focus: FocusHandle,
    load_task: Option<Task<()>>,
    /// 프로젝트 폴더 감시. 버리면 멈춘다.
    _watcher: Option<(notify::RecommendedWatcher, Task<()>)>,
    _prefs: Subscription,
}

impl BookPreview {
    fn new(
        project: PathBuf,
        selected: Option<Vec<PathBuf>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let mut this = Self {
            project,
            selected,
            source: None,
            revision: 0,
            error: None,
            format: Format::Pdf,
            paper: Paper::Shinguk,
            device: Device::Phone,
            zoom: DEFAULT_ZOOM,
            generate_toc: false,
            layout: None,
            epub_page: 0,
            pdf_scroll: UniformListScrollHandle::new(),
            focus,
            load_task: None,
            _watcher: None,
            // 에디터 서체가 바뀌면 다시 배치한다.
            _prefs: cx.observe_global::<Prefs>(|_, cx| cx.notify()),
        };
        this.start_watching(window, cx);
        this.reload(window, cx);
        this
    }

    /// 원고를 백그라운드에서 다시 읽는다.
    fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let project = self.project.clone();
        let selected = self.selected.clone();
        self.load_task = Some(cx.spawn_in(window, async move |this, cx| {
            let loaded = cx
                .background_executor()
                .spawn(async move {
                    match selected {
                        Some(selected) => load_selection(&project, &selected),
                        None => load_source(&project),
                    }
                })
                .await;
            this.update_in(cx, |this, window, cx| {
                match loaded {
                    Ok(source) => {
                        window.set_window_title(&format!("{} — 미리보기", source.title));
                        this.source = Some(Rc::new(source));
                        this.error = None;
                    }
                    Err(err) => this.error = Some(format!("원고를 읽을 수 없습니다: {err}").into()),
                }
                this.revision += 1;
                cx.notify();
            })
            .ok();
        }));
    }

    /// 프로젝트 폴더를 감시해 바뀌면 다시 읽는다.
    fn start_watching(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (watcher, mut events) = match watch::watch(&self.project) {
            Ok(pair) => pair,
            Err(err) => {
                eprintln!("미리보기 폴더 감시 실패: {err}");
                return;
            }
        };
        let task = cx.spawn_in(window, async move |this, cx| {
            use futures::StreamExt as _;
            while events.next().await.is_some() {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(RELOAD_DEBOUNCE_MS))
                    .await;
                while events.try_recv().is_ok() {}
                if this
                    .update_in(cx, |this, window, cx| this.reload(window, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        self._watcher = Some((watcher, task));
    }

    fn setup(&self) -> PageSetup {
        match self.format {
            Format::Pdf => self.paper.setup(),
            Format::Epub => self.device.setup(),
        }
    }

    /// 지금 기준의 배치. 원고·판형·서체가 바뀌었으면 다시 배치한다.
    fn ensure_layout(&mut self, window: &mut Window, cx: &App) -> Option<Rc<BookLayout>> {
        let source = self.source.clone()?;
        let key = LayoutKey {
            revision: self.revision,
            generate_toc: self.generate_toc,
            setup: self.setup(),
            fonts: layout_fonts(cx),
        };
        if let Some((current, layout)) = &self.layout
            && *current == key
        {
            return Some(layout.clone());
        }
        let layout = Rc::new(layout::layout_book(
            &source,
            key.setup,
            &key.fonts,
            window.text_system(),
            key.generate_toc,
        ));
        self.epub_page = self.epub_page.min(layout.pages.len().saturating_sub(1));
        self.layout = Some((key, layout.clone()));
        Some(layout)
    }

    fn set_format(&mut self, format: Format, cx: &mut Context<Self>) {
        self.format = format;
        self.epub_page = 0;
        cx.notify();
    }

    fn turn_page(&mut self, delta: isize, cx: &mut Context<Self>) {
        let count = self
            .layout
            .as_ref()
            .map_or(0, |(_, layout)| layout.pages.len());
        let page = (self.epub_page as isize + delta).clamp(0, count.saturating_sub(1) as isize);
        self.epub_page = page as usize;
        cx.notify();
    }

    /// 차례 항목으로 간다.
    fn go_to(&mut self, page: usize, cx: &mut Context<Self>) {
        match self.format {
            Format::Pdf => self.pdf_scroll.scroll_to_item(page, ScrollStrategy::Top),
            Format::Epub => self.epub_page = page,
        }
        cx.notify();
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.format != Format::Epub {
            return;
        }
        match event.keystroke.key.as_str() {
            "left" | "pageup" => self.turn_page(-1, cx),
            "right" | "pagedown" | "space" => self.turn_page(1, cx),
            _ => {}
        }
    }

    /// 지금 보이는 PDF 배치(판형·목차 포함)를 그대로 PDF 파일로 내보낸다.
    fn export_pdf(&self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some((_, layout)), Some(source)) = (&self.layout, &self.source) else {
            return;
        };
        if !layout.setup.print {
            return;
        }
        let pdf = pdf::document(layout, &source.title, &source.authors);
        let default_name = format!("{}.pdf", source.title.trim());
        let default_dir = self
            .project
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.project.clone());
        let receiver = cx.prompt_for_new_path(&default_dir, Some(&default_name));
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(target))) = receiver.await else {
                return;
            };
            let name = target
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_default();
            let result = cx
                .background_executor()
                .spawn(async move { crate::export::pdf::write(&pdf, &target) })
                .await;
            this.update_in(cx, |_, window, cx| {
                let message = match result {
                    Ok(()) => format!("PDF 내보내기 완료: {name}"),
                    Err(err) => format!("PDF 내보내기 실패: {err}"),
                };
                window.push_notification(
                    gpui_kit::component::notification::Notification::info(SharedString::from(
                        message,
                    )),
                    cx,
                );
            })
            .ok();
        })
        .detach();
    }

    /// 현재 프로젝트를 ePub 파일로 내보낸다.
    fn export_epub(&self, window: &mut Window, cx: &mut Context<Self>) {
        let title = self
            .source
            .as_ref()
            .map(|s| s.title.clone())
            .unwrap_or_else(|| "책".to_string());
        let default_name = format!("{title}.epub");
        let project_dir = self.project.clone();
        let default_dir = project_dir
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| project_dir.clone());

        let receiver = cx.prompt_for_new_path(&default_dir, Some(&default_name));
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(output_path))) = receiver.await else {
                return;
            };

            let project = project_dir.clone();
            let target = output_path.clone();
            let result = cx
                .background_executor()
                .spawn(async move { crate::export::epub::export_to_file(&project, &target) })
                .await;

            this.update_in(cx, |_this, window, cx| {
                let file_name = output_path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "책".to_string());
                match result {
                    Ok(()) => {
                        window.push_notification(
                            gpui_kit::component::notification::Notification::info(
                                SharedString::from(format!("ePub 내보내기 완료: {file_name}")),
                            ),
                            cx,
                        );
                    }
                    Err(err) => {
                        window.push_notification(
                            gpui_kit::component::notification::Notification::info(
                                SharedString::from(format!("ePub 내보내기 실패: {err}")),
                            ),
                            cx,
                        );
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    // --- 렌더 ---------------------------------------------------------------

    fn render_toolbar(
        &self,
        layout: Option<&Rc<BookLayout>>,
        cx: &mut Context<Self>,
        pal: Palette,
    ) -> AnyElement {
        let view = cx.entity().downgrade();
        let segment = |format: Format, cx: &mut Context<Self>| {
            let selected = self.format == format;
            div()
                .id(format.title())
                .h(px(24.0))
                .px_3()
                .flex()
                .items_center()
                .rounded_full()
                .text_sm()
                .cursor_pointer()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .map(|this| {
                    if selected {
                        this.bg(pal.accent)
                            .text_color(pal.accent_foreground)
                            .font_semibold()
                    } else {
                        this.text_color(pal.foreground.opacity(0.8))
                            .hover(move |style| style.bg(pal.hover))
                    }
                })
                .on_click(cx.listener(move |this, _, _, cx| this.set_format(format, cx)))
                .child(format.title())
        };
        let formats = tool_group(pal)
            .child(segment(Format::Pdf, cx))
            .child(segment(Format::Epub, cx));

        let size_menu = match self.format {
            Format::Pdf => {
                let current = self.paper;
                let view = view.clone();
                Button::new("paper")
                    .label(current.short_title())
                    .ghost()
                    .small()
                    .dropdown_caret(true)
                    .dropdown_menu(move |menu, _, _| {
                        Paper::ALL
                            .into_iter()
                            .fold(menu.min_w(px(200.0)), |menu, paper| {
                                let view = view.clone();
                                menu.item(
                                    PopupMenuItem::new(paper.title())
                                        .checked(paper == current)
                                        .on_click(move |_, _, cx| {
                                            view.update(cx, |this, cx| {
                                                this.paper = paper;
                                                cx.notify();
                                            })
                                            .ok();
                                        }),
                                )
                            })
                    })
                    .into_any_element()
            }
            Format::Epub => {
                let current = self.device;
                let view = view.clone();
                Button::new("device")
                    .label(current.title())
                    .ghost()
                    .small()
                    .dropdown_caret(true)
                    .dropdown_menu(move |menu, _, _| {
                        Device::ALL
                            .into_iter()
                            .fold(menu.min_w(px(140.0)), |menu, device| {
                                let view = view.clone();
                                menu.item(
                                    PopupMenuItem::new(device.title())
                                        .checked(device == current)
                                        .on_click(move |_, _, cx| {
                                            view.update(cx, |this, cx| {
                                                this.device = device;
                                                this.epub_page = 0;
                                                cx.notify();
                                            })
                                            .ok();
                                        }),
                                )
                            })
                    })
                    .into_any_element()
            }
        };

        let toc_menu = layout
            .filter(|layout| !layout.toc.is_empty())
            .map(|layout| {
                let entries: Vec<_> = layout
                    .toc
                    .iter()
                    .map(|entry| (entry.title.clone(), entry.depth, entry.page))
                    .collect();
                let view = view.clone();
                Button::new("toc")
                    .icon(icons::toc())
                    .ghost()
                    .small()
                    .dropdown_menu(move |menu, _, _| {
                        let menu = menu
                            .min_w(px(220.0))
                            .max_w(px(360.0))
                            .max_h(px(480.0))
                            .scrollable(true)
                            .label("차례");
                        entries.iter().fold(menu, |menu, (title, depth, page)| {
                            let (view, page) = (view.clone(), *page);
                            let indent = "\u{2003}".repeat(*depth);
                            menu.item(PopupMenuItem::new(format!("{indent}{title}")).on_click(
                                move |_, _, cx| {
                                    view.update(cx, |this, cx| this.go_to(page, cx)).ok();
                                },
                            ))
                        })
                    })
            });

        let pages = layout.map_or(0, |layout| layout.pages.len());
        let navigation = match self.format {
            Format::Pdf => tool_group(pal)
                .child(
                    icon_button("zoom-out", icons::zoom_out(), self.zoom > 0, pal).on_click(
                        cx.listener(|this, _, _, cx| {
                            this.zoom = this.zoom.saturating_sub(1);
                            cx.notify();
                        }),
                    ),
                )
                .child(
                    div()
                        .w(px(44.0))
                        .text_center()
                        .text_xs()
                        .text_color(pal.muted)
                        .child(format!("{}%", (ZOOMS[self.zoom] * 100.0).round())),
                )
                .child(
                    icon_button(
                        "zoom-in",
                        icons::zoom_in(),
                        self.zoom + 1 < ZOOMS.len(),
                        pal,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.zoom = (this.zoom + 1).min(ZOOMS.len() - 1);
                        cx.notify();
                    })),
                )
                .into_any_element(),
            Format::Epub => tool_group(pal)
                .child(
                    icon_button("page-prev", icons::back(), self.epub_page > 0, pal)
                        .on_click(cx.listener(|this, _, _, cx| this.turn_page(-1, cx))),
                )
                .child(
                    div()
                        .min_w(px(56.0))
                        .px_1()
                        .text_center()
                        .text_xs()
                        .text_color(pal.muted)
                        .child(format!("{} / {pages}", (self.epub_page + 1).min(pages))),
                )
                .child(
                    icon_button(
                        "page-next",
                        icons::forward(),
                        self.epub_page + 1 < pages,
                        pal,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.turn_page(1, cx))),
                )
                .into_any_element(),
        };

        let title = self
            .source
            .as_ref()
            .map(|source| source.title.clone())
            .unwrap_or_default();
        window_drag_area(
            div()
                .h(px(TOOLBAR_HEIGHT))
                .flex_shrink_0()
                .px_3()
                .flex()
                .items_center()
                .gap_2()
                .border_b_1()
                .border_color(pal.border),
        )
        .map(|this| {
            #[cfg(target_os = "macos")]
            let this = this.pl(px(TRAFFIC_LIGHT_SPACE));
            this
        })
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap_2()
                .child(div().text_sm().font_semibold().truncate().child(title))
                .when(self.format == Format::Pdf && pages > 0, |this| {
                    this.child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(pal.muted)
                            .child(format!("{pages}쪽")),
                    )
                }),
        )
        .child(formats)
        .when(self.format == Format::Pdf, |this| {
            this.child(
                Checkbox::new("preview-generate-toc")
                    .label("목차 생성")
                    .checked(self.generate_toc)
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.generate_toc = *checked;
                        this.pdf_scroll.scroll_to_item(0, ScrollStrategy::Top);
                        cx.notify();
                    })),
            )
        })
        .child(
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_end()
                .gap_2()
                .child(
                    tool_group(pal)
                        .child(size_menu)
                        .when_some(toc_menu, |this, menu| this.child(menu)),
                )
                .child(navigation)
                // PDF는 보이는 배치를 그대로, ePub은 프로젝트 전체를 내보낸다.
                .when(
                    self.format == Format::Pdf || self.selected.is_none(),
                    |this| {
                        this.child(tool_group(pal).child(
                            icon_button("export", icons::export(), pages > 0, pal).on_click(
                                cx.listener(|this, _, window, cx| match this.format {
                                    Format::Pdf => this.export_pdf(window, cx),
                                    Format::Epub => this.export_epub(window, cx),
                                }),
                            ),
                        ))
                    },
                ),
        )
        .into_any_element()
    }

    fn render_pdf(&self, layout: Rc<BookLayout>) -> AnyElement {
        let scale = ZOOMS[self.zoom] * PX_PER_PT;
        let count = layout.pages.len();
        uniform_list("pdf-pages", count, move |range, _, _| {
            range
                .map(|ix| {
                    div()
                        .w_full()
                        .flex()
                        .justify_center()
                        .pt(px(PAGE_GAP))
                        .when(ix + 1 == count, |this| this.pb(px(PAGE_GAP)))
                        .child(page::page(layout.clone(), ix, scale))
                })
                .collect()
        })
        .track_scroll(&self.pdf_scroll)
        .size_full()
        .into_any_element()
    }

    fn render_epub(
        &self,
        layout: Rc<BookLayout>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let setup = layout.setup;
        // 창 높이에 맞춰 줄인다(키우지는 않는다).
        let viewport = window.viewport_size();
        let available = size(
            viewport.width - px(64.0),
            viewport.height - px(TOOLBAR_HEIGHT + 64.0),
        );
        let scale = (available.width / setup.width)
            .min(available.height / setup.height)
            .clamp(0.3, 1.0);
        let index = self.epub_page;
        // 화면 왼쪽 절반을 누르면 앞 페이지, 오른쪽 절반은 다음 페이지(리더처럼).
        let half = |id: &'static str, delta: isize, cx: &mut Context<Self>| {
            div()
                .id(id)
                .absolute()
                .top_0()
                .bottom_0()
                .w_1_2()
                .when(delta < 0, |this| this.left_0())
                .when(delta > 0, |this| this.right_0())
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| this.turn_page(delta, cx)))
        };
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .relative()
                    .child(page::page(layout, index, scale))
                    .child(half("page-left", -1, cx))
                    .child(half("page-right", 1, cx)),
            )
            .into_any_element()
    }
}

impl Render for BookPreview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pal = Palette::from_theme(cx.theme());
        let layout = self.ensure_layout(window, cx);
        let toolbar = self.render_toolbar(layout.as_ref(), cx, pal);

        let message = |text: SharedString| {
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(pal.muted)
                .child(text)
                .into_any_element()
        };
        let body = match (layout, &self.error) {
            (_, Some(error)) => message(error.clone()),
            (None, None) => message("원고를 읽는 중…".into()),
            (Some(layout), None) => match self.format {
                Format::Pdf => self.render_pdf(layout),
                Format::Epub => self.render_epub(layout, window, cx),
            },
        };

        let root = div()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key_down))
            .flex()
            .flex_col()
            .size_full()
            .bg(pal.panel)
            .text_color(pal.foreground);

        #[cfg(not(target_os = "macos"))]
        let root = {
            let theme = cx.theme();
            root.child(
                TitleBar::new()
                    .bg(theme.title_bar)
                    .border_color(theme.title_bar_border)
                    .child(div().text_sm().child("미리보기")),
            )
        };

        root.child(toolbar)
            .child(div().flex_1().min_h_0().child(body))
    }
}
