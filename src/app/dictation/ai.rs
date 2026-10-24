//! 받아쓰기의 녹음 중 요약(ADR-0055). 녹음 중 번역은 쓰기 스레드(`dictation::output`)가 한다.
//!
//! 파일 하나에는 쓰는 쪽이 하나다. 요약은 기록과 짝인 요약 파일(`STT/.summaries/<기록>.md`)에만 쓰고,
//! 녹음이 끝난 뒤 사용자가 고르면 기록 맨 위에 넣는다. 요약은 따로 연 창(요약 보기)에서 본다. 녹음하며 다른
//! 시트에 메모할 수 있게 모달이 아닌 창이다.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _};
use gpui_kit::gpui::prelude::FluentBuilder as _;
use gpui_kit::gpui::*;

use super::super::WriterApp;
use super::super::style::Palette;
use crate::agent::dictation::{Language, SummaryRequest};
use crate::agent::{self, Backend};
use crate::{icons, library};

/// 받아쓰기 요약 상태(앱에 하나). 녹음이 끝난 뒤 마지막 요약까지 이어진다.
#[derive(Default)]
pub(in crate::app) struct DictationAi {
    summary: Option<SummaryJob>,
    /// 열어 둔 요약 창(하나만 둔다).
    summary_window: Option<AnyWindowHandle>,
    /// 마지막 요약·요약 넣기 실패.
    summary_error: Option<SharedString>,
}

/// 녹음 중(과 끝난 뒤 마지막 요약까지)의 요약.
struct SummaryJob {
    record: PathBuf,
    backend: Backend,
    language: Option<&'static Language>,
    interval: Duration,
    /// 최신 요약(마크다운 글머리표 줄).
    text: String,
    /// 아직 요약에 넣지 않은 원문.
    unsummarized: String,
    last: Instant,
    running: Option<Task<()>>,
    /// 녹음이 끝났다. 남은 원문을 마지막으로 요약하면 끝낸다.
    finishing: bool,
}

/// 기록의 요약 파일(`<기록 폴더>/.summaries/<기록 파일 이름>`). 점 폴더라 라이브러리에 시트로 잡히지 않는다.
pub(super) fn summary_path(record: &Path) -> Option<PathBuf> {
    Some(
        record
            .parent()?
            .join(".summaries")
            .join(record.file_name()?),
    )
}

/// 에디터 본문의 요약 구간 머리줄.
fn summary_heading() -> String {
    format!("## {}", t!("dictation.summary.heading"))
}

/// 본문(에디터 줄)의 요약 구간: 머리줄과 그 뒤의 글머리표 줄. (시작 줄, 끝 줄(제외)).
fn summary_section(lines: &[&str]) -> Option<(usize, usize)> {
    let heading = summary_heading();
    let start = lines.iter().position(|line| line.trim_end() == heading)?;
    let end = lines[start + 1..]
        .iter()
        .position(|line| !line.starts_with("- "))
        .map_or(lines.len(), |offset| start + 1 + offset);
    Some((start, end))
}

/// 본문의 요약 구간을 `summary`로 바꾸거나(없으면 제목 줄 뒤에) 넣는다.
fn with_summary(body: &str, summary: &str) -> String {
    let lines: Vec<&str> = body.lines().collect();
    let heading = summary_heading();
    let bullets: Vec<&str> = summary
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    let mut out: Vec<&str> = Vec::with_capacity(lines.len() + bullets.len() + 1);
    let (keep_before, keep_after) = match summary_section(&lines) {
        Some((start, end)) => (start, end),
        None => {
            let after_title = usize::from(lines.first().is_some_and(|line| line.starts_with("# ")));
            (after_title, after_title)
        }
    };
    out.extend(&lines[..keep_before]);
    out.push(&heading);
    out.extend(bullets);
    out.extend(&lines[keep_after..]);
    let mut text = out.join("\n");
    if body.ends_with('\n') {
        text.push('\n');
    }
    text
}

