use gpui_kit::component::button::*;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::*;
use gpui_kit::gpui::prelude::FluentBuilder;
use gpui_kit::gpui::*;
use std::path::PathBuf;

#[cfg(not(target_os = "macos"))]
use gpui_kit::component::menu::AppMenuBar;

mod config;
mod icons;
mod library;
mod theme;

actions!(
    writer,
    [
        NewSheet,
        SaveSheet,
        OpenLibrary,
        CloseLibrary,
        OpenSettings,
        ToggleSidebar,
        TogglePreview,
        ToggleTheme,
        ShowAbout,
        QuitApp
    ]
);

/// 이름을 가진 테마 전환 액션 (메뉴에서 동적으로 생성).
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = writer, no_json)]
struct SetTheme {
    name: SharedString,
}

const APP_NAME: &str = "Writer";

/// 메뉴 트리.
///
/// - macOS: `Settings…`는 앱 메뉴(`Writer`)에 둔다 (Apple HIG).
/// - 그 외: `Edit > Settings…`에 둔다.
/// - 테마 목록은 `ThemeRegistry`에서 읽어 `View > Theme`에 채운다.
fn build_menus(theme_names: &[SharedString], active_theme: &str) -> Vec<Menu> {
    let theme_items: Vec<MenuItem> = theme_names
        .iter()
        .map(|name| {
            let checked = name == active_theme;
            MenuItem::action(name.clone(), SetTheme { name: name.clone() }).checked(checked)
        })
        .collect();

    let mut menus: Vec<Menu> = Vec::new();

    #[cfg(target_os = "macos")]
    menus.push(Menu::new(APP_NAME).items([
        MenuItem::action("About Writer", ShowAbout),
        MenuItem::separator(),
        MenuItem::action("Settings…", OpenSettings),
        MenuItem::separator(),
        MenuItem::action("Quit Writer", QuitApp),
    ]));

    let mut file_items = vec![
        MenuItem::action("New Sheet", NewSheet),
        MenuItem::action("Save", SaveSheet),
        MenuItem::separator(),
        MenuItem::action("Open Library…", OpenLibrary),
        MenuItem::action("Close Library", CloseLibrary),
    ];
    #[cfg(not(target_os = "macos"))]
    {
        file_items.push(MenuItem::separator());
        file_items.push(MenuItem::action("Quit", QuitApp));
    }
    menus.push(Menu::new("File").items(file_items));

    #[cfg(not(target_os = "macos"))]
    menus.push(Menu::new("Edit").items([MenuItem::action("Settings…", OpenSettings)]));

    let mut view_items = vec![
        MenuItem::action("Toggle Sidebar", ToggleSidebar),
        MenuItem::action("Toggle Preview", TogglePreview),
        MenuItem::action("Toggle Light/Dark", ToggleTheme),
    ];
    if !theme_items.is_empty() {
        view_items.push(MenuItem::separator());
        view_items.push(MenuItem::submenu(Menu::new("Theme").items(theme_items)));
    }
    menus.push(Menu::new("View").items(view_items));

    menus.push(Menu::new("Help").items([MenuItem::action("About Writer", ShowAbout)]));

    menus
}

/// 현재 테마를 반영해 앱 메뉴를 (재)설치한다.
fn install_menus(cx: &mut App) {
    let theme_names = theme::theme_names(cx);
    let active = theme::active_name(cx);

    cx.set_menus(build_menus(&theme_names, &active));
    let owned: Vec<OwnedMenu> = build_menus(&theme_names, &active)
        .into_iter()
        .map(|menu| menu.owned())
        .collect();
    GlobalState::global_mut(cx).set_app_menus(owned);
}

// --- 사이드바 렌더 헬퍼 -----------------------------------------------------

fn sheet_row(title: &str, indent: f32, foreground: Hsla) -> impl IntoElement {
    div()
        .pl(px(indent))
        .pr_2()
        .py_1()
        .rounded_md()
        .flex()
        .items_center()
        .gap_2()
        .text_sm()
        .text_color(foreground)
        .child(Icon::new(icons::sheet()).small().text_color(foreground))
        .child(title.to_string())
}

