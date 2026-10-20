//! 프로젝트 컴파일 대화상자: ePub 등의 형식으로 원고를 묶어 내보낸다.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::component::{ActiveTheme as _, Icon, StyledExt as _, WindowExt as _};
use gpui_kit::gpui::prelude::FluentBuilder as _;
use gpui_kit::gpui::*;
use std::path::{Path, PathBuf};

use super::WriterApp;
use super::style::*;
use crate::compile::compile_epub_to_file;
use crate::{icons, library};

/// 컴파일 대화상자 폭.
const DIALOG_WIDTH: f32 = 540.0;

/// 컴파일 출력 형식.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CompileFormat {
    Epub,
    Pdf,
    Docx,
}

impl CompileFormat {
    pub fn title(self) -> &'static str {
        match self {
            CompileFormat::Epub => "ePub 3.0",
            CompileFormat::Pdf => "PDF",
            CompileFormat::Docx => "DOCX",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            CompileFormat::Epub => "표준 전자책 형식 (리더기·스마트폰·태블릿)",
            CompileFormat::Pdf => "인쇄 및 고정 판형 문서 (준비 중)",
            CompileFormat::Docx => "Microsoft Word 문서 (준비 중)",
        }
    }

    pub fn is_supported(self) -> bool {
        matches!(self, CompileFormat::Epub)
    }

    pub fn extension(self) -> &'static str {
        match self {
            CompileFormat::Epub => "epub",
            CompileFormat::Pdf => "pdf",
            CompileFormat::Docx => "docx",
        }
    }
}

/// 컴파일 대화상자 뷰.
pub struct CompileDialog {
    app: WeakEntity<WriterApp>,
    project_dir: PathBuf,
    project_name: String,
    book: library::BookMeta,
    sheet_count: usize,
    format: CompileFormat,
}

impl CompileDialog {
    pub fn new(
        app: WeakEntity<WriterApp>,
        project_dir: PathBuf,
        project_name: String,
        book: library::BookMeta,
        sheet_count: usize,
    ) -> Self {
        Self {
            app,
            project_dir,
            project_name,
            book,
            sheet_count,
            format: CompileFormat::Epub,
        }
    }

    /// 파일 저장 대화상자를 열고 컴파일을 수행한다.
    fn start_compile(&self, window: &mut Window, cx: &mut Context<Self>) {
        let app = self.app.clone();
        let project_dir = self.project_dir.clone();
        let format = self.format;

        let title = if self.book.title.trim().is_empty() {
            &self.project_name
        } else {
            &self.book.title
        };
        let default_name = format!("{}.{}", title.trim(), format.extension());
        let default_dir = project_dir
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| project_dir.clone());

        window.close_dialog(cx);

        let receiver = cx.prompt_for_new_path(&default_dir, Some(&default_name));
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(output_path))) = receiver.await else {
                return;
            };

            let project_path = project_dir.clone();
            let target_path = output_path.clone();

            let result = cx
                .background_executor()
                .spawn(async move {
                    compile_epub_to_file(&project_path, &target_path)
                })
                .await;

            let file_display_name = output_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "책".to_string());

            this.update(cx, |_dialog, cx| {
                if let Some(app) = app.upgrade() {
                    app.update(cx, |writer, cx| {
                        match result {
                            Ok(()) => {
                                writer.set_notice(
                                    format!("ePub 컴파일 완료: {file_display_name}"),
                                    cx,
                                );
                            }
                            Err(err) => {
                                writer.set_notice(
                                    format!("ePub 컴파일 실패: {err}"),
                                    cx,
                                );
                            }
                        }
                    });
                }
            })
            .ok();
        })
        .detach();
    }
}