impl WriterApp {
    /// 녹음을 시작할 때: 요약을 켰으면 요약 일을 연다. 지난 요약(요약 파일, 없으면 기록의 요약 구간)에서 잇는다.
    pub(super) fn start_summary_job(&mut self, record: &Path, cx: &App) {
        self.dictation_ai.summary = None;
        self.dictation_ai.summary_error = None;
        let config = crate::config::Config::load().dictation;
        if !config.summarize {
            return;
        }
        let Some(backend) = config.backend(cx.global::<crate::settings::Prefs>()) else {
            return;
        };
        let previous = summary_path(record)
            .and_then(|path| std::fs::read_to_string(path).ok())
            .or_else(|| {
                let body = library::read_sheet(record).ok()?;
                let body = crate::text_transform::to_editor(&body);
                let lines: Vec<&str> = body.lines().collect();
                let (start, end) = summary_section(&lines)?;
                Some(lines[start + 1..end].join("\n"))
            })
            .unwrap_or_default();
        self.dictation_ai.summary = Some(SummaryJob {
            record: record.to_path_buf(),
            backend,
            language: config.translation_language(),
            interval: Duration::from_secs(config.summary_minutes.max(1) * 60),
            text: previous,
            unsummarized: String::new(),
            last: Instant::now(),
            running: None,
            finishing: false,
        });
    }

    /// 쓰기 스레드가 기록에 덧붙인 원문을 요약할 글에 더하고, 때가 되었으면 요약한다.
    pub(super) fn on_summary_written(&mut self, originals: &[String], cx: &mut Context<Self>) {
        let Some(job) = &mut self.dictation_ai.summary else {
            return;
        };
        for original in originals {
            job.unsummarized.push_str(original);
            job.unsummarized.push('\n');
        }
        self.run_summary(false, cx);
    }

    /// 녹음(쓰기 스레드)이 끝났다: 남은 원문을 마지막으로 요약한다.
    pub(super) fn finish_summary_job(&mut self, cx: &mut Context<Self>) {
        let Some(job) = &mut self.dictation_ai.summary else {
            return;
        };
        job.finishing = true;
        self.run_summary(true, cx);
    }

    /// 요약할 때가 되었으면(`force`면 바로) 요약을 보낸다. 하나가 돌고 있으면 기다린다.
    fn run_summary(&mut self, force: bool, cx: &mut Context<Self>) {
        let Some(job) = &mut self.dictation_ai.summary else {
            return;
        };
        if job.running.is_some() {
            return;
        }
        if job.unsummarized.trim().is_empty() {
            if job.finishing {
                // 더 요약할 것이 없다. 요약 파일이 있으면 기록을 볼 때 넣을지 묻는다.
                self.dictation_ai.summary = None;
                cx.notify();
            }
            return;
        }
        if !force && job.last.elapsed() < job.interval {
            return;
        }
        let transcript = std::mem::take(&mut job.unsummarized);
        let answer = agent::dictation::summarize(SummaryRequest {
            backend: job.backend.clone(),
            language: job.language,
            previous: job.text.clone(),
            transcript: transcript.clone(),
        });
        job.running = Some(cx.spawn(async move |this, cx| {
            let result = answer
                .await
                .unwrap_or_else(|_| Err(t!("assistant.model.lookup_stopped").into()));
            this.update(cx, |this, cx| this.on_summary(result, transcript, cx))
                .ok();
        }));
    }

    /// 요약 답: 요약 파일을 고쳐 쓰고, 녹음이 끝났으면 남은 것을 마저 요약한다.
    fn on_summary(
        &mut self,
        result: Result<String, String>,
        transcript: String,
        cx: &mut Context<Self>,
    ) {
        let Some(job) = &mut self.dictation_ai.summary else {
            return;
        };
        job.running = None;
        job.last = Instant::now();
        match result {
            Ok(text) if !text.trim().is_empty() => {
                job.text = text;
                let written = summary_path(&job.record).map(|path| {
                    std::fs::create_dir_all(path.parent().unwrap_or(&path))
                        .and_then(|()| library::write_sheet(&path, &format!("{}\n", job.text)))
                });
                if let Some(Err(err)) = written {
                    self.dictation_ai.summary_error =
                        Some(t!("dictation.summary.failed", err = err).into());
                } else {
                    self.dictation_ai.summary_error = None;
                }
            }
            Ok(_) => {}
            Err(err) => {
                // 못 넣은 원문은 다음 요약에 다시 넣는다. 녹음이 끝났으면 더 기다리지 않는다.
                job.unsummarized.insert_str(0, &transcript);
                if job.finishing {
                    job.unsummarized.clear();
                }
                self.dictation_ai.summary_error =
                    Some(t!("dictation.summary.failed", err = err).into());
            }
        }
        let finishing = self
            .dictation_ai
            .summary
            .as_ref()
            .is_some_and(|job| job.finishing);
        self.run_summary(finishing, cx);
        cx.notify();
    }