fn render_section(section: &library::Section, muted: Hsla, foreground: Hsla) -> AnyElement {
    let mut rows: Vec<AnyElement> = Vec::new();
    for entry in &section.entries {
        match entry {
            library::Entry::Sheet(sheet) => {
                rows.push(sheet_row(&sheet.title, 8.0, foreground).into_any_element());
            }
            library::Entry::Group(group) => {
                rows.push(
                    div()
                        .pl(px(8.0))
                        .py_1()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_sm()
                        .text_color(muted)
                        .child(Icon::new(icons::group()).small().text_color(muted))
                        .child(group.name.clone())
                        .into_any_element(),
                );
                for sheet in &group.sheets {
                    rows.push(sheet_row(&sheet.title, 24.0, foreground).into_any_element());
                }
            }
        }
    }

    div()
        .flex()
        .flex_col()
        .gap_1()
        .mb_2()
        .child(
            div()
                .px_2()
                .py_1()
                .flex()
                .items_center()
                .gap_2()
                .text_xs()
                .font_semibold()
                .text_color(muted)
                .child(
                    Icon::new(icons::section(section.kind))
                        .small()
                        .text_color(muted),
                )
                .child(section.kind.title()),
        )
        .children(rows)
        .into_any_element()
}

struct WriterApp {
    status_text: String,
    sidebar_visible: bool,
    preview_visible: bool,
    library: Option<library::Library>,
    /// 온보딩 화면에 표시할 안내/오류 문구.
    notice: Option<SharedString>,
    #[cfg(not(target_os = "macos"))]
    menu_bar: Entity<AppMenuBar>,
}

impl WriterApp {
    fn notify(
        &mut self,
        message: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let message = message.into();
        self.status_text = message.to_string();
        window.push_notification(Notification::info(message), cx);
        cx.notify();
    }

    fn set_notice(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.notice = Some(message.into());
        cx.notify();
    }

    // --- 라이브러리 폴더 설정 ---------------------------------------------

    /// 기본 제안 경로에 새 라이브러리를 만든다.
    fn create_default_library(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = config::default_library_root() else {
            self.set_notice("기본 라이브러리 위치를 찾을 수 없습니다.", cx);
            return;
        };
        self.initialize_at(root, window, cx);
    }

    /// 라이브러리 골격을 만든 뒤 연다.
    fn initialize_at(&mut self, root: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        match library::initialize(&root) {
            Ok(()) => self.adopt(root, window, cx),
            Err(err) => self.set_notice(format!("라이브러리를 만들 수 없습니다: {err}"), cx),
        }
    }

    /// 기존 폴더를 검사해 알맞게 연다.
    fn open_folder(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        match library::inspect(&path) {
            Ok(library::FolderKind::Library) => self.adopt(path, window, cx),
            Ok(library::FolderKind::Empty) | Ok(library::FolderKind::MarkdownOnly) => {
                self.initialize_at(path, window, cx);
            }
            // 비어 있지 않은 폴더: 하위에 Writer/ 를 만든다 (ADR-0003 D3).
            Ok(library::FolderKind::NonEmpty) => {
                self.initialize_at(path.join(APP_NAME), window, cx);
            }
            Err(err) => self.set_notice(format!("폴더를 사용할 수 없습니다: {err}"), cx),
        }
    }

    /// 라이브러리를 현재 라이브러리로 채택하고 설정에 저장한다.
    fn adopt(&mut self, root: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        match library::scan(&root) {
            Ok(opened) => {
                let mut config = config::Config::load();
                config.set_library(opened.root.clone());
                if let Err(err) = config.save() {
                    eprintln!("설정 저장 실패: {err}");
                }
                window.push_notification(
                    Notification::info(format!("라이브러리 열기: {}", opened.root.display())),
                    cx,
                );
                self.library = Some(opened);
                self.notice = None;
                cx.notify();
            }
            Err(err) => self.set_notice(format!("라이브러리를 읽을 수 없습니다: {err}"), cx),
        }
    }

