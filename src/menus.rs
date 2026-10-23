//! 앱 메뉴(macOS 네이티브 / 그 외 인윈도우 메뉴바).

use gpui_kit::component::*;
use gpui_kit::gpui::*;

#[cfg(target_os = "macos")]
use crate::APP_NAME;
use crate::{
    CloseLibrary, CompileProject, InsertExtension, InsertFootnote, InsertImage, InsertSpecialChar,
    InsertTable, ManageExtensions, NewSheet, OpenLibrary, OpenSettings, QuitApp,
    RunExtensionCommand, SaveSheet, SetTheme, ShowAbout, ToggleBold, ToggleItalic, ToggleLineGroup,
    TogglePreview, ToggleSidebar, ToggleTheme, theme,
};

/// 메뉴 트리.
///
/// - macOS: `Settings…`는 앱 메뉴(`Writer`)에 둔다 (Apple HIG).
/// - 그 외: `Edit > Settings…`에 둔다.
/// - `Edit > Toggle Line Group`: 선택한 줄을 줄 묶음(`:::`)으로 감싸거나 푼다(ADR-0016).
/// - 테마 목록은 `ThemeRegistry`에서 읽어 `View > Theme`에 채운다.
/// - `Extensions` 메뉴(ADR-0026): 확장 명령을 확장 이름으로 묶고, 맨 아래에 `Manage Extensions…`를 둔다.
///   명령이 없어도 메뉴는 보인다.
pub(crate) fn build_menus(
    theme_names: &[SharedString],
    active_theme: &str,
    extension_commands: &[(String, usize, String)],
) -> Vec<Menu> {
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
        MenuItem::action(t!("menu.about"), ShowAbout),
        MenuItem::separator(),
        MenuItem::action(t!("menu.settings"), OpenSettings),
        MenuItem::separator(),
        MenuItem::action(t!("menu.quit_app"), QuitApp),
    ]));

    // macOS에서는 아래 Quit 추가가 빠지므로 `mut`이 쓰이지 않는다.
    #[cfg_attr(target_os = "macos", allow(unused_mut))]
    let mut file_items = vec![
        MenuItem::action(t!("menu.new_sheet"), NewSheet),
        MenuItem::action(t!("menu.save"), SaveSheet),
        MenuItem::separator(),
        MenuItem::action(t!("menu.compile_project"), CompileProject),
        MenuItem::separator(),
        MenuItem::action(t!("menu.open_library"), OpenLibrary),
        MenuItem::action(t!("menu.close_library"), CloseLibrary),
    ];
    #[cfg(not(target_os = "macos"))]
    {
        file_items.push(MenuItem::separator());
        file_items.push(MenuItem::action(t!("menu.quit"), QuitApp));
    }
    menus.push(Menu::new(t!("menu.file")).items(file_items));

    // `Emoji & Symbols…`는 macOS 관례상 Edit에, 그 밖에는 Insert > Special Characters… 하나로 둔다(같은 대화상자를 두 번 보이지 않게).
    let mut edit_items = vec![
        MenuItem::action(t!("menu.bold"), ToggleBold),
        MenuItem::action(t!("menu.italic"), ToggleItalic),
        MenuItem::separator(),
        MenuItem::action(t!("menu.toggle_line_group"), ToggleLineGroup),
    ];
    #[cfg(target_os = "macos")]
    {
        edit_items.push(MenuItem::separator());
        edit_items.push(MenuItem::action(
            t!("menu.emoji_symbols"),
            InsertSpecialChar,
        ));
    }
    #[cfg(not(target_os = "macos"))]
    {
        edit_items.push(MenuItem::separator());
        edit_items.push(MenuItem::action(t!("menu.settings"), OpenSettings));
    }
    menus.push(Menu::new(t!("menu.edit")).items(edit_items));

    menus.push(Menu::new(t!("menu.insert")).items([
        MenuItem::action(t!("menu.insert_image"), InsertImage),
        MenuItem::action(t!("menu.insert_table"), InsertTable),
        MenuItem::action(t!("menu.insert_footnote"), InsertFootnote),
        MenuItem::action(t!("menu.insert_extension"), InsertExtension),
        MenuItem::separator(),
        MenuItem::action(t!("menu.special_characters"), InsertSpecialChar),
    ]));

    {
        let mut items = Vec::new();
        let mut previous: Option<&str> = None;
        for (extension, index, title) in extension_commands {
            if previous.is_some_and(|previous| previous != extension) {
                items.push(MenuItem::separator());
            }
            previous = Some(extension);
            items.push(MenuItem::action(
                title.clone(),
                RunExtensionCommand {
                    extension: extension.clone().into(),
                    index: *index,
                },
            ));
        }
        if !items.is_empty() {
            items.push(MenuItem::separator());
        }
        items.push(MenuItem::action(
            t!("menu.manage_extensions"),
            ManageExtensions,
        ));
        menus.push(Menu::new(t!("menu.extensions")).items(items));
    }

    let mut view_items = vec![
        MenuItem::action(t!("menu.toggle_sidebar"), ToggleSidebar),
        MenuItem::action(t!("menu.toggle_preview"), TogglePreview),
        MenuItem::action(t!("menu.toggle_theme"), ToggleTheme),
    ];
    if !theme_items.is_empty() {
        view_items.push(MenuItem::separator());
        view_items.push(MenuItem::submenu(
            Menu::new(t!("menu.theme")).items(theme_items),
        ));
    }
    menus.push(Menu::new(t!("menu.view")).items(view_items));

    menus.push(Menu::new(t!("menu.help")).items([MenuItem::action(t!("menu.about"), ShowAbout)]));

    menus
}

/// 열린 라이브러리의 확장 명령: (확장 이름, 명령 번호, 제목). 메뉴를 만들 때 읽는다.
#[derive(Default)]
pub(crate) struct ExtensionCommands(pub(crate) Vec<(String, usize, String)>);

impl Global for ExtensionCommands {}

/// 현재 테마와 확장 명령을 반영해 앱 메뉴를 (재)설치한다.
pub(crate) fn install_menus(cx: &mut App) {
    let theme_names = theme::theme_names(cx);
    let active = theme::active_name(cx);
    let commands = cx
        .try_global::<ExtensionCommands>()
        .map(|commands| commands.0.clone())
        .unwrap_or_default();

    cx.set_menus(build_menus(&theme_names, &active, &commands));
    let owned: Vec<OwnedMenu> = build_menus(&theme_names, &active, &commands)
        .into_iter()
        .map(|menu| menu.owned())
        .collect();
    GlobalState::global_mut(cx).set_app_menus(owned);
}
