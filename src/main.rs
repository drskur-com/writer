use gpui_kit::component::notification::Notification;
use gpui_kit::component::*;
use gpui_kit::gpui::prelude::FluentBuilder;
use gpui_kit::gpui::*;

#[cfg(not(target_os = "macos"))]
use gpui_kit::component::menu::AppMenuBar;

mod theme;

actions!(
    writer,
    [
        NewSheet,
        SaveSheet,
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
///
/// macOS는 네이티브 메뉴, Windows/Linux는 인윈도우 `AppMenuBar`가 읽는
/// `GlobalState` 메뉴를 함께 갱신한다.
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

struct WriterApp {
    status_text: String,
    sidebar_visible: bool,
    preview_visible: bool,
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

    fn on_new_sheet(&mut self, _: &NewSheet, window: &mut Window, cx: &mut Context<Self>) {
        self.notify("새 문서를 준비했습니다.", window, cx);
    }

    fn on_save_sheet(&mut self, _: &SaveSheet, window: &mut Window, cx: &mut Context<Self>) {
        self.notify("문서가 저장되었습니다.", window, cx);
    }

    fn on_open_settings(&mut self, _: &OpenSettings, window: &mut Window, cx: &mut Context<Self>) {
        // TODO(M8): 실제 환경설정 페이지로 교체
        self.notify("환경설정 (준비 중)", window, cx);
    }

    fn on_show_about(&mut self, _: &ShowAbout, window: &mut Window, cx: &mut Context<Self>) {
        self.notify(format!("{APP_NAME} v0.1.0"), window, cx);
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

    /// 테마 변경 후 메뉴(체크 표시)를 갱신하고 알린다.
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
        let sidebar_foreground = theme.sidebar_foreground;
        let panel_background = theme.popover;
        let selected_background = theme.secondary;
        let status_background = theme.status_bar;
        let status_border = theme.status_bar_border;
        let title_bar_background = theme.title_bar;
        let title_bar_border = theme.title_bar_border;

        let root = div()
            .flex()
            .flex_col()
            .size_full()
            .bg(background)
            .text_color(foreground)
            .on_action(cx.listener(Self::on_new_sheet))
            .on_action(cx.listener(Self::on_save_sheet))
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

        // 2. 메인 바디 (사이드바 + 에디터 + 프리뷰)
        root.child(
            div()
                .flex_1()
                .flex()
                .size_full()
                .when(self.sidebar_visible, |this| {
                    this.child(
                        div()
                            .w(px(220.0))
                            .h_full()
                            .p_3()
                            .border_r_1()
                            .border_color(border)
                            .bg(sidebar_background)
                            .text_color(sidebar_foreground)
                            .child(
                                div()
                                    .text_xs()
                                    .font_semibold()
                                    .text_color(muted_foreground)
                                    .mb_2()
                                    .child("EXPLORER"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(
                                        div()
                                            .px_2()
                                            .py_1()
                                            .rounded_md()
                                            .bg(selected_background)
                                            .text_sm()
                                            .child("📄 Untitled-1.md"),
                                    ),
                            ),
                    )
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
                                .child("GPUI Editor Ready"),
                        )
                        .child(
                            div()
                                .text_color(muted_foreground)
                                .line_height(relative(1.6))
                                .child("gpui-kit 기반 에디터 앱 껍데기가 성공적으로 구동되었습니다.\n여기에 에디터 버퍼와 컴포넌트를 연결할 수 있습니다."),
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
                                    .text_color(muted_foreground)
                                    .mb_2()
                                    .child("PREVIEW"),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(muted_foreground)
                                    .child("프리뷰 영역 (M4)"),
                            ),
                    )
                }),
        )
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
        .with_assets(gpui_kit::assets::Assets)
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
                KeyBinding::new("ctrl-,", OpenSettings, None),
                KeyBinding::new("ctrl-b", ToggleSidebar, None),
                KeyBinding::new("ctrl-shift-l", ToggleTheme, None),
            ]);

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
                    #[cfg(not(target_os = "macos"))]
                    menu_bar: AppMenuBar::new(_cx),
                })
            })
            .expect("창 열기에 실패했습니다");
        });
}
