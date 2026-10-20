use gpui_kit::assets::IconName as AssetIcon;
use gpui_kit::component::button::*;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::*;
use gpui_kit::gpui::prelude::{
    FluentBuilder, InteractiveElement as _, StatefulInteractiveElement as _,
};
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

    // macOS에서는 아래 Quit 추가가 빠지므로 `mut`이 쓰이지 않는다.
    #[cfg_attr(target_os = "macos", allow(unused_mut))]
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

// --- 레이아웃 치수 -----------------------------------------------------------

/// 컬럼 위쪽 툴바 높이. macOS 신호등 버튼도 이 줄의 세로 중앙에 맞춘다.
const TOOLBAR_HEIGHT: f32 = 52.0;
/// 라이브러리 패널(사이드바 1단)과 창 가장자리 사이 여백.
const PANEL_INSET: f32 = 8.0;
/// 라이브러리 패널 폭.
const PANEL_WIDTH: f32 = 220.0;
/// 패널과 시트 목록 사이 여백(그림자가 잘리지 않을 만큼).
const PANEL_GAP: f32 = 4.0;
/// 패널이 차지하는 전체 슬롯 폭(접기 애니메이션 대상).
const PANEL_SLOT_WIDTH: f32 = PANEL_INSET + PANEL_WIDTH + PANEL_GAP;
/// 사이드바 2단(시트 목록) 폭.
const SHEET_COLUMN_WIDTH: f32 = 280.0;
/// macOS: 패널을 접었을 때 시트 목록 툴바 왼쪽에 비워 둘 폭.
/// 신호등은 x=20에서 시작해 약 x=88에서 끝나므로, 그 뒤로 8px 여유를 둔다.
#[cfg(target_os = "macos")]
const TRAFFIC_LIGHT_SPACE: f32 = 96.0;

/// 사이드바 1단 접기/펴기 스프링. 임계 감쇠(ζ = 1)라 튀지 않고 약 0.3초에 멈춘다.
const SIDEBAR_SPRING: SpringConfig = SpringConfig::new(400.0, 40.0, 1.0);

/// 렌더에 쓰는 테마 토큰 묶음. 렌더 초입에서 값으로 복사해 `cx` 대여와 분리한다.
/// 색은 모두 테마 JSON에서 온다(ADR-0001).
#[derive(Clone, Copy)]
struct Palette {
    background: Hsla,
    foreground: Hsla,
    muted: Hsla,
    border: Hsla,
    danger: Hsla,
    /// 라이브러리 패널 배경/테두리.
    panel: Hsla,
    panel_border: Hsla,
    /// 패널의 선택된 줄.
    accent: Hsla,
    accent_foreground: Hsla,
    /// 목록 hover / 선택된 시트.
    hover: Hsla,
    active: Hsla,
    /// "새로운 프로젝트…" 같은 옅은 강조 버튼.
    tint: Hsla,
    tint_foreground: Hsla,
}

impl Palette {
    fn from_theme(theme: &Theme) -> Self {
        Self {
            background: theme.background,
            foreground: theme.foreground,
            muted: theme.muted_foreground,
            border: theme.border,
            danger: theme.danger,
            panel: theme.sidebar,
            panel_border: theme.sidebar_border,
            accent: theme.sidebar_accent,
            accent_foreground: theme.sidebar_accent_foreground,
            hover: theme.list_hover,
            active: theme.list_active,
            tint: theme.secondary,
            tint_foreground: theme.primary,
        }
    }
}

/// 아이콘만 있는 둥근 툴바 버튼. `enabled`가 false면 흐리게 그리고 hover/커서를 주지 않는다.
fn icon_button(
    id: impl Into<ElementId>,
    icon: AssetIcon,
    enabled: bool,
    pal: Palette,
) -> Stateful<Div> {
    let color = if enabled {
        pal.foreground.opacity(0.8)
    } else {
        pal.muted.opacity(0.6)
    };
    div()
        .id(id)
        .size(px(30.0))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        // 툴바의 창 이동(drag)으로 번지지 않게 막는다.
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .when(enabled, |this| {
            this.cursor_pointer()
                .hover(move |style| style.bg(pal.hover))
        })
        .child(Icon::new(icon).small().text_color(color))
}