    /// 요약 파일을 기록 맨 위(제목 줄 뒤)의 요약 구간에 넣고 요약 파일을 지운다.
    fn insert_summary(&mut self, record: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let result = (|| -> std::io::Result<()> {
            let summary_file = summary_path(&record).ok_or(std::io::ErrorKind::NotFound)?;
            let summary = std::fs::read_to_string(&summary_file)?;
            if self
                .editor
                .as_ref()
                .is_some_and(|editor| editor.path() == record)
            {
                self.save_open_sheet(cx)?;
            }
            let body = crate::text_transform::to_editor(&library::read_sheet(&record)?);
            let body = with_summary(&body, &summary);
            library::write_sheet(&record, &crate::text_transform::to_markdown(&body))?;
            std::fs::remove_file(&summary_file)
        })();
        match result {
            Ok(()) => {
                self.dictation_ai.summary_error = None;
                self.close_summary_window(cx);
                self.reload_recorded_sheet(&record, window, cx);
                self.load_previews(cx);
            }
            Err(err) => {
                self.dictation_ai.summary_error =
                    Some(t!("dictation.summary.insert_failed", err = err).into());
            }
        }
        cx.notify();
    }

    fn discard_summary(&mut self, record: &Path, cx: &mut Context<Self>) {
        if let Some(path) = summary_path(record) {
            let _ = std::fs::remove_file(path);
        }
        self.dictation_ai.summary_error = None;
        self.close_summary_window(cx);
        cx.notify();
    }

    /// 녹음하지 않을 때 녹음 막대 아래의 기록 도구: 마무리 중인 요약, 남은 요약 넣기·버리기.
    /// `record`는 고른 받아쓰기 기록.
    pub(super) fn render_record_tools(
        &self,
        record: &Path,
        cx: &mut Context<Self>,
        pal: Palette,
    ) -> Option<AnyElement> {
        let finishing = self
            .dictation_ai
            .summary
            .as_ref()
            .is_some_and(|job| job.record == record);
        let has_summary = !finishing && summary_path(record).is_some_and(|path| path.is_file());
        let error = self.dictation_ai.summary_error.clone();
        if !finishing && !has_summary && error.is_none() {
            return None;
        }

        let mut column = div().flex().flex_col().gap_2().text_sm();
        if finishing {
            column = column.child(
                div()
                    .text_color(pal.muted)
                    .child(t!("dictation.summary.finishing")),
            );
        } else if has_summary {
            let (insert, discard) = (record.to_path_buf(), record.to_path_buf());
            column = column.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(Icon::new(icons::summary()).small().text_color(pal.muted))
                    .child(div().flex_1().child(t!("dictation.summary.ready")))
                    .child(summary_button(record.to_path_buf(), cx))
                    .child(
                        Button::new("dictation-summary-discard")
                            .small()
                            .ghost()
                            .label(t!("dictation.summary.discard"))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.discard_summary(&discard, cx)
                            })),
                    )
                    .child(
                        Button::new("dictation-summary-insert")
                            .small()
                            .primary()
                            .label(t!("dictation.summary.insert"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.insert_summary(insert.clone(), window, cx)
                            })),
                    ),
            );
        }
        column = column.children(error.map(|error| div().text_color(pal.danger).child(error)));

        Some(column.into_any_element())
    }

    /// 녹음 중 "요약 보기" 버튼. 요약을 켜지 않았으면 `None`.
    pub(super) fn render_live_summary(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let record = self.dictation_ai.summary.as_ref()?.record.clone();
        Some(summary_button(record, cx).into_any_element())
    }

    /// 요약 창을 닫는다(넣거나 버린 뒤).
    fn close_summary_window(&mut self, cx: &mut App) {
        if let Some(handle) = self.dictation_ai.summary_window.take() {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
    }

    /// 기록 `record`의 요약 창을 연다. 이미 열려 있으면 닫고 이 기록으로 새로 연다.
    fn open_summary_window(&mut self, record: PathBuf, cx: &mut Context<Self>) {
        self.close_summary_window(cx);
        let title = self
            .previews
            .get(&record)
            .map(|(_, preview)| preview.title.clone())
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| t!("core.stt").to_string());
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(420.0), px(520.0)),
                cx,
            ))),
            window_min_size: Some(size(px(280.0), px(240.0))),
            titlebar: Some(TitlebarOptions {
                title: Some(t!("dictation.summary.window_title", title = title).into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        // 이 앱을 고치는 중이라 창을 만들며 앱을 읽을 수 없다. 첫 내용은 여기서 채워 넘긴다.
        let app = cx.entity();
        let mut view = SummaryView {
            record,
            text: String::new(),
            minutes: None,
            error: None,
        };
        view.refresh(self);
        match gpui_kit::open_window(options, cx, move |_, cx| {
            cx.new(|cx: &mut Context<SummaryView>| {
                cx.observe(&app, |this, app, cx| {
                    this.refresh(app.read(cx));
                    cx.notify();
                })
                .detach();
                view
            })
        }) {
            Ok((handle, _)) => self.dictation_ai.summary_window = Some(handle),
            Err(err) => eprintln!("요약 창을 열 수 없습니다: {err}"),
        }
    }
}

/// "요약 보기" 버튼.
fn summary_button(record: PathBuf, cx: &mut Context<WriterApp>) -> Button {
    Button::new("dictation-summary-view")
        .small()
        .ghost()
        .icon(icons::summary())
        .label(t!("dictation.summary.view"))
        .on_click(cx.listener(move |this, _, _, cx| this.open_summary_window(record.clone(), cx)))
}

/// 요약 창. 녹음 중이면 요약이 고쳐질 때마다, 끝났으면 요약 파일을 보인다.
struct SummaryView {
    record: PathBuf,
    text: String,
    /// 녹음 중 요약 주기(분). 녹음 중이 아니면 `None`.
    minutes: Option<u64>,
    error: Option<SharedString>,
}

impl SummaryView {
    /// 앱의 요약 일(이 기록이면)이나 요약 파일에서 요약을 읽는다.
    fn refresh(&mut self, app: &WriterApp) {
        self.error = app.dictation_ai.summary_error.clone();
        match app
            .dictation_ai
            .summary
            .as_ref()
            .filter(|job| job.record == self.record)
        {
            Some(job) => {
                self.text.clone_from(&job.text);
                self.minutes = Some(job.interval.as_secs() / 60);
            }
            None => {
                self.minutes = None;
                if let Some(text) =
                    summary_path(&self.record).and_then(|path| std::fs::read_to_string(path).ok())
                {
                    self.text = text;
                }
            }
        }
    }
}

impl Render for SummaryView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pal = Palette::from_theme(cx.theme());
        let empty = self.text.trim().is_empty();
        let body: SharedString = match (empty, self.minutes) {
            (false, _) => self.text.trim().to_string().into(),
            (true, Some(minutes)) => t!("dictation.summary.waiting", minutes = minutes).into(),
            (true, None) => t!("dictation.summary.none").into(),
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(pal.background)
            .text_color(pal.foreground)
            .child(
                div()
                    .id("dictation-summary")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_4()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .text_sm()
                    .when(empty, |this| this.text_color(pal.muted).child(body.clone()))
                    .when(!empty, |this| {
                        this.children(
                            body.lines()
                                .map(|line| div().child(SharedString::from(line.to_string()))),
                        )
                    }),
            )
            .when_some(self.minutes.filter(|_| !empty), |this, minutes| {
                this.child(
                    div()
                        .px_4()
                        .py_2()
                        .border_t_1()
                        .border_color(pal.panel_border)
                        .text_xs()
                        .text_color(pal.muted)
                        .child(t!("dictation.summary.waiting", minutes = minutes)),
                )
            })
            .children(self.error.clone().map(|error| {
                div()
                    .px_4()
                    .pb_2()
                    .text_xs()
                    .text_color(pal.danger)
                    .child(error)
            }))
    }
}

