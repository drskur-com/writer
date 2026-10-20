//! 앱 상태(`WriterApp`)와 액션 핸들러. 화면별 렌더는 하위 모듈에 나눠 둔다.

mod book_info;
mod editor;
mod file_view;
mod library_ops;
mod library_panel;
mod new_project;
mod onboarding;
mod project_panel;
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
    APP_NAME, CloseLibrary, OpenLibrary, QuitApp, SaveSheet, SetTheme, ShowAbout, TogglePreview,
    ToggleSidebar, ToggleTheme, config, icons, library, theme,
};
use style::*;

/// 그룹의 시트 정렬(방법 + 방향)과, 사용자가 고를 수 있는지.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct GroupSort {
    mode: library::SortMode,
    descending: bool,
    choosable: bool,
}

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
    /// 사이드바 1단에 펼쳐 둔 프로젝트. 있으면 라이브러리 대신 프로젝트 화면을 그린다.
    open_project: Option<PathBuf>,
    /// 프로젝트 화면에서 하위 그룹을 펼친 그룹.
    expanded_groups: std::collections::HashSet<PathBuf>,
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
    /// 에디터에 열린 시트.
    editor: Option<editor::OpenSheet>,
    /// 에디터 자리 뷰어에 연 파일(파일 그룹). 시트와 동시에 열리지 않는다.
    open_file: Option<file_view::OpenFile>,
    /// 파일 그룹 목록에서 보여 줄 파일 종류. None이면 모두.
    file_filter: Option<library::FileKind>,
    /// 앱 종료 직전에 열린 시트를 저장하는 구독.
    _quit_subscription: Subscription,
    _prefs_subscription: Subscription,
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
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // 창을 닫거나 앱을 끝낼 때 저장하지 않은 본문을 내린다.
        let this = cx.entity().downgrade();
        window.on_window_should_close(cx, move |_, cx| {
            this.update(cx, |this, cx| this.close_sheet(cx)).ok();
            true
        });
        // 설정(서체 등)이 바뀌면 다시 그린다.
        let prefs_subscription = cx.observe_global::<crate::settings::Prefs>(|_, cx| cx.notify());
        let quit_subscription = cx.on_app_quit(|this, cx| {
            this.close_sheet(cx);
            async {}
        });
        let mut app = WriterApp {
            sidebar_visible: true,
            preview_visible: false,
            library,
            notice,
            collapsed_sections: std::collections::HashSet::new(),
            open_project: None,
            expanded_groups: std::collections::HashSet::new(),
            selected: None,
            selected_sheet: None,
            previews: std::collections::HashMap::new(),
            preview_task: None,
            editor: None,
            open_file: None,
            file_filter: None,
            _quit_subscription: quit_subscription,
            _prefs_subscription: prefs_subscription,
            watcher: None,
            #[cfg(not(target_os = "macos"))]
            menu_bar: AppMenuBar::new(cx),
        };
        app.start_watching(window, cx);
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
        if let Err(err) = self.save_open_sheet(cx) {
            self.notify(format!("시트를 저장할 수 없습니다: {err}"), window, cx);
        }
    }

    fn on_open_library(&mut self, _: &OpenLibrary, window: &mut Window, cx: &mut Context<Self>) {
        self.pick_library(window, cx);
    }

    fn on_close_library(&mut self, _: &CloseLibrary, _: &mut Window, cx: &mut Context<Self>) {
        self.close_sheet(cx);
        self.watcher = None;
        self.preview_task = None;
        self.previews.clear();
        self.library = None;
        self.notice = None;
        self.open_project = None;
        self.expanded_groups.clear();
        self.selected = None;
        self.selected_sheet = None;
        self.open_file = None;
        cx.notify();
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

    fn on_set_theme(&mut self, action: &SetTheme, _: &mut Window, cx: &mut Context<Self>) {
        theme::set_theme(&action.name, cx);
        self.after_theme_change(cx);
    }

    fn on_toggle_theme(&mut self, _: &ToggleTheme, _: &mut Window, cx: &mut Context<Self>) {
        let next = if Theme::global(cx).is_dark() {
            theme::DEFAULT_LIGHT
        } else {
            theme::DEFAULT_DARK
        };
        theme::set_theme(next, cx);
        self.after_theme_change(cx);
    }

    fn after_theme_change(&mut self, cx: &mut Context<Self>) {
        // 선택한 테마를 기억해 다음 실행에 복원한다.
        let mut config = config::Config::load();
        config.theme = Some(theme::active_name(cx).to_string());
        if let Err(err) = config.save() {
            eprintln!("설정 저장 실패: {err}");
        }

        install_menus(cx);
        self.refresh_markup(cx);
        #[cfg(not(target_os = "macos"))]
        self.menu_bar.update(cx, |menu_bar, cx| menu_bar.reload(cx));
        cx.notify();
    }

    fn on_quit(&mut self, _: &QuitApp, _: &mut Window, cx: &mut Context<Self>) {
        self.close_sheet(cx);
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
                    .section(*kind)
                    .map(|section| section.sheets().into_iter().cloned().collect())
                    .unwrap_or_default();
                sheets.sort_by_key(|sheet| std::cmp::Reverse(sheet.modified));
                sheets
            }
            Selection::Group(path) => {
                let mut sheets = lib
                    .find_group(path)
                    .map(|group| group.sheets.clone())
                    .unwrap_or_default();
                self.sort_sheets(&mut sheets, self.sort_for(path));
                sheets
            }
        }
    }

    /// 그룹의 정렬 방법·방향과, 사용자가 고를 수 있는지.
    ///
    /// 프로젝트 원고는 순서가 곧 원고 구조라서 늘 수동이고 고를 수 없다.
    /// 지정이 없으면 메모는 수정일순, 프로젝트(추가자료)는 수동이다.
    fn sort_for(&self, path: &std::path::Path) -> GroupSort {
        let fixed = GroupSort {
            mode: library::SortMode::Manual,
            descending: false,
            choosable: false,
        };
        let Some(lib) = &self.library else {
            return fixed;
        };
        let projects = lib.root.join(library::SectionKind::Projects.dir());
        let default = match path.strip_prefix(&projects) {
            // <Project>/Extras/… 만 고를 수 있다.
            Ok(relative)
                if relative
                    .components()
                    .nth(1)
                    .is_some_and(|part| part.as_os_str() == library::EXTRAS_DIR) =>
            {
                library::SortMode::Manual
            }
            Ok(_) => return fixed,
            Err(_) => library::SortMode::Modified,
        };
        let group = lib.find_group(path);
        let mode = group.and_then(|group| group.sort).unwrap_or(default);
        GroupSort {
            mode,
            descending: group
                .and_then(|group| group.descending)
                .unwrap_or(mode.default_descending()),
            choosable: true,
        }
    }

    /// 시트를 정렬 방법·방향대로 늘어놓는다. 수동이면 읽은 순서(메타 `sheets`) 그대로 둔다.
    ///
    /// 제목순은 읽어 둔 미리보기 제목을 쓴다. 아직 못 읽은 시트와 빈 시트는 방향과 상관없이 뒤로 간다.
    fn sort_sheets(&self, sheets: &mut [library::Sheet], sort: GroupSort) {
        let directed = |ordering: std::cmp::Ordering| {
            if sort.descending {
                ordering.reverse()
            } else {
                ordering
            }
        };
        match sort.mode {
            library::SortMode::Manual => {}
            library::SortMode::Modified => {
                sheets.sort_by(|a, b| directed(a.modified.cmp(&b.modified)))
            }
            library::SortMode::Created => {
                sheets.sort_by(|a, b| directed(a.created.cmp(&b.created)))
            }
            library::SortMode::Title => {
                let title = |sheet: &library::Sheet| {
                    self.previews
                        .get(&sheet.path)
                        .map(|(_, preview)| preview.title.to_lowercase())
                        .unwrap_or_default()
                };
                let mut keyed: Vec<(String, library::Sheet)> = sheets
                    .iter()
                    .map(|sheet| (title(sheet), sheet.clone()))
                    .collect();
                keyed.sort_by(|(a, _), (b, _)| {
                    a.is_empty()
                        .cmp(&b.is_empty())
                        .then_with(|| directed(a.cmp(b)))
                });
                for (slot, (_, sheet)) in sheets.iter_mut().zip(keyed) {
                    *slot = sheet;
                }
            }
        }
    }

    /// 파일 그룹의 파일을 정렬 방법·방향대로 늘어놓는다. 수동이면 읽은 순서(메타 `files`) 그대로,
    /// 제목순은 파일 이름순이다.
    fn sort_files(&self, files: &mut [library::FileItem], sort: GroupSort) {
        let directed = |ordering: std::cmp::Ordering| {
            if sort.descending {
                ordering.reverse()
            } else {
                ordering
            }
        };
        match sort.mode {
            library::SortMode::Manual => {}
            library::SortMode::Modified => {
                files.sort_by(|a, b| directed(a.modified.cmp(&b.modified)))
            }
            library::SortMode::Created => files.sort_by(|a, b| directed(a.created.cmp(&b.created))),
            library::SortMode::Title => {
                files.sort_by_cached_key(|file| file.name().to_lowercase());
                if sort.descending {
                    files.reverse();
                }
            }
        }
    }

    /// 기본 선택: 프로젝트 화면이면 그 프로젝트의 기본 선택, 아니면 첫 그룹, 없으면 첫 섹션.
    fn effective_selection(&self) -> Option<Selection> {
        if let Some(selection) = &self.selected {
            return Some(selection.clone());
        }
        let lib = self.library.as_ref()?;
        if let Some(project) = self
            .open_project
            .as_ref()
            .and_then(|dir| lib.find_project(dir))
        {
            return Some(project_panel::default_selection(project));
        }
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

    /// 지금 보고 있는 목록이 파일 그룹(첨부파일 등)인지.
    fn viewing_file_group(&self) -> bool {
        match self.effective_selection() {
            Some(Selection::Group(path)) => self
                .library
                .as_ref()
                .and_then(|lib| lib.find_group(&path))
                .is_some_and(|group| group.kind == library::GroupKind::Files),
            _ => false,
        }
    }

    /// 선택 항목의 표시 이름. 그룹은 메타의 표시 이름(`title`)을 쓴다.
    fn selection_label(&self, selection: &Selection) -> String {
        match selection {
            Selection::Section(library::SectionKind::Notes) => "모두".to_string(),
            Selection::Section(kind) => kind.title().to_string(),
            Selection::Group(path) => self
                .library
                .as_ref()
                .and_then(|lib| lib.find_group(path))
                .map(|group| group.name.clone())
                .unwrap_or_default(),
        }
    }

    /// 경로로 시트를 찾는다.
    fn find_sheet(&self, path: &PathBuf) -> Option<library::Sheet> {
        self.library
            .iter()
            .flat_map(|lib| &lib.sections)
            .flat_map(library::Section::sheets)
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
