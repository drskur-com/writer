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
use crate::aws_profiles::{self, AwsProfile};
use crate::config::Config;
use crate::i18n::{self, Language};
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
    /// 어시스턴트가 쓸 AI 제공자. 없으면 쓰지 않는다.
    pub ai_provider: Option<AiProvider>,
    /// Bedrock에 쓸 AWS 프로필 이름.
    pub aws_profile: Option<SharedString>,
    /// Bedrock을 부를 리전. 없으면 프로필의 리전을 따른다.
    pub aws_region: Option<SharedString>,
    /// 어시스턴트가 부를 Bedrock 모델(추론 프로필) ID. 어시스턴트 패널에서 고른다.
    pub ai_model: Option<SharedString>,
    /// UI 언어(ADR-0027).
    pub language: Language,
}

impl Global for Prefs {}

/// 어시스턴트가 쓸 AI 제공자.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AiProvider {
    /// Amazon Bedrock(AWS 프로필의 자격 증명으로 부른다).
    Bedrock,
}

impl AiProvider {
    const ALL: [AiProvider; 1] = [AiProvider::Bedrock];

    /// `config.toml`에 적는 이름.
    fn key(self) -> &'static str {
        match self {
            AiProvider::Bedrock => "bedrock",
        }
    }

    fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|provider| provider.key() == key)
    }

    fn title(self) -> &'static str {
        match self {
            AiProvider::Bedrock => "Amazon Bedrock",
        }
    }
}

impl Prefs {
    /// 설정 파일 값으로 전역 설정을 만든다.
    pub fn init(config: &Config, cx: &mut App) {
        cx.set_global(Prefs {
            editor_font: config.editor_font.clone().map(SharedString::from),
            editor_font_size: config
                .editor_font_size
                .filter(|size| (12..=32).contains(size)),
            ai_provider: config.ai_provider.as_deref().and_then(AiProvider::from_key),
            aws_profile: config.aws_profile.clone().map(SharedString::from),
            aws_region: config.aws_region.clone().map(SharedString::from),
            ai_model: config.ai_model.clone().map(SharedString::from),
            language: Language::from_key(config.language.as_deref()),
        });
        rust_i18n::set_locale(cx.global::<Prefs>().language.locale());
    }