/// 떠 있는 둥근 툴바 묶음.
fn tool_group(pal: Palette) -> Div {
    div()
        .flex()
        .items_center()
        .p(px(3.0))
        .gap(px(2.0))
        .rounded_full()
        .bg(pal.background)
        .border_1()
        .border_color(pal.border.opacity(0.6))
        .shadow_sm()
}

/// macOS: 투명 타이틀바 대신 툴바를 끌어 창을 옮긴다. 더블클릭은 시스템 동작(확대/최소화).
/// 그 외 플랫폼은 커스텀 타이틀바가 따로 있으므로 그대로 둔다.
fn window_drag_area(element: Div) -> Div {
    #[cfg(target_os = "macos")]
    let element = element.on_mouse_down(MouseButton::Left, |event, window, _| {
        if event.click_count >= 2 {
            window.titlebar_double_click();
        } else {
            window.start_window_move();
        }
    });
    element
}

/// 사이드바에서 선택된 항목(섹션 또는 그룹).
#[derive(Clone, PartialEq, Eq, Debug)]
enum Selection {
    Section(library::SectionKind),
    Group(PathBuf),
}

struct WriterApp {
    /// 사이드바 1단(섹션+그룹) 표시 여부. 2단(시트 목록)은 항상 보인다.
    sidebar_visible: bool,
    preview_visible: bool,
    library: Option<library::Library>,
    /// 접힌 최상위 섹션(UI 상태; 추후 registry에 저장).
    collapsed_sections: std::collections::HashSet<library::SectionKind>,
    /// 사이드바에서 선택한 항목(섹션/그룹). None이면 기본 선택을 쓴다.
    selected: Option<Selection>,
    /// 선택한 시트.
    selected_sheet: Option<PathBuf>,
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
        window.push_notification(Notification::info(message.into()), cx);
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
        match library::open(&root) {
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
                self.selected = None;
                self.selected_sheet = None;
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
        self.selected = None;
        self.selected_sheet = None;
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

    /// 최상위 섹션을 접거나 편다.
    fn toggle_section(&mut self, kind: library::SectionKind, cx: &mut Context<Self>) {
        // remove()가 true면 펼치기(이미 접힘), false면 접기.
        if !self.collapsed_sections.remove(&kind) {
            self.collapsed_sections.insert(kind);
        }
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

    /// 선택 상태에 해당하는 시트 목록.
    fn sheets_for(&self, selection: &Selection) -> Vec<library::Sheet> {
        let Some(lib) = &self.library else {
            return Vec::new();
        };
        match selection {
            // 섹션 선택("모두"): 섹션 안의 모든 시트를 최근 수정순으로.
            Selection::Section(kind) => {
                let mut sheets: Vec<library::Sheet> = lib
                    .sections
                    .iter()
                    .filter(|section| section.kind == *kind)
                    .flat_map(|section| &section.entries)
                    .flat_map(|entry| match entry {
                        library::Entry::Sheet(sheet) => std::slice::from_ref(sheet),
                        library::Entry::Group(group) => group.sheets.as_slice(),
                    })
                    .cloned()
                    .collect();
                sheets.sort_by(|a, b| b.modified.cmp(&a.modified));
                sheets
            }
            Selection::Group(path) => lib
                .sections
                .iter()
                .flat_map(|section| &section.entries)
                .find_map(|entry| match entry {
                    library::Entry::Group(group) if &group.path == path => {
                        Some(group.sheets.clone())
                    }
                    _ => None,
                })
                .unwrap_or_default(),
        }
    }

    /// 기본 선택: 첫 그룹, 없으면 첫 섹션.
    fn effective_selection(&self) -> Option<Selection> {
        if let Some(selection) = &self.selected {
            return Some(selection.clone());
        }
        let lib = self.library.as_ref()?;
        for section in &lib.sections {
            for entry in &section.entries {
                if let library::Entry::Group(group) = entry {
                    return Some(Selection::Group(group.path.clone()));
                }
            }
        }
        lib.sections
            .first()
            .map(|section| Selection::Section(section.kind))
    }

    /// 선택 항목의 표시 이름. 그룹은 메타의 표시 이름(`title`)을 쓴다.
    fn selection_label(&self, selection: &Selection) -> String {
        match selection {
            Selection::Section(library::SectionKind::Notes) => "모두".to_string(),
            Selection::Section(kind) => kind.title().to_string(),
            Selection::Group(path) => self
                .library
                .iter()
                .flat_map(|lib| &lib.sections)
                .flat_map(|section| &section.entries)
                .find_map(|entry| match entry {
                    library::Entry::Group(group) if &group.path == path => Some(group.name.clone()),
                    _ => None,
                })
                .unwrap_or_default(),
        }
    }

    /// 경로로 시트를 찾는다.
    fn find_sheet(&self, path: &PathBuf) -> Option<library::Sheet> {
        self.library
            .iter()
            .flat_map(|lib| &lib.sections)
            .flat_map(|section| &section.entries)
            .flat_map(|entry| match entry {
                library::Entry::Sheet(sheet) => std::slice::from_ref(sheet),
                library::Entry::Group(group) => group.sheets.as_slice(),
            })
            .find(|sheet| &sheet.path == path)
            .cloned()
    }

    /// 사이드바 1단: 창 안쪽에 떠 있는 라이브러리 패널(섹션 + 그룹).
    ///
    /// 패널은 항상 그리고, 바깥 슬롯 너비만 스프링으로 0 ↔ `PANEL_SLOT_WIDTH` 사이에서 움직인다.
    /// 안쪽은 고정 폭이라 접히는 동안 내용이 다시 배치되지 않고 잘려 나간다.
    fn render_library_panel(&self, cx: &mut Context<Self>, pal: Palette) -> impl IntoElement {
        let current = self.effective_selection();

        let mut rows: Vec<AnyElement> = Vec::new();
        if let Some(lib) = &self.library {
            for section in &lib.sections {
                let kind = section.kind;
                let collapsed = self.collapsed_sections.contains(&kind);
                let header_group = SharedString::from(format!("section-{}", kind.dir()));

                rows.push(
                    div()
                        .group(header_group.clone())
                        .mt_3()
                        .mb_1()
                        .px_4()
                        .flex()
                        .items_center()
                        .justify_between()
                        .text_xs()
                        .font_semibold()
                        .text_color(pal.muted)
                        .child(kind.title())
                        .child(
                            div()
                                .id(SharedString::from(format!("section-toggle-{}", kind.dir())))
                                .cursor_pointer()
                                .font_normal()
                                // 펼친 섹션은 헤더에 마우스를 올렸을 때만 "가리기"를 보인다.
                                .when(!collapsed, |this| {
                                    this.invisible()
                                        .group_hover(header_group, |style| style.visible())
                                })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.toggle_section(kind, cx);
                                }))
                                .child(if collapsed { "보기" } else { "가리기" }),
                        )
                        .into_any_element(),
                );

                if collapsed {
                    continue;
                }

                // 메모: 그룹을 가로지르는 "모두" 보기.
                if kind == library::SectionKind::Notes {
                    let selection = Selection::Section(kind);
                    let selected = current.as_ref() == Some(&selection);
                    rows.push(
                        self.library_row(
                            cx,
                            pal,
                            "all-notes".into(),
                            icons::all_sheets(),
                            "모두".into(),
                            selected,
                            selection,
                        )
                        .into_any_element(),
                    );
                }

                let mut has_group = false;
                for entry in &section.entries {
                    let library::Entry::Group(group) = entry else {
                        continue;
                    };
                    has_group = true;
                    let selection = Selection::Group(group.path.clone());
                    let selected = current.as_ref() == Some(&selection);
                    rows.push(
                        self.library_row(
                            cx,
                            pal,
                            format!("group-{}", group.path.display()).into(),
                            icons::group_icon(group.icon.as_deref()),
                            group.name.clone().into(),
                            selected,
                            selection,
                        )
                        .into_any_element(),
                    );
                }

                if kind == library::SectionKind::Projects && !has_group {
                    rows.push(
                        div()
                            .id("new-project")
                            .mx_3()
                            .mt_1()
                            .h(px(26.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_md()
                            .bg(pal.tint)
                            .text_xs()
                            .text_color(pal.tint_foreground)
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.notify("새 프로젝트 (준비 중)", window, cx);
                            }))
                            .child("새로운 프로젝트…")
                            .into_any_element(),
                    );
                }
            }
        }

        let panel = div()
            .w(px(PANEL_WIDTH))
            .h_full()
            .flex()
            .flex_col()
            .rounded_xl()
            .bg(pal.panel)
            .border_1()
            .border_color(pal.panel_border)
            .shadow_md()
            .child(
                // 위쪽 줄: (macOS) 신호등 버튼 자리 + 오른쪽 도구 버튼.
                window_drag_area(
                    div()
                        .h(px(TOOLBAR_HEIGHT - PANEL_INSET * 2.0))
                        .flex_shrink_0()
                        .px_2()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap_1(),
                )
                .child(icon_button("new-group", icons::new_group(), false, pal))
                .child(
                    icon_button("hide-sidebar", icons::panel(), true, pal).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.sidebar_visible = false;
                            cx.notify();
                        },
                    )),
                ),
            )
            .child(
                div()
                    .id("library-rows")
                    .flex_1()
                    .overflow_y_scroll()
                    .pb_2()
                    .children(rows),
            );

        div()
            .h_full()
            .flex_shrink_0()
            .overflow_hidden()
            .child(
                div()
                    .w(px(PANEL_SLOT_WIDTH))
                    .h_full()
                    .pl(px(PANEL_INSET))
                    .pr(px(PANEL_GAP))
                    .py(px(PANEL_INSET))
                    .child(panel),
            )
            .with_spring(
                "library-panel",
                SpringAnimation::new(SIDEBAR_SPRING).to(if self.sidebar_visible {
                    1.0_f32
                } else {
                    0.0
                }),
                |this, open| this.w(px(PANEL_SLOT_WIDTH * open.clamp(0.0, 1.0))),
            )
    }

    /// 라이브러리 패널의 한 줄(“모두”, 그룹).
    #[allow(clippy::too_many_arguments)]
    fn library_row(
        &self,
        cx: &mut Context<Self>,
        pal: Palette,
        id: SharedString,
        icon: AssetIcon,
        label: SharedString,
        selected: bool,
        selection: Selection,
    ) -> Stateful<Div> {
        let (text_color, icon_color) = if selected {
            (pal.accent_foreground, pal.accent_foreground)
        } else {
            (pal.foreground, pal.foreground.opacity(0.7))
        };

        div()
            .id(id)
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
                this.selected = Some(selection.clone());
                this.selected_sheet = None;
                cx.notify();
            }))
            .child(Icon::new(icon).small().text_color(icon_color))
            .child(label)
    }

    /// 사이드바 2단: 선택한 그룹(또는 "모두")의 시트 목록. 항상 보인다.
    fn render_sheet_column(&self, cx: &mut Context<Self>, pal: Palette) -> impl IntoElement {
        let current = self.effective_selection();
        let title = current
            .as_ref()
            .map(|selection| self.selection_label(selection))
            .unwrap_or_else(|| "시트".to_string());
        let sheets = current
            .as_ref()
            .map(|selection| self.sheets_for(selection))
            .unwrap_or_default();

        let toolbar = window_drag_area(
            div()
                .h(px(TOOLBAR_HEIGHT))
                .flex_shrink_0()
                .px_3()
                .flex()
                .items_center()
                .gap_2(),
        )
        // 패널이 접히면 (macOS) 신호등 자리를 비우고 다시 열기 버튼을 둔다.
        .when(!self.sidebar_visible, |this| {
            #[cfg(target_os = "macos")]
            let this = this.pl(px(TRAFFIC_LIGHT_SPACE));
            this.child(tool_group(pal).child(
                icon_button("show-sidebar", icons::panel(), true, pal).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.sidebar_visible = true;
                        cx.notify();
                    },
                )),
            ))
        })
        .child(div().flex_1())
        .child(tool_group(pal).child(icon_button("sheet-filter", icons::filter(), false, pal)))
        .child(tool_group(pal).child(
            icon_button("new-sheet", icons::compose(), true, pal).on_click(
                cx.listener(|this, _, window, cx| this.on_new_sheet(&NewSheet, window, cx)),
            ),
        ));

        let list: AnyElement = if sheets.is_empty() {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(pal.muted)
                .child("시트 없음")
                .into_any_element()
        } else {
            let mut items: Vec<AnyElement> = Vec::with_capacity(sheets.len());
            for sheet in sheets {
                let path = sheet.path.clone();
                let selected = self.selected_sheet.as_ref() == Some(&path);
                items.push(
                    div()
                        .id(SharedString::from(format!("sheet-{}", path.display())))
                        .px_4()
                        .py_3()
                        .border_b_1()
                        .border_color(pal.border.opacity(0.6))
                        .text_sm()
                        .text_color(pal.foreground)
                        .when(selected, |this| this.bg(pal.active))
                        .when(!selected, |this| {
                            this.hover(move |style| style.bg(pal.hover))
                        })
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.selected_sheet = Some(path.clone());
                            cx.notify();
                        }))
                        .child(div().font_semibold().child(sheet.title.clone()))
                        .into_any_element(),
                );
            }
            div()
                .id("sheet-list")
                .flex_1()
                .overflow_y_scroll()
                .children(items)
                .into_any_element()
        };

        div()
            .w(px(SHEET_COLUMN_WIDTH))
            .h_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .bg(pal.background)
            .border_r_1()
            .border_color(pal.border)
            .child(toolbar)
            .child(
                div()
                    .px_4()
                    .pb_2()
                    .border_b_1()
                    .border_color(pal.border)
                    .text_lg()
                    .font_semibold()
                    .text_color(pal.foreground)
                    .child(title),
            )
            .child(list)
    }

    /// 에디터 영역: 떠 있는 툴바 + 본문(빈 상태 또는 선택한 시트).
    fn render_editor_column(&self, cx: &mut Context<Self>, pal: Palette) -> impl IntoElement {
        let toolbar = window_drag_area(
            div()
                .h(px(TOOLBAR_HEIGHT))
                .flex_shrink_0()
                .px_3()
                .flex()
                .items_center()
                .gap_2(),
        )
        .child(
            tool_group(pal)
                .child(icon_button("nav-back", icons::back(), false, pal))
                .child(icon_button("nav-forward", icons::forward(), false, pal)),
        )
        .child(tool_group(pal).child(icon_button("search", icons::search(), false, pal)))
        .child(div().flex_1())
        .child(
            tool_group(pal)
                .child(icon_button("format-bold", icons::bold(), false, pal))
                .child(icon_button("format-italic", icons::italic(), false, pal))
                .child(icon_button("format-link", icons::link(), false, pal))
                .child(icon_button(
                    "format-paragraph",
                    icons::paragraph(),
                    false,
                    pal,
                )),
        )
        .child(
            tool_group(pal)
                .child(icon_button("share", icons::share(), false, pal))
                .child(icon_button("export", icons::export(), false, pal)),
        )
        .child(tool_group(pal).child(
            icon_button("toggle-preview", icons::preview(), true, pal).on_click(cx.listener(
                |this, _, window, cx| this.on_toggle_preview(&TogglePreview, window, cx),
            )),
        ));

        let body: AnyElement = match self
            .selected_sheet
            .as_ref()
            .and_then(|path| self.find_sheet(path))
        {
            Some(sheet) => div()
                .flex_1()
                .px(px(48.0))
                .py_6()
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    div()
                        .text_2xl()
                        .font_bold()
                        .text_color(pal.foreground)
                        .child(sheet.title),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(pal.muted)
                        .child("에디터 연결은 M2에서 진행합니다."),
                )
                .into_any_element(),
            None => div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_2xl()
                .text_color(pal.muted.opacity(0.7))
                .child("선택한 시트 없음")
                .into_any_element(),
        };

        div()
            .flex_1()
            .h_full()
            .flex()
            .flex_col()
            .bg(pal.background)
            .child(toolbar)
            .child(body)
    }

    fn render_workspace(&self, cx: &mut Context<Self>, pal: Palette) -> impl IntoElement {
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .size_full()
            .child(self.render_library_panel(cx, pal))
            .child(self.render_sheet_column(cx, pal))
            .child(self.render_editor_column(cx, pal))
            .when(self.preview_visible, |this| {
                this.child(
                    div()
                        .w(px(320.0))
                        .h_full()
                        .p_4()
                        .border_l_1()
                        .border_color(pal.border)
                        .bg(pal.panel)
                        .child(
                            div()
                                .text_xs()
                                .font_semibold()
                                .text_color(pal.muted)
                                .mb_2()
                                .child("PREVIEW"),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(pal.muted)
                                .child("프리뷰 영역 (M4)"),
                        ),
                )
            })
    }

    /// 라이브러리가 없을 때의 온보딩 화면.
    fn render_onboarding(&self, cx: &mut Context<Self>, pal: Palette) -> impl IntoElement {
        let suggested = config::default_library_root()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "(위치를 찾을 수 없음)".to_string());

        div()
            .flex_1()
            .size_full()
            .flex()
            .flex_col()
            // macOS: 투명 타이틀바 자리에서 창을 끌 수 있게 한다.
            .child(window_drag_area(
                div().w_full().h(px(TOOLBAR_HEIGHT)).flex_shrink_0(),
            ))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_4()
                    .child(div().text_2xl().font_bold().child(APP_NAME))
                    .child(
                        div()
                            .text_sm()
                            .text_color(pal.muted)
                            .child("모든 메모와 프로젝트를 담을 폴더를 선택하세요."),
                    )
                    .child(
                        div()
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .border_1()
                            .border_color(pal.border)
                            .bg(pal.panel)
                            .text_xs()
                            .text_color(pal.muted)
                            .child(format!("제안 위치: {suggested}")),
                    )
                    .when_some(self.notice.clone(), |this, notice| {
                        this.child(div().text_sm().text_color(pal.danger).child(notice))
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
                    ),
            )
    }
}