    /// OS 네이티브 폴더 선택기로 라이브러리 폴더를 고른다.
    fn pick_library(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("라이브러리 폴더 선택".into()),
        });

        cx.spawn_in(window, async move |this, cx| {
            let picked = match receiver.await {
                Ok(Ok(Some(mut paths))) => paths.pop(),
                _ => None,
            };
            let Some(path) = picked else {
                return;
            };
            this.update_in(cx, |this, window, cx| this.open_folder(path, window, cx))
                .ok();
        })
        .detach();
    }

    // --- 액션 핸들러 --------------------------------------------------------

    fn on_new_sheet(&mut self, _: &NewSheet, window: &mut Window, cx: &mut Context<Self>) {
        self.notify("새 문서를 준비했습니다.", window, cx);
    }

    fn on_save_sheet(&mut self, _: &SaveSheet, window: &mut Window, cx: &mut Context<Self>) {
        self.notify("문서가 저장되었습니다.", window, cx);
    }

    fn on_open_library(&mut self, _: &OpenLibrary, window: &mut Window, cx: &mut Context<Self>) {
        self.pick_library(window, cx);
    }

    fn on_close_library(&mut self, _: &CloseLibrary, _: &mut Window, cx: &mut Context<Self>) {
        self.library = None;
        self.notice = None;
        cx.notify();
    }

    fn on_open_settings(&mut self, _: &OpenSettings, window: &mut Window, cx: &mut Context<Self>) {
        // TODO(M8): 실제 환경설정 페이지로 교체
        self.notify("환경설정 (준비 중)", window, cx);
    }

    fn on_show_about(&mut self, _: &ShowAbout, window: &mut Window, cx: &mut Context<Self>) {
        self.notify(
            format!("{APP_NAME} v0.1.0 · 아이콘: {}", icons::PACK),
            window,
            cx,
        );
    }

    fn on_toggle_sidebar(&mut self, _: &ToggleSidebar, _: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_visible = !self.sidebar_visible;
        cx.notify();
    }

    fn on_toggle_preview(&mut self, _: &TogglePreview, _: &mut Window, cx: &mut Context<Self>) {
        self.preview_visible = !self.preview_visible;
        cx.notify();
    }

    fn on_set_theme(&mut self, action: &SetTheme, window: &mut Window, cx: &mut Context<Self>) {
        theme::set_theme(&action.name, cx);
        let name = action.name.clone();
        self.after_theme_change(format!("테마: {name}"), window, cx);
    }

    fn on_toggle_theme(&mut self, _: &ToggleTheme, window: &mut Window, cx: &mut Context<Self>) {
        let next = if Theme::global(cx).is_dark() {
            theme::DEFAULT_LIGHT
        } else {
            theme::DEFAULT_DARK
        };
        theme::set_theme(next, cx);
        self.after_theme_change(format!("테마: {next}"), window, cx);
    }

    fn after_theme_change(
        &mut self,
        message: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        install_menus(cx);
        #[cfg(not(target_os = "macos"))]
        self.menu_bar.update(cx, |menu_bar, cx| menu_bar.reload(cx));
        self.notify(message, window, cx);
    }

    fn on_quit(&mut self, _: &QuitApp, _: &mut Window, cx: &mut Context<Self>) {
        cx.quit();
    }

    // --- 렌더 헬퍼 ---------------------------------------------------------

    fn render_sidebar(
        &self,
        muted: Hsla,
        foreground: Hsla,
        background: Hsla,
        border: Hsla,
    ) -> impl IntoElement {
        let sections: Vec<AnyElement> = self
            .library
            .as_ref()
            .map(|lib| {
                lib.sections
                    .iter()
                    .map(|section| render_section(section, muted, foreground))
                    .collect()
            })
            .unwrap_or_default();

        div()
            .w(px(220.0))
            .h_full()
            .p_3()
            .border_r_1()
            .border_color(border)
            .bg(background)
            .child(
                div()
                    .text_xs()
                    .font_semibold()
                    .text_color(muted)
                    .mb_2()
                    .child("LIBRARY"),
            )
            .children(sections)
    }

    fn render_workspace(
        &self,
        muted: Hsla,
        foreground: Hsla,
        sidebar_background: Hsla,
        border: Hsla,
        panel_background: Hsla,
    ) -> impl IntoElement {
        let root_label = self
            .library
            .as_ref()
            .map(|lib| lib.root.display().to_string())
            .unwrap_or_default();

        div()
            .flex_1()
            .flex()
            .size_full()
            .when(self.sidebar_visible, |this| {
                this.child(self.render_sidebar(
                    muted,
                    foreground,
                    sidebar_background,
                    border,
                ))
            })
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .p_6()
                    .flex()
                    .flex_col()
                    .bg(panel_background)
                    .child(
                        div()
                            .text_xl()
                            .font_bold()
                            .mb_4()
                            .child("라이브러리 준비됨"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .mb_3()
                            .child(root_label),
                    )
                    .child(
                        div()
                            .text_color(muted)
                            .line_height(relative(1.6))
                            .child("왼쪽에서 시트를 선택하면 여기에 표시됩니다.\n에디터 연결은 M2에서 진행합니다."),
                    ),
            )
            .when(self.preview_visible, |this| {
                this.child(
                    div()
                        .w(px(320.0))
                        .h_full()
                        .p_4()
                        .border_l_1()
                        .border_color(border)
                        .bg(sidebar_background)
                        .child(
                            div()
                                .text_xs()
                                .font_semibold()
                                .text_color(muted)
                                .mb_2()
                                .child("PREVIEW"),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(muted)
                                .child("프리뷰 영역 (M4)"),
                        ),
                )
            })
    }
}

