//! 라이브러리 열기·감시·다시 스캔·미리보기·시트 만들기/옮기기 (상태 변경 로직).

use gpui_kit::component::notification::Notification;
use gpui_kit::component::*;
use gpui_kit::gpui::*;
use std::path::PathBuf;

use super::{Selection, WriterApp};
use crate::{APP_NAME, NewSheet, config, library, watch};

/// 정렬 메뉴에서 고른 것.
#[derive(Clone, Copy)]
pub(super) enum SortChange {
    Mode(library::SortMode),
    Descending(bool),
}

impl WriterApp {
    // --- 라이브러리 폴더 설정 ---------------------------------------------

    /// 기본 제안 경로에 새 라이브러리를 만든다.
    pub(super) fn create_default_library(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = config::default_library_root() else {
            self.set_notice("기본 라이브러리 위치를 찾을 수 없습니다.", cx);
            return;
        };
        self.initialize_at(root, window, cx);
    }

    /// 라이브러리 골격을 만든 뒤 연다.
    pub(super) fn initialize_at(
        &mut self,
        root: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match library::initialize(&root) {
            Ok(()) => self.adopt(root, window, cx),
            Err(err) => self.set_notice(format!("라이브러리를 만들 수 없습니다: {err}"), cx),
        }
    }

    /// 기존 폴더를 검사해 알맞게 연다.
    pub(super) fn open_folder(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
    pub(super) fn adopt(&mut self, root: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
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
                self.close_sheet(cx);
                self.library = Some(opened);
                self.notice = None;
                self.open_project = None;
                self.expanded_groups.clear();
                self.selected = None;
                self.previews.clear();
                self.start_watching(window, cx);
                self.load_previews(cx);
                cx.notify();
            }
            Err(err) => self.set_notice(format!("라이브러리를 읽을 수 없습니다: {err}"), cx),
        }
    }

    /// OS 네이티브 폴더 선택기로 라이브러리 폴더를 고른다.
    pub(super) fn pick_library(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    // --- 폴더 감시 ---------------------------------------------------------

    /// 현재 라이브러리 루트 감시를 (다시) 시작한다. 변경이 오면 잠깐 모았다가 다시 스캔한다.
    pub(super) fn start_watching(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.watcher = None;
        let Some(root) = self.library.as_ref().map(|lib| lib.root.clone()) else {
            return;
        };
        let (watcher, mut events) = match watch::watch(&root) {
            Ok(pair) => pair,
            Err(err) => {
                eprintln!("폴더 감시 실패: {err}");
                return;
            }
        };
        let task = cx.spawn_in(window, async move |this, cx| {
            use futures::StreamExt as _;
            while events.next().await.is_some() {
                // 저장·이동은 이벤트가 여러 개 연달아 온다. 잠깐 기다렸다가 한꺼번에 처리한다.
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(150))
                    .await;
                while events.try_recv().is_ok() {}

                // 디스크 I/O는 백그라운드에서 하고, 결과 반영만 UI 스레드에서 한다.
                let scan_root = root.clone();
                let scanned = cx
                    .background_executor()
                    .spawn(async move { library::scan(&scan_root) })
                    .await;
                if this
                    .update_in(cx, |this, window, cx| {
                        this.apply_scan(&root, scanned, window, cx)
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        self.watcher = Some((watcher, task));
    }

    /// 다시 읽은 라이브러리를 반영한다. 사라진 시트/그룹의 선택은 푼다.
    ///
    /// 스캔하는 사이에 다른 라이브러리로 바뀌었으면 결과를 버린다.
    pub(super) fn apply_scan(
        &mut self,
        root: &std::path::Path,
        scanned: std::io::Result<library::Library>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.library.as_ref().map(|lib| lib.root.as_path()) != Some(root) {
            return;
        }
        let scanned = match scanned {
            Ok(scanned) => scanned,
            Err(err) => {
                eprintln!("라이브러리 다시 읽기 실패: {err}");
                return;
            }
        };
        self.library = Some(scanned);
        if let Some(Selection::Group(path)) = &self.selected
            && !path.is_dir()
        {
            self.selected = None;
        }
        if self.open_project.as_ref().is_some_and(|dir| !dir.is_dir()) {
            self.open_project = None;
        }
        self.expanded_groups.retain(|dir| dir.is_dir());
        if let Some(path) = &self.selected_sheet
            && self.find_sheet(path).is_none()
        {
            self.selected_sheet = None;
        }
        if self
            .open_file
            .as_ref()
            .is_some_and(|file| !file.path.is_file())
        {
            self.open_file = None;
        }
        if self
            .book_style
            .as_ref()
            .is_some_and(|view| !view.read(cx).project().is_dir())
        {
            self.book_style = None;
        }
        self.sync_open_sheet(window, cx);
        self.load_previews(cx);
        cx.notify();
    }

    /// 그룹 안에서 시트를 옮기고 순서를 저장한다.
    pub(super) fn move_sheet(
        &mut self,
        group_path: &std::path::Path,
        from: usize,
        to: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(group) = self
            .library
            .as_mut()
            .and_then(|lib| lib.find_group_mut(group_path))
        else {
            return;
        };
        if let Err(err) = library::move_sheet(group, from, to) {
            self.notify(format!("순서를 저장할 수 없습니다: {err}"), window, cx);
            return;
        }
        if !self.selected_sheets.is_empty() {
            self.selected_sheets = group.sheets.iter()
                .filter(|sheet| self.selected_sheets.contains(&sheet.path))
                .map(|sheet| sheet.path.clone())
                .collect();
        }
        cx.notify();
    }

    /// 그룹의 `range` 시트를 접합하거나(`glue`) 앞뒤 시트에서 떼어 내고 그룹 메타에 저장한다.
    pub(super) fn set_glued(
        &mut self,
        group_path: &std::path::Path,
        range: std::ops::Range<usize>,
        glue: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(group) = self
            .library
            .as_mut()
            .and_then(|lib| lib.find_group_mut(group_path))
        else {
            return;
        };
        let result = if glue {
            library::glue_sheets(group, range)
        } else {
            library::unglue_sheets(group, range)
        };
        if let Err(err) = result {
            self.notify(format!("시트 접합을 저장할 수 없습니다: {err}"), window, cx);
            return;
        }
        cx.notify();
    }

    /// 파일 그룹에서 파일 순서를 바꾸고 그룹 메타에 저장한다.
    pub(super) fn move_file(
        &mut self,
        group_path: &std::path::Path,
        from: usize,
        to: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(group) = self
            .library
            .as_mut()
            .and_then(|lib| lib.find_group_mut(group_path))
        else {
            return;
        };
        if let Err(err) = library::move_file(group, from, to) {
            self.notify(format!("순서를 저장할 수 없습니다: {err}"), window, cx);
            return;
        }
        cx.notify();
    }

    /// 그룹의 정렬 방법이나 방향을 바꾸고 그룹 메타에 저장한다.
    pub(super) fn set_sort(
        &mut self,
        group_path: &std::path::Path,
        change: SortChange,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(group) = self
            .library
            .as_mut()
            .and_then(|lib| lib.find_group_mut(group_path))
        else {
            return;
        };
        let saved = match change {
            SortChange::Mode(mode) => library::set_sort(group, mode),
            SortChange::Descending(descending) => library::set_sort_descending(group, descending),
        };
        if let Err(err) = saved {
            self.notify(format!("정렬 방법을 저장할 수 없습니다: {err}"), window, cx);
            return;
        }
        cx.notify();
    }

    /// 지금 보이는 목록에서 아직 안 읽었거나 바뀐 시트의 미리보기를 백그라운드에서 읽는다.
    ///
    /// 제목순 그룹은 순서가 미리보기 제목에 달려 있으므로 바로 읽는다. 백그라운드로 읽으면
    /// 처음엔 읽은 순서로 그렸다가 제목이 들어오면서 다시 정렬되어 목록이 한 번 뒤바뀐다.
    pub(super) fn load_previews(&mut self, cx: &mut Context<Self>) {
        let selection = self.effective_selection();
        let by_title = matches!(&selection, Some(Selection::Group(path))
            if self.sort_for(path).mode == library::SortMode::Title);
        let pending: Vec<(PathBuf, u128)> = selection
            .map(|selection| self.sheets_for(&selection))
            .unwrap_or_default()
            .into_iter()
            .filter(|sheet| {
                self.previews
                    .get(&sheet.path)
                    .is_none_or(|(modified, _)| *modified != sheet.modified)
            })
            .map(|sheet| (sheet.path, sheet.modified))
            .collect();
        if pending.is_empty() {
            return;
        }
        if by_title {
            self.preview_task = None;
            self.previews.extend(
                pending.into_iter().map(|(path, modified)| {
                    (path.clone(), (modified, library::read_preview(&path)))
                }),
            );
            cx.notify();
            return;
        }
        self.preview_task = Some(cx.spawn(async move |this, cx| {
            let loaded = cx
                .background_executor()
                .spawn(async move {
                    pending
                        .into_iter()
                        .map(|(path, modified)| {
                            let preview = library::read_preview(&path);
                            (path, (modified, preview))
                        })
                        .collect::<Vec<_>>()
                })
                .await;
            this.update(cx, |this, cx| {
                this.previews.extend(loaded);
                cx.notify();
            })
            .ok();
        }));
    }

    /// 현재 선택한 그룹에 빈 시트를 만들고 선택한다.
    pub(super) fn on_new_sheet(
        &mut self,
        _: &NewSheet,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(lib) = &self.library else {
            self.notify("먼저 라이브러리를 여세요.", window, cx);
            return;
        };
        let root = lib.root.clone();
        let dir = match self.effective_selection() {
            Some(Selection::Group(path)) => path,
            // "모두"(메모)에서는 Inbox에 만든다.
            Some(Selection::Section(library::SectionKind::Notes)) | None => root
                .join(library::SectionKind::Notes.dir())
                .join(library::DEFAULT_INBOX),
            Some(Selection::Section(kind)) => root.join(kind.dir()),
        };
        if lib
            .find_group(&dir)
            .is_some_and(|group| group.kind == library::GroupKind::Files)
        {
            self.notify("파일 그룹에는 시트를 만들 수 없습니다.", window, cx);
            return;
        }

        let created = library::create_sheet(&dir).and_then(|path| {
            let scanned = library::scan(&root)?;
            Ok((path, scanned))
        });
        match created {
            Ok((path, scanned)) => {
                self.library = Some(scanned);
                // 새 시트는 바로 쓸 수 있게 에디터에 열고 포커스를 준다.
                self.select_sheet(Some(path), true, window, cx);
                self.load_previews(cx);
            }
            Err(err) => self.notify(format!("시트를 만들 수 없습니다: {err}"), window, cx),
        }
    }
}
