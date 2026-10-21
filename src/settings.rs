//! 설정 창: 메인 창과 같은 떠 있는 왼쪽 패널(페이지 목록) + 오른쪽 가운데 정렬 폼.
//!
//! 설정 값은 전역 [`Prefs`]에 두고, 바꾸면 `config.toml`에 저장한다.
//! 메인 창은 [`Prefs`]를 관찰해 다시 그린다.

use gpui_kit::assets::IconName as AssetIcon;
#[cfg(not(target_os = "macos"))]
use gpui_kit::component::TitleBar;
use gpui_kit::component::searchable_list::{SearchableListItem, SearchableVec};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{
    ActiveTheme as _, FocusableExt as _, Icon, IndexPath, Sizable as _, StyledExt as _,
};
use gpui_kit::gpui::prelude::FluentBuilder as _;
use gpui_kit::gpui::*;

use crate::app::style::{PANEL_INSET, Palette, TOOLBAR_HEIGHT, window_drag_area};
use crate::config::Config;
use crate::icons;

/// 설정 창 크기.
const WINDOW_SIZE: Size<Pixels> = Size {
    width: px(720.0),
    height: px(480.0),
};

/// 왼쪽 페이지 패널 폭.
const PAGE_PANEL_WIDTH: f32 = 180.0;

/// 오른쪽 폼 최대 폭.
const FORM_MAX_WIDTH: f32 = 400.0;

/// 사용자 설정(앱 전역). 시작할 때 `config.toml`에서 읽는다.
#[derive(Clone, Default)]
pub struct Prefs {
    /// 에디터 서체. 없으면 테마 서체(시스템 서체).
    pub editor_font: Option<SharedString>,
    pub editor_font_size: Option<u16>,
}

impl Global for Prefs {}

impl Prefs {
    /// 설정 파일 값으로 전역 설정을 만든다.
    pub fn init(config: &Config, cx: &mut App) {
        cx.set_global(Prefs {
            editor_font: config.editor_font.clone().map(SharedString::from),
            editor_font_size: config
                .editor_font_size
                .filter(|size| (12..=32).contains(size)),
        });
    }

    /// 에디터에 쓸 서체: 고른 서체, 없으면 테마 서체.
    pub fn editor_font_family(cx: &App) -> SharedString {
        cx.global::<Prefs>()
            .editor_font
            .clone()
            .unwrap_or_else(|| cx.theme().font_family.clone())
    }

    pub fn editor_font_size(cx: &App) -> f32 {
        f32::from(cx.global::<Prefs>().editor_font_size.unwrap_or(16))
    }

    fn set_editor_font_size(size: u16, cx: &mut App) {
        if Self::editor_font_size(cx) == f32::from(size) {
            return;
        }
        cx.global_mut::<Prefs>().editor_font_size = Some(size);
        let mut config = Config::load();
        config.editor_font_size = Some(size);
        if let Err(err) = config.save() {
            eprintln!("설정 저장 실패: {err}");
        }
    }

    /// 에디터 서체를 바꾸고 저장한다.
    fn set_editor_font(font: Option<SharedString>, cx: &mut App) {
        if cx.global::<Prefs>().editor_font == font {
            return;
        }
        cx.global_mut::<Prefs>().editor_font = font.clone();
        let mut config = Config::load();
        config.editor_font = font.map(|font| font.to_string());
        if let Err(err) = config.save() {
            eprintln!("설정 저장 실패: {err}");
        }
    }
}

/// 열려 있는 설정 창. 다시 열기 요청이 오면 새로 만들지 않고 앞으로 가져온다.
struct OpenSettingsWindow(AnyWindowHandle);

impl Global for OpenSettingsWindow {}