impl Render for WriterApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 테마 토큰을 값(Copy)으로 읽어 둔다. 이후 `cx.listener`와 대여 충돌이 없다.
        let theme = cx.theme();
        let background = theme.background;
        let foreground = theme.foreground;
        let border = theme.border;
        let muted_foreground = theme.muted_foreground;
        let sidebar_background = theme.sidebar;
        let panel_background = theme.popover;
        let status_background = theme.status_bar;
        let status_border = theme.status_bar_border;
        let title_bar_background = theme.title_bar;
        let title_bar_border = theme.title_bar_border;
        let danger = theme.danger;

        let root = div()
            .flex()
            .flex_col()
            .size_full()
            .bg(background)
            .text_color(foreground)
            .on_action(cx.listener(Self::on_new_sheet))
            .on_action(cx.listener(Self::on_save_sheet))
            .on_action(cx.listener(Self::on_open_library))
            .on_action(cx.listener(Self::on_close_library))
            .on_action(cx.listener(Self::on_open_settings))
            .on_action(cx.listener(Self::on_show_about))
            .on_action(cx.listener(Self::on_toggle_sidebar))
            .on_action(cx.listener(Self::on_toggle_preview))
            .on_action(cx.listener(Self::on_toggle_theme))
            .on_action(cx.listener(Self::on_set_theme))
            .on_action(cx.listener(Self::on_quit));

        // 1. 커스텀 타이틀바 + 메뉴바 (macOS는 네이티브 타이틀바/메뉴를 쓰므로 생략)
        #[cfg(not(target_os = "macos"))]
        let root = root.child(
            TitleBar::new()
                .bg(title_bar_background)
                .border_color(title_bar_border)
                .child(self.menu_bar.clone()),
        );

        // 2. 바디: 라이브러리가 있으면 워크스페이스, 없으면 온보딩
        let body: AnyElement =
            if self.library.is_some() {
                self.render_workspace(
                    muted_foreground,
                    foreground,
                    sidebar_background,
                    border,
                    panel_background,
                )
                .into_any_element()
            } else {
                let suggested = config::default_library_root()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "(위치를 찾을 수 없음)".to_string());

                div()
                    .flex_1()
                    .size_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_4()
                    .child(div().text_2xl().font_bold().child(APP_NAME))
                    .child(
                        div()
                            .text_sm()
                            .text_color(muted_foreground)
                            .child("모든 메모와 프로젝트를 담을 폴더를 선택하세요."),
                    )
                    .child(
                        div()
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .border_1()
                            .border_color(border)
                            .bg(panel_background)
                            .text_xs()
                            .text_color(muted_foreground)
                            .child(format!("제안 위치: {suggested}")),
                    )
                    .when_some(self.notice.clone(), |this, notice| {
                        this.child(div().text_sm().text_color(danger).child(notice))
                    })
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                Button::new("create-library")
                                    .primary()
                                    .label("라이브러리 만들기")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.create_default_library(window, cx)
                                    })),
                            )
                            .child(
                                Button::new("open-library")
                                    .label("기존 라이브러리 열기")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.pick_library(window, cx)
                                    })),
                            ),
                    )
                    .into_any_element()
            };

        root.child(body)
            // 3. 하단 상태 바
            .child(
                div()
                    .h(px(26.0))
                    .px_3()
                    .flex()
                    .items_center()
                    .justify_between()
                    .bg(status_background)
                    .border_t_1()
                    .border_color(status_border)
                    .text_xs()
                    .text_color(muted_foreground)
                    .child(div().child(self.status_text.clone()))
                    .child(div().child("UTF-8 | Markdown | GPUI")),
            )
    }
}