impl Render for WriterApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 테마 토큰을 값(Copy)으로 읽어 둔다. 이후 `cx.listener`와 대여 충돌이 없다.
        let pal = Palette::from_theme(cx.theme());

        let root = div()
            .flex()
            .flex_col()
            .size_full()
            .bg(pal.background)
            .text_color(pal.foreground)
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

        // 커스텀 타이틀바 + 메뉴바 (macOS는 투명 타이틀바 + 네이티브 메뉴를 쓰므로 생략)
        #[cfg(not(target_os = "macos"))]
        let root = {
            let theme = cx.theme();
            let (title_bar_background, title_bar_border) =
                (theme.title_bar, theme.title_bar_border);
            root.child(
                TitleBar::new()
                    .bg(title_bar_background)
                    .border_color(title_bar_border)
                    .child(self.menu_bar.clone()),
            )
        };

        // 바디: 라이브러리가 있으면 워크스페이스, 없으면 온보딩
        let body: AnyElement = if self.library.is_some() {
            self.render_workspace(cx, pal).into_any_element()
        } else {
            self.render_onboarding(cx, pal).into_any_element()
        };

        root.child(body)
    }
}

fn main() {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(|cx| {
            gpui_kit::init(cx);

            // 테마 등록 후 기본 라이트 테마 적용
            theme::register(cx);
            theme::set_theme(theme::DEFAULT_LIGHT, cx);

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

            gpui_kit::open_window(options, cx, |_, cx| {
                cx.new(|_cx| WriterApp {
                    sidebar_visible: true,
                    preview_visible: false,
                    library,
                    notice,
                    collapsed_sections: std::collections::HashSet::new(),
                    selected: None,
                    selected_sheet: None,
                    #[cfg(not(target_os = "macos"))]
                    menu_bar: AppMenuBar::new(_cx),
                })
            })
            .expect("창 열기에 실패했습니다");
        });
}