    /// UI 언어를 바꾸고 저장한다. 메뉴와 창은 바로 바뀐다.
    fn set_language(language: Language, cx: &mut App) {
        if cx.global::<Prefs>().language == language {
            return;
        }
        cx.global_mut::<Prefs>().language = language;
        let mut config = Config::load();
        config.language = language.key().map(str::to_string);
        if let Err(err) = config.save() {
            eprintln!("설정 저장 실패: {err}");
        }
        i18n::apply(language, cx);
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

    /// AI 제공자를 바꾸고 저장한다.
    fn set_ai_provider(provider: Option<AiProvider>, cx: &mut App) {
        if cx.global::<Prefs>().ai_provider == provider {
            return;
        }
        cx.global_mut::<Prefs>().ai_provider = provider;
        let mut config = Config::load();
        config.ai_provider = provider.map(|provider| provider.key().to_string());
        if let Err(err) = config.save() {
            eprintln!("설정 저장 실패: {err}");
        }
    }

    /// Bedrock에 쓸 AWS 프로필을 바꾸고 저장한다.
    fn set_aws_profile(profile: Option<SharedString>, cx: &mut App) {
        if cx.global::<Prefs>().aws_profile == profile {
            return;
        }
        cx.global_mut::<Prefs>().aws_profile = profile.clone();
        let mut config = Config::load();
        config.aws_profile = profile.map(|profile| profile.to_string());
        if let Err(err) = config.save() {
            eprintln!("설정 저장 실패: {err}");
        }
    }

    /// Bedrock 리전을 바꾸고 저장한다. `None`이면 프로필의 리전을 따른다.
    fn set_aws_region(region: Option<SharedString>, cx: &mut App) {
        if cx.global::<Prefs>().aws_region == region {
            return;
        }
        cx.global_mut::<Prefs>().aws_region = region.clone();
        let mut config = Config::load();
        config.aws_region = region.map(|region| region.to_string());
        if let Err(err) = config.save() {
            eprintln!("설정 저장 실패: {err}");
        }
    }

    /// 어시스턴트가 부를 Bedrock 모델(추론 프로필) ID를 바꾸고 저장한다.
    pub(crate) fn set_ai_model(model: Option<SharedString>, cx: &mut App) {
        if cx.global::<Prefs>().ai_model == model {
            return;
        }
        cx.global_mut::<Prefs>().ai_model = model.clone();
        let mut config = Config::load();
        config.ai_model = model.map(|model| model.to_string());
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
            title: Some(t!("settings.title").into()),
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
    Ai,
}

impl Page {
    const ALL: [Page; 2] = [Page::General, Page::Ai];

    fn id(self) -> &'static str {
        match self {
            Page::General => "general",
            Page::Ai => "ai",
        }
    }

    fn title(self) -> SharedString {
        match self {
            Page::General => t!("settings.page.general").into(),
            Page::Ai => t!("settings.page.ai").into(),
        }
    }

    fn icon(self) -> AssetIcon {
        match self {
            Page::General => icons::settings(),
            Page::Ai => icons::ai_settings(),
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
    default_title: impl Into<SharedString>,
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

/// 이름만 있는 선택지(AI 제공자, AWS 프로필). 값이 `None`이면 "사용 안 함" 같은 비움 항목.
#[derive(Clone)]
struct ChoiceItem {
    title: SharedString,
    value: Option<SharedString>,
}

impl SearchableListItem for ChoiceItem {
    type Value = Option<SharedString>;

    fn title(&self) -> SharedString {
        self.title.clone()
    }

    fn render(&self, _: &mut Window, _: &mut App) -> impl IntoElement {
        div().child(self.title.clone())
    }

    fn value(&self) -> &Self::Value {
        &self.value
    }
}

/// 설정 창 화면.
struct SettingsWindow {
    page: Page,
    /// 선택지를 만들 때의 언어. 바뀌면 선택지를 다시 만든다(이름이 그 언어로 들어 있다).
    language: Language,
    language_picker: Entity<SelectState<SearchableVec<ChoiceItem>>>,
    font_picker: Entity<SelectState<SearchableVec<FontItem>>>,
    font_size_picker: Entity<SelectState<SearchableVec<FontSizeItem>>>,
    provider_picker: Entity<SelectState<SearchableVec<ChoiceItem>>>,
    profile_picker: Entity<SelectState<SearchableVec<ChoiceItem>>>,
    region_picker: Entity<SelectState<SearchableVec<ChoiceItem>>>,
    /// 창을 열 때 읽은 AWS 프로필.
    aws_profiles: Vec<AwsProfile>,
    _subscriptions: Vec<Subscription>,
}

impl SettingsWindow {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        // 언어: 시스템, English, 한국어.
        let language = cx.global::<Prefs>().language;
        let languages: Vec<_> = Language::ALL
            .into_iter()
            .map(|language| ChoiceItem {
                title: language.title().into(),
                value: language.key().map(SharedString::from),
            })
            .collect();
        let selected_language = Language::ALL
            .iter()
            .position(|item| *item == language)
            .unwrap_or(0);
        let language_picker = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(languages),
                Some(IndexPath::new(selected_language)),
                window,
                cx,
            )
        });
        let system_family = cx.theme().font_family.clone();
        let items = font_items(t!("settings.font.system"), system_family, cx);
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
        // AI 제공자: 맨 앞은 "사용 안 함".
        let providers: Vec<_> = std::iter::once(ChoiceItem {
            title: t!("settings.ai.provider.none").into(),
            value: None,
        })
        .chain(AiProvider::ALL.into_iter().map(|provider| ChoiceItem {
            title: provider.title().into(),
            value: Some(provider.key().into()),
        }))
        .collect();
        let current_provider = cx.global::<Prefs>().ai_provider.map(|p| p.key());
        let selected_provider = providers
            .iter()
            .position(|item| item.value.as_deref() == current_provider)
            .unwrap_or(0);
        let provider_picker = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(providers),
                Some(IndexPath::new(selected_provider)),
                window,
                cx,
            )
        });

        // AWS 프로필: 저장한 프로필이 목록에 없으면 고르지 않은 상태로 둔다.
        let aws_profiles = aws_profiles::list();
        let current_profile = cx.global::<Prefs>().aws_profile.clone();
        let selected_profile = aws_profiles
            .iter()
            .position(|profile| Some(profile.name.as_str()) == current_profile.as_deref());
        let profile_items: Vec<_> = aws_profiles
            .iter()
            .map(|profile| ChoiceItem {
                title: profile.name.clone().into(),
                value: Some(profile.name.clone().into()),
            })
            .collect();
        let profile_picker = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(profile_items),
                selected_profile.map(IndexPath::new),
                window,
                cx,
            )
            .searchable(true)
        });

        // 리전: 맨 앞은 "프로필 리전 따르기". 저장한 리전이 목록에 없으면(직접 고친 설정) 끝에 더한다.
        let current_region = cx.global::<Prefs>().aws_region.clone();
        let mut regions: Vec<_> = std::iter::once(ChoiceItem {
            title: t!("settings.ai.region.follow_profile").into(),
            value: None,
        })
        .chain(
            aws_profiles::BEDROCK_REGIONS
                .iter()
                .map(|(code, name)| ChoiceItem {
                    title: format!("{name} · {code}").into(),
                    value: Some((*code).into()),
                }),
        )
        .collect();
        if let Some(region) = &current_region
            && !regions
                .iter()
                .any(|item| item.value.as_ref() == Some(region))
        {
            regions.push(ChoiceItem {
                title: region.clone(),
                value: Some(region.clone()),
            });
        }
        let selected_region = regions
            .iter()
            .position(|item| item.value == current_region)
            .unwrap_or(0);
        let region_picker = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(regions),
                Some(IndexPath::new(selected_region)),
                window,
                cx,
            )
            .searchable(true)
        });

        let subscriptions = vec![
            cx.subscribe(
                &language_picker,
                |_, _, event: &SelectEvent<SearchableVec<ChoiceItem>>, cx| {
                    let SelectEvent::Confirm(value) = event;
                    if let Some(key) = value {
                        Prefs::set_language(Language::from_key(key.as_deref()), cx);
                    }
                },
            ),
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
            cx.subscribe(
                &provider_picker,
                |_, _, event: &SelectEvent<SearchableVec<ChoiceItem>>, cx| {
                    let SelectEvent::Confirm(value) = event;
                    if let Some(key) = value {
                        Prefs::set_ai_provider(key.as_deref().and_then(AiProvider::from_key), cx);
                    }
                },
            ),
            cx.subscribe(
                &profile_picker,
                |_, _, event: &SelectEvent<SearchableVec<ChoiceItem>>, cx| {
                    let SelectEvent::Confirm(value) = event;
                    if let Some(profile) = value {
                        Prefs::set_aws_profile(profile.clone(), cx);
                    }
                },
            ),
            cx.subscribe(
                &region_picker,
                |_, _, event: &SelectEvent<SearchableVec<ChoiceItem>>, cx| {
                    let SelectEvent::Confirm(value) = event;
                    if let Some(region) = value {
                        Prefs::set_aws_region(region.clone(), cx);
                    }
                },
            ),
            // 언어가 바뀌면 선택지 이름도 그 언어로 다시 만든다. 보던 페이지는 그대로 둔다.
            cx.observe_global_in::<Prefs>(window, |this, window, cx| {
                if cx.global::<Prefs>().language != this.language {
                    let page = this.page;
                    *this = Self::new(window, cx);
                    this.page = page;
                }
                cx.notify();
            }),
        ];

        Self {
            page: Page::General,
            language,
            language_picker,
            font_picker,
            font_size_picker,
            provider_picker,
            profile_picker,
            region_picker,
            aws_profiles,
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
                .id(page.id())
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

    /// 일반: 언어, 에디터 서체와 글자 크기.
    fn render_general(&self) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(t!("settings.language"))
                    .child(Select::new(&self.language_picker).large().focus_ring(false)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(t!("settings.font"))
                    .child(
                        Select::new(&self.font_picker)
                            .large()
                            .focus_ring(false)
                            .menu_max_h(px(320.0))
                            .search_placeholder(t!("settings.font.search")),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(t!("settings.font.size"))
                    .child(
                        Select::new(&self.font_size_picker)
                            .large()
                            .focus_ring(false)
                            .menu_max_h(px(240.0)),
                    ),
            )
    }
}