impl Render for CompileDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pal = Palette::from_theme(cx.theme());
        let title = if self.book.title.trim().is_empty() {
            &self.project_name
        } else {
            &self.book.title
        };

        let author_text = if self.book.authors.is_empty() {
            "지은이 미지정".to_string()
        } else {
            self.book.authors.join(", ")
        };

        let formats = [CompileFormat::Epub, CompileFormat::Pdf, CompileFormat::Docx];

        div()
            .flex()
            .flex_col()
            .gap_4()
            .text_sm()
            .child(
                // 프로젝트 요약 카드
                div()
                    .p_3()
                    .rounded_md()
                    .bg(pal.hover)
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .w(px(36.0))
                            .h(px(36.0))
                            .rounded_md()
                            .bg(pal.accent.opacity(0.15))
                            .text_color(pal.accent)
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(Icon::new(icons::book_info()).size(px(20.0))),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_0p5()
                            .child(
                                div()
                                    .font_semibold()
                                    .truncate()
                                    .child(title.to_string()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(pal.muted)
                                    .truncate()
                                    .child(format!("{author_text} · 원고 시트 {}개", self.sheet_count)),
                            ),
                    ),
            )
            .child(
                // 출력 형식 선택
                div()
                    .flex()
                    .flex_col()
                    .gap_1p5()
                    .child(div().font_medium().child("내보내기 형식"))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .children(formats.into_iter().map(|fmt| {
                                let is_selected = self.format == fmt;
                                let is_supported = fmt.is_supported();
                                div()
                                    .id(fmt.title())
                                    .px_3()
                                    .py_2()
                                    .rounded_md()
                                    .border_1()
                                    .map(|this| {
                                        if is_selected {
                                            this.border_color(pal.accent)
                                                .bg(pal.accent.opacity(0.08))
                                        } else if is_supported {
                                            this.border_color(pal.border)
                                                .hover(|s| s.bg(pal.hover))
                                                .cursor_pointer()
                                        } else {
                                            this.border_color(pal.border.opacity(0.5))
                                                .opacity(0.6)
                                        }
                                    })
                                    .when(is_supported, |this| {
                                        this.on_click(cx.listener(move |this, _, _, cx| {
                                            this.format = fmt;
                                            cx.notify();
                                        }))
                                    })
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .gap_0p5()
                                            .child(
                                                div()
                                                    .font_medium()
                                                    .flex()
                                                    .items_center()
                                                    .gap_2()
                                                    .child(fmt.title())
                                                    .when(!is_supported, |this| {
                                                        this.child(
                                                            div()
                                                                .text_xs()
                                                                .px_1p5()
                                                                .py_0p5()
                                                                .rounded(px(4.0))
                                                                .bg(pal.muted.opacity(0.2))
                                                                .text_color(pal.muted)
                                                                .child("준비 중"),
                                                        )
                                                    }),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(pal.muted)
                                                    .child(fmt.description()),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .w(px(18.0))
                                            .h(px(18.0))
                                            .rounded_full()
                                            .border_1()
                                            .border_color(if is_selected { pal.accent } else { pal.border })
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .when(is_selected, |this| {
                                                this.child(
                                                    div()
                                                        .w(px(10.0))
                                                        .h(px(10.0))
                                                        .rounded_full()
                                                        .bg(pal.accent),
                                                )
                                            }),
                                    )
                            })),
                    ),
            )
            .child(
                // ePub 상세 안내
                div()
                    .px_3()
                    .py_2p5()
                    .rounded_md()
                    .border_1()
                    .border_color(pal.border)
                    .bg(pal.panel)
                    .flex()
                    .flex_col()
                    .gap_1()
                    .text_xs()
                    .text_color(pal.muted)
                    .child(
                        div()
                            .font_medium()
                            .text_color(pal.foreground)
                            .child("포함 정보"),
                    )
                    .child(div().child("• 원고 시트 순서 및 접합(연결) 여부 반영"))
                    .child(div().child("• 프로젝트 책 스타일(글꼴 크기, 여백, 문단 들여쓰기) 적용"))
                    .child(div().child(if self.book.cover.is_some() {
                        "• 표지 이미지 포함 (Extras/Attachments/cover)"
                    } else {
                        "• 표지 이미지 없음 (기본 표지 생략)"
                    }))
                    .child(div().child("• 제목(H1~H6) 기반 TOC 목차(toc.ncx) 자동 생성")),
            )
    }
}

impl WriterApp {
    /// 프로젝트 컴파일 대화상자를 연다. 열린 시트를 먼저 저장한다.
    pub fn open_compile_dialog(
        &mut self,
        dir: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Err(err) = self.save_open_sheet(cx) {
            self.notify(format!("시트를 저장할 수 없습니다: {err}"), window, cx);
        }

        let Some(project) = self.find_group(dir) else {
            return;
        };

        let project_name = project.name.clone();
        let book = library::read_book(dir);
        let manuscript = library::manuscript_sheets(&project);
        let sheet_count = manuscript.len();

        let app = cx.entity().downgrade();
        let dialog_view = cx.new(|_| {
            CompileDialog::new(
                app,
                dir.to_path_buf(),
                project_name,
                book,
                sheet_count,
            )
        });

        let compile_action_view = dialog_view.clone();
        window.open_dialog(cx, move |dialog, _, _| {
            let ok_view = compile_action_view.clone();
            dialog
                .w(px(DIALOG_WIDTH))
                .title("프로젝트 컴파일")
                .close_button(true)
                .child(dialog_view.clone())
                .footer(
                    DialogFooter::new()
                        .child(
                            Button::new("compile-cancel")
                                .label("취소")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("compile-start")
                                .primary()
                                .label("컴파일...")
                                .on_click(move |_, window, cx| {
                                    ok_view.update(cx, |dialog, cx| {
                                        dialog.start_compile(window, cx);
                                    });
                                }),
                        ),
                )
        });
    }
}
