//! 앱 상태(`WriterApp`)와 액션 핸들러. 화면별 렌더는 하위 모듈에 나눠 둔다.

mod editor;
mod library_ops;
mod library_panel;
mod onboarding;
mod sheet_list;
pub(crate) mod style;

use gpui_kit::component::notification::Notification;
use gpui_kit::component::*;
use gpui_kit::gpui::prelude::FluentBuilder;
use gpui_kit::gpui::*;
use std::path::PathBuf;

#[cfg(not(target_os = "macos"))]
use gpui_kit::component::menu::AppMenuBar;

use crate::menus::install_menus;
use crate::{
    APP_NAME, CloseLibrary, OpenLibrary, OpenSettings, QuitApp, SaveSheet, SetTheme, ShowAbout,
    TogglePreview, ToggleSidebar, ToggleTheme, config, icons, library, theme,
};
use style::*;

/// 사이드바에서 선택된 항목(섹션 또는 그룹).
#[derive(Clone, PartialEq, Eq, Debug)]
enum Selection {
    Section(library::SectionKind),
    Group(PathBuf),
}

pub(crate) struct WriterApp {
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
    /// 읽어 둔 시트 미리보기(경로 → 읽은 시점의 수정 시각, 미리보기).
    /// 보이는 목록의 시트만 읽는다.
    previews: std::collections::HashMap<PathBuf, (u128, library::Preview)>,
    /// 미리보기를 읽는 백그라운드 태스크. 새로 시작하면 이전 것은 버려진다.
    preview_task: Option<Task<()>>,
    /// 라이브러리 폴더 감시(watcher + 변경 시 다시 스캔하는 태스크). 버리면 멈춘다.
    watcher: Option<(notify::RecommendedWatcher, Task<()>)>,
    #[cfg(not(target_os = "macos"))]
    menu_bar: Entity<AppMenuBar>,
}

impl WriterApp {
    /// 앱 상태를 만들고, 라이브러리가 있으면 감시와 미리보기 읽기를 시작한다.
    pub(crate) fn new(
        library: Option<library::Library>,
        notice: Option<SharedString>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut app = WriterApp {
            sidebar_visible: true,
            preview_visible: false,
            library,
            notice,
            collapsed_sections: std::collections::HashSet::new(),
            selected: None,
            selected_sheet: None,
            previews: std::collections::HashMap::new(),
            preview_task: None,
            watcher: None,
            #[cfg(not(target_os = "macos"))]
            menu_bar: AppMenuBar::new(cx),
        };
        app.start_watching(cx);
        app.load_previews(cx);
        app
    }

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

    // --- 액션 핸들러 --------------------------------------------------------

    fn on_save_sheet(&mut self, _: &SaveSheet, window: &mut Window, cx: &mut Context<Self>) {
        self.notify("문서가 저장되었습니다.", window, cx);
    }

    fn on_open_library(&mut self, _: &OpenLibrary, window: &mut Window, cx: &mut Context<Self>) {
        self.pick_library(window, cx);
    }

    fn on_close_library(&mut self, _: &CloseLibrary, _: &mut Window, cx: &mut Context<Self>) {
        self.watcher = None;
        self.preview_task = None;
        self.previews.clear();
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
        // 선택한 테마를 기억해 다음 실행에 복원한다.
        let mut config = config::Config::load();
        config.theme = Some(theme::active_name(cx).to_string());
        if let Err(err) = config.save() {
            eprintln!("설정 저장 실패: {err}");
        }

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
                sheets.sort_by_key(|sheet| std::cmp::Reverse(sheet.modified));
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
