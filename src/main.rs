#![recursion_limit = "512"]

#[cfg(not(target_os = "macos"))]
use gpui_kit::component::*;
use gpui_kit::gpui::*;

mod app;
mod assets;
pub mod export;
mod config;
mod dock_icon;
mod icons;
mod library;
mod markdown;
mod menus;
mod settings;
mod text_transform;
mod theme;
mod thumbnail;
mod watch;

use app::WriterApp;
#[cfg(target_os = "macos")]
use app::style::{PANEL_INSET, TOOLBAR_HEIGHT};
use menus::install_menus;

actions!(
    writer,
    [
        NewSheet,
        SaveSheet,
        InsertImage,
        InsertFootnote,
        InsertTable,
        InsertSpecialChar,
        ToggleLineGroup,
        CompileProject,
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

fn main() {
    gpui_kit::application()
        .with_assets(assets::Assets)
        .run(|cx| {
            gpui_kit::init(cx);
            // 웹 주소 이미지(`img(url)`, 이미지 삽입의 내려받기)에 쓰는 HTTP 클라이언트.
            match reqwest_client::ReqwestClient::user_agent(&format!("{APP_NAME}/0.1")) {
                Ok(client) => cx.set_http_client(std::sync::Arc::new(client)),
                Err(err) => eprintln!("HTTP 클라이언트를 만들 수 없습니다: {err}"),
            }
            dock_icon::install();

            // 테마 등록 후 저장된 테마(없거나 모르는 이름이면 기본 라이트) 적용
            let config = config::Config::load();
            theme::register(cx);
            let restored = config
                .theme
                .as_deref()
                .is_some_and(|name| theme::set_theme(name, cx));
            if !restored {
                theme::set_theme(theme::DEFAULT_LIGHT, cx);
            }

            // macOS: 네이티브 메뉴 / Windows·Linux: 인윈도우 AppMenuBar용 메뉴 등록
            install_menus(cx);
            settings::Prefs::init(&config, cx);
            // 설정 창은 어느 창에 포커스가 있든(또는 없든) 열 수 있어야 하므로 앱 전역에서 받는다.
            cx.on_action(|_: &OpenSettings, cx| settings::open(cx));

            // 최소 단축키
            cx.bind_keys([
                KeyBinding::new("ctrl-n", NewSheet, None),
                KeyBinding::new("ctrl-s", SaveSheet, None),
                KeyBinding::new("ctrl-o", OpenLibrary, None),
                KeyBinding::new("ctrl-,", OpenSettings, None),
                #[cfg(target_os = "macos")]
                KeyBinding::new("cmd-,", OpenSettings, None),
                KeyBinding::new("ctrl-b", ToggleSidebar, None),
                KeyBinding::new("ctrl-shift-l", ToggleTheme, None),
                KeyBinding::new("cmd-shift-e", CompileProject, None),
                KeyBinding::new("ctrl-shift-e", CompileProject, None),
                #[cfg(target_os = "macos")]
                KeyBinding::new("ctrl-cmd-space", InsertSpecialChar, None),
                KeyBinding::new("ctrl-alt-space", InsertSpecialChar, None),
            ]);

            // 저장된 라이브러리를 열어본다(없거나 실패하면 온보딩).
            let library = config
                .library_root
                .as_deref()
                .and_then(|root| library::open(root).ok());
            let notice = if config.library_root.is_some() && library.is_none() {
                Some(SharedString::from(
                    "저장된 라이브러리를 열 수 없습니다. 폴더를 다시 선택하세요.",
                ))
            } else {
                None
            };

            let window_bounds = Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(1180.0), px(740.0)),
                cx,
            )));

            // Windows/Linux: 네이티브 타이틀바를 숨기고 커스텀 타이틀바를 그린다.
            #[cfg(not(target_os = "macos"))]
            let options = WindowOptions {
                window_bounds,
                window_min_size: Some(size(px(760.0), px(420.0))),
                ..TitleBar::window_options()
            };

            // macOS: 투명 타이틀바. 신호등 버튼은 라이브러리 패널 위쪽 줄 안에 둔다.
            #[cfg(target_os = "macos")]
            let options = WindowOptions {
                window_bounds,
                window_min_size: Some(size(px(760.0), px(420.0))),
                titlebar: Some(TitlebarOptions {
                    title: Some(APP_NAME.into()),
                    appears_transparent: true,
                    traffic_light_position: Some(point(
                        px(PANEL_INSET + 12.0),
                        px(TOOLBAR_HEIGHT / 2.0 - 7.0),
                    )),
                }),
                ..Default::default()
            };

            gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| WriterApp::new(library, notice, window, cx))
            })
            .expect("창 열기에 실패했습니다");
        });
}
