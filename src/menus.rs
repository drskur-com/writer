//! 앱 메뉴(macOS 네이티브 / 그 외 인윈도우 메뉴바).

use gpui_kit::component::*;
use gpui_kit::gpui::*;

#[cfg(target_os = "macos")]
use crate::APP_NAME;
use crate::{
    CloseLibrary, CompileProject, InsertImage, ToggleLineGroup, NewSheet, OpenLibrary, OpenSettings, QuitApp, SaveSheet,
    SetTheme, ShowAbout, TogglePreview, ToggleSidebar, ToggleTheme, theme,
};

/// 메뉴 트리.
///
/// - macOS: `Settings…`는 앱 메뉴(`Writer`)에 둔다 (Apple HIG).
/// - 그 외: `Edit > Settings…`에 둔다.
/// - `Edit > Toggle Line Group`: 선택한 줄을 줄 묶음(`:::`)으로 감싸거나 푼다(ADR-0016).
/// - 테마 목록은 `ThemeRegistry`에서 읽어 `View > Theme`에 채운다.
pub(crate) fn build_menus(theme_names: &[SharedString], active_theme: &str) -> Vec<Menu> {
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

    // macOS에서는 아래 Quit 추가가 빠지므로 `mut`이 쓰이지 않는다.
    #[cfg_attr(target_os = "macos", allow(unused_mut))]
    let mut file_items = vec![
        MenuItem::action("New Sheet", NewSheet),
        MenuItem::action("Save", SaveSheet),
        MenuItem::separator(),
        MenuItem::action("Compile Project…", CompileProject),
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

    #[cfg_attr(target_os = "macos", allow(unused_mut))]
    let mut edit_items = vec![MenuItem::action("Toggle Line Group", ToggleLineGroup)];
    #[cfg(not(target_os = "macos"))]
    {
        edit_items.push(MenuItem::separator());
        edit_items.push(MenuItem::action("Settings…", OpenSettings));
    }
    menus.push(Menu::new("Edit").items(edit_items));

    menus.push(Menu::new("Insert").items([MenuItem::action("Image…", InsertImage)]));

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
pub(crate) fn install_menus(cx: &mut App) {
    let theme_names = theme::theme_names(cx);
    let active = theme::active_name(cx);

    cx.set_menus(build_menus(&theme_names, &active));
    let owned: Vec<OwnedMenu> = build_menus(&theme_names, &active)
        .into_iter()
        .map(|menu| menu.owned())
        .collect();
    GlobalState::global_mut(cx).set_app_menus(owned);
}