/// 설정 창을 연다(이미 열려 있으면 앞으로 가져온다).
pub fn open(cx: &mut App) {
    if let Some(OpenSettingsWindow(handle)) = cx.try_global::<OpenSettingsWindow>()
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
        window_min_size: Some(size(px(560.0), px(360.0))),
        ..TitleBar::window_options()
    };
    // 메인 창과 같은 투명 타이틀바. 신호등 버튼은 왼쪽 패널 위쪽 줄 안에 둔다.
    #[cfg(target_os = "macos")]
    let options = WindowOptions {
        window_bounds,
        window_min_size: Some(size(px(560.0), px(360.0))),
        titlebar: Some(TitlebarOptions {
            title: Some("설정".into()),
            appears_transparent: true,
            traffic_light_position: Some(point(
                px(PANEL_INSET + 12.0),
                px(TOOLBAR_HEIGHT / 2.0 - 7.0),
            )),
        }),
        ..Default::default()
    };

    match gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| SettingsWindow::new(window, cx))
    }) {
        Ok((handle, _)) => cx.set_global(OpenSettingsWindow(handle)),
        Err(err) => eprintln!("설정 창을 열 수 없습니다: {err}"),
    }
}

/// 설정 페이지.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    General,
}

impl Page {
    const ALL: [Page; 1] = [Page::General];

    fn title(self) -> &'static str {
        match self {
            Page::General => "일반",
        }
    }

    fn icon(self) -> AssetIcon {
        match self {
            Page::General => icons::settings(),
        }
    }
}

/// 서체 선택지 하나. 이름을 그 서체로 그린다.
#[derive(Clone)]
pub(crate) struct FontItem {
    /// 표시 이름.
    title: SharedString,
    /// 서체 이름. 시스템 서체는 `None`.
    pub(crate) value: Option<SharedString>,
    /// 그릴 때 쓰는 서체(시스템 서체는 테마 서체).
    family: SharedString,
}

impl SearchableListItem for FontItem {
    type Value = Option<SharedString>;

    fn title(&self) -> SharedString {
        self.title.clone()
    }

    fn display_title(&self) -> Option<AnyElement> {
        Some(
            div()
                .text_lg()
                .font_family(self.family.clone())
                .child(self.title.clone())
                .into_any_element(),
        )
    }

    fn render(&self, _: &mut Window, _: &mut App) -> impl IntoElement {
        div()
            .font_family(self.family.clone())
            .child(self.title.clone())
    }

    fn value(&self) -> &Self::Value {
        &self.value
    }
}

/// 서체 선택지: 맨 앞의 기본 항목(`value`가 `None`, `default_family`로 그림) + 설치된 서체.
pub(crate) fn font_items(
    default_title: &'static str,
    default_family: SharedString,
    cx: &App,
) -> Vec<FontItem> {
    // `.`으로 시작하는 이름은 시스템 내부 서체라 고를 수 없게 숨긴다.
    let fonts = cx
        .text_system()
        .all_font_names()
        .into_iter()
        .filter(|name| !name.starts_with('.'))
        .map(|name| {
            let name = SharedString::from(name);
            FontItem {
                title: name.clone(),
                value: Some(name.clone()),
                family: name,
            }
        });
    std::iter::once(FontItem {
        title: default_title.into(),
        value: None,
        family: default_family,
    })
    .chain(fonts)
    .collect()
}

#[derive(Clone)]
struct FontSizeItem(u16);

impl SearchableListItem for FontSizeItem {
    type Value = u16;

    fn title(&self) -> SharedString {
        format!("{} px", self.0).into()
    }

    fn render(&self, _: &mut Window, _: &mut App) -> impl IntoElement {
        div().child(self.title())
    }

    fn value(&self) -> &u16 {
        &self.0
    }
}

/// 설정 창 화면.
struct SettingsWindow {
    page: Page,
    font_picker: Entity<SelectState<SearchableVec<FontItem>>>,
    font_size_picker: Entity<SelectState<SearchableVec<FontSizeItem>>>,
    _subscriptions: Vec<Subscription>,
}