impl SettingsWindow {
    /// AI: 어시스턴트가 쓸 제공자와, Bedrock이면 AWS 프로필.
    fn render_ai(&self, cx: &App, pal: Palette) -> impl IntoElement {
        let prefs = cx.global::<Prefs>();
        let saved_profile = prefs.aws_profile.clone();
        let profile = saved_profile.as_ref().and_then(|name| {
            self.aws_profiles
                .iter()
                .find(|profile| profile.name == name.as_ref())
        });
        // 프로필 아래 안내: 프로필이 없거나 사라졌다는 안내, 아니면 프로필을 읽는 곳.
        let hint: Option<SharedString> = if self.aws_profiles.is_empty() {
            Some(t!("settings.ai.profile.hint_none").into())
        } else if profile.is_some() {
            None
        } else if let Some(name) = &saved_profile {
            Some(t!("settings.ai.profile.hint_missing", name = name).into())
        } else {
            Some(t!("settings.ai.profile.hint_source").into())
        };
        // 리전 아래 안내: 프로필 리전을 따를 때 실제로 쓰일 리전.
        let region_hint: Option<SharedString> = match (&prefs.aws_region, profile) {
            (Some(_), _) => None,
            (None, Some(profile)) => Some(match &profile.region {
                Some(region) => t!("settings.ai.region.hint_profile", region = region).into(),
                None => t!("settings.ai.region.hint_env").into(),
            }),
            (None, None) => Some(t!("settings.ai.region.hint_pick").into()),
        };

        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(t!("settings.ai.provider"))
                    .child(Select::new(&self.provider_picker).large().focus_ring(false)),
            )
            .when(prefs.ai_provider == Some(AiProvider::Bedrock), |this| {
                this.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(t!("settings.ai.profile"))
                        .child(
                            Select::new(&self.profile_picker)
                                .large()
                                .focus_ring(false)
                                .disabled(self.aws_profiles.is_empty())
                                .placeholder(t!("settings.ai.profile.placeholder"))
                                .menu_max_h(px(240.0))
                                .search_placeholder(t!("settings.ai.profile.search")),
                        )
                        .children(
                            hint.map(|hint| div().text_sm().text_color(pal.muted).child(hint)),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(t!("settings.ai.region"))
                        .child(
                            Select::new(&self.region_picker)
                                .large()
                                .focus_ring(false)
                                .menu_max_h(px(240.0))
                                .search_placeholder(t!("settings.ai.region.search")),
                        )
                        .children(
                            region_hint
                                .map(|hint| div().text_sm().text_color(pal.muted).child(hint)),
                        ),
                )
            })
    }
}

impl Render for SettingsWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pal = Palette::from_theme(cx.theme());
        let content = match self.page {
            Page::General => self.render_general().into_any_element(),
            Page::Ai => self.render_ai(cx, pal).into_any_element(),
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
                        // 폼이 창보다 길면(AI 페이지) 세로로 스크롤한다.
                        div()
                            .id("settings-form")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .px_8()
                            .pb_8()
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
                    .child(div().text_sm().child(t!("settings.title"))),
            )
        };

        root.child(body)
    }
}