fn main() {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(|cx| {
            gpui_kit::init(cx);

            // 테마 등록 후 기본 다크 테마 적용
            theme::register(cx);
            theme::set_theme(theme::DEFAULT_DARK, cx);

            // macOS: 네이티브 메뉴 / Windows·Linux: 인윈도우 AppMenuBar용 메뉴 등록
            install_menus(cx);

            // 최소 단축키
            cx.bind_keys([
                KeyBinding::new("ctrl-n", NewSheet, None),
                KeyBinding::new("ctrl-s", SaveSheet, None),
                KeyBinding::new("ctrl-o", OpenLibrary, None),
                KeyBinding::new("ctrl-,", OpenSettings, None),
                KeyBinding::new("ctrl-b", ToggleSidebar, None),
                KeyBinding::new("ctrl-shift-l", ToggleTheme, None),
            ]);

            // 저장된 라이브러리를 열어본다(없거나 실패하면 온보딩).
            let config = config::Config::load();
            let library = config
                .library_root
                .as_deref()
                .and_then(|root| library::scan(root).ok());
            let notice = if config.library_root.is_some() && library.is_none() {
                Some(SharedString::from(
                    "저장된 라이브러리를 열 수 없습니다. 폴더를 다시 선택하세요.",
                ))
            } else {
                None
            };

            let window_bounds = Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(1024.0), px(680.0)),
                cx,
            )));

            // Windows/Linux: 네이티브 타이틀바를 숨기고 커스텀 타이틀바를 그린다.
            #[cfg(not(target_os = "macos"))]
            let options = WindowOptions {
                window_bounds,
                window_min_size: Some(size(px(640.0), px(400.0))),
                ..TitleBar::window_options()
            };

            // macOS: 네이티브 타이틀바 유지
            #[cfg(target_os = "macos")]
            let options = WindowOptions {
                window_bounds,
                titlebar: Some(TitlebarOptions {
                    title: Some(APP_NAME.into()),
                    appears_transparent: false,
                    traffic_light_position: None,
                }),
                ..Default::default()
            };

            gpui_kit::open_window(options, cx, |_, cx| {
                cx.new(|_cx| WriterApp {
                    status_text: "준비 완료".to_string(),
                    sidebar_visible: true,
                    preview_visible: false,
                    library,
                    notice,
                    #[cfg(not(target_os = "macos"))]
                    menu_bar: AppMenuBar::new(_cx),
                })
            })
            .expect("창 열기에 실패했습니다");
        });
}