impl SettingsWindow {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let system_family = cx.theme().font_family.clone();
        let items = font_items("시스템 서체", system_family, cx);
        let current = cx.global::<Prefs>().editor_font.clone();
        let selected = items
            .iter()
            .position(|item| item.value == current)
            .unwrap_or(0);
        let font_picker = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(items),
                Some(IndexPath::new(selected)),
                window,
                cx,
            )
            .searchable(true)
        });

        let current_size = Prefs::editor_font_size(cx) as usize;
        let font_size_picker = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new((12..=32).map(FontSizeItem).collect::<Vec<_>>()),
                Some(IndexPath::new(current_size - 12)),
                window,
                cx,
            )
        });
        let subscriptions = vec![
            cx.subscribe(
                &font_picker,
                |_, _, event: &SelectEvent<SearchableVec<FontItem>>, cx| {
                    let SelectEvent::Confirm(value) = event;
                    if let Some(font) = value {
                        Prefs::set_editor_font(font.clone(), cx);
                    }
                },
            ),
            cx.subscribe(
                &font_size_picker,
                |_, _, event: &SelectEvent<SearchableVec<FontSizeItem>>, cx| {
                    let SelectEvent::Confirm(value) = event;
                    if let Some(size) = value {
                        Prefs::set_editor_font_size(*size, cx);
                    }
                },
            ),
            cx.observe_global::<Prefs>(|_, cx| cx.notify()),
        ];

        Self {
            page: Page::General,
            font_picker,
            font_size_picker,
            _subscriptions: subscriptions,
        }
    }

    /// 왼쪽 떠 있는 페이지 패널(메인 창의 라이브러리 패널과 같은 모양).
    fn render_page_panel(&self, cx: &mut Context<Self>, pal: Palette) -> impl IntoElement {
        let rows = Page::ALL.into_iter().map(|page| {
            let selected = page == self.page;
            let (text_color, icon_color) = if selected {
                (pal.accent_foreground, pal.accent_foreground)
            } else {
                (pal.foreground, pal.foreground.opacity(0.7))
            };
            div()
                .id(page.title())
                .mx_2()
                .px_2()
                .h(px(30.0))
                .flex()
                .items_center()
                .gap_2()
                .rounded_lg()
                .text_sm()
                .text_color(text_color)
                .when(selected, |this| this.bg(pal.accent).font_semibold())
                .when(!selected, |this| {
                    this.hover(move |style| style.bg(pal.hover))
                })
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.page = page;
                    cx.notify();
                }))
                .child(Icon::new(page.icon()).small().text_color(icon_color))
                .child(page.title())
        });

        div().h_full().flex_shrink_0().p(px(PANEL_INSET)).child(
            div()
                .w(px(PAGE_PANEL_WIDTH))
                .h_full()
                .flex()
                .flex_col()
                .rounded_xl()
                .bg(pal.panel)
                .border_1()
                .border_color(pal.panel_border)
                .shadow_md()
                // 위쪽 줄: (macOS) 신호등 버튼 자리.
                .child(window_drag_area(
                    div()
                        .h(px(TOOLBAR_HEIGHT - PANEL_INSET * 2.0))
                        .flex_shrink_0(),
                ))
                .child(div().pt_1().flex().flex_col().gap_1().children(rows)),
        )
    }

    /// 일반: 에디터 서체와 글자 크기.
    fn render_general(&self) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                Select::new(&self.font_picker)
                    .large()
                    .focus_ring(false)
                    .menu_max_h(px(320.0))
                    .search_placeholder("서체 검색"),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child("에디터 폰트 크기")
                    .child(
                        Select::new(&self.font_size_picker)
                            .large()
                            .focus_ring(false)
                            .menu_max_h(px(240.0)),
                    ),
            )
    }
}

impl Render for SettingsWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pal = Palette::from_theme(cx.theme());
        let content = match self.page {
            Page::General => self.render_general().into_any_element(),
        };

        let body = div()
            .flex_1()
            .min_h_0()
            .flex()
            .child(self.render_page_panel(cx, pal))
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .flex()
                    .flex_col()
                    // 위쪽 빈 줄은 창을 끄는 자리(메인 창 툴바 높이와 맞춘다).
                    .child(window_drag_area(
                        div().h(px(TOOLBAR_HEIGHT)).flex_shrink_0(),
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .px_8()
                            .flex()
                            .justify_center()
                            .child(div().w_full().max_w(px(FORM_MAX_WIDTH)).child(content)),
                    ),
            );

        let root = div()
            .flex()
            .flex_col()
            .size_full()
            .bg(pal.background)
            .text_color(pal.foreground);

        #[cfg(not(target_os = "macos"))]
        let root = {
            let theme = cx.theme();
            root.child(
                TitleBar::new()
                    .bg(theme.title_bar)
                    .border_color(theme.title_bar_border)
                    .child(div().text_sm().child("설정")),
            )
        };

        root.child(body)
    }
}