#[cfg(test)]
mod tests {
    // `super::*`는 gpui의 `test` 매크로까지 들여와 `#[test]`를 가린다.
    use super::{summary_heading, summary_path, with_summary};
    use std::path::{Path, PathBuf};

    #[test]
    fn summary_goes_after_title_and_replaces_old_section() {
        let heading = summary_heading();
        let body = "# 받아쓰기 2026-10-25 10:00\n첫 문단\n둘째 문단\n";
        let once = with_summary(body, "- 하나\n\n- 둘\n");
        assert_eq!(
            once,
            format!("# 받아쓰기 2026-10-25 10:00\n{heading}\n- 하나\n- 둘\n첫 문단\n둘째 문단\n")
        );
        let twice = with_summary(&once, "- 셋");
        assert_eq!(
            twice,
            format!("# 받아쓰기 2026-10-25 10:00\n{heading}\n- 셋\n첫 문단\n둘째 문단\n")
        );
        assert_eq!(with_summary("본문", "- a"), format!("{heading}\n- a\n본문"));
        // 마크다운으로 저장해도 구간이 그대로 돌아온다.
        let markdown = crate::text_transform::to_markdown(&twice);
        assert_eq!(crate::text_transform::to_editor(&markdown), twice);
    }

    #[test]
    fn summary_file_sits_in_hidden_folder() {
        assert_eq!(
            summary_path(Path::new("/lib/STT/01J.md")),
            Some(PathBuf::from("/lib/STT/.summaries/01J.md"))
        );
    }
}
