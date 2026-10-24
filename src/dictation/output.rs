//! 받아 적은 문단을 기록 파일에 쓰는 스레드(ADR-0054, ADR-0055).
//!
//! 녹음 중 기록 파일에 쓰는 것은 이 스레드 하나뿐이다. 받아 적은 원문은 기다리지 않고 받는 대로 덧붙인다.
//! 사용 언어를 골랐으면 원문을 모아([`BATCH_CHARS`]·[`BATCH_WAIT`]) 번역 스레드로 넘기고, 답(다른 언어는
//! 번역문, 사용 언어는 다듬은 글)이 오면 그 묶음의 원문 바로 아래에 `> …` 줄로 끼워 넣는다. 모델이 느려도
//! 원문은 밀리지 않고, 답은 차례대로 제자리에 들어간다. 모델이 실패하거나 [`TRANSLATE_TIMEOUT`]을 넘기면 그
//! 묶음은 원문만 남는다.
//!
//! 끼워 넣으려면 파일 가운데를 고쳐야 하므로 기록 전체(에디터 본문의 줄)를 들고 있다가 바뀔 때마다 원자적으로
//! 다시 쓴다. 화면은 같은 줄 번호로 에디터에 끼워 넣는다([`Output::Inserted`]).

use std::collections::VecDeque;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use crate::agent::Backend;
use crate::agent::dictation::{Glossary, Language, Rendered, TranslateRequest, translate_blocking};

/// 모은 글이 이만큼(글자)을 넘으면 번역 스레드로 넘긴다.
const BATCH_CHARS: usize = 200;

/// 묶음의 첫 문단을 받고 이만큼 지나면 모은 글이 적어도 넘긴다.
const BATCH_WAIT: Duration = Duration::from_secs(15);

/// 번역 답을 이만큼 기다린다. 넘으면 그 묶음은 원문만 남는다.
const TRANSLATE_TIMEOUT: Duration = Duration::from_secs(30);

/// 번역 맥락으로 함께 보낼 앞 문단 수.
const CONTEXT_PARAGRAPHS: usize = 4;

/// 문단을 기다리는 사이 번역 답을 살피는 간격.
const POLL: Duration = Duration::from_millis(200);

/// 녹음 중 번역·다듬기 설정.
#[derive(Clone, Debug)]
pub struct TranslationSetup {
    pub backend: Backend,
    pub language: &'static Language,
}

/// 쓰기 스레드가 알리는 일.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Output {
    /// 원문 한 줄을 끝에 덧붙였다.
    Written {
        text: String,
        originals: Vec<String>,
    },
    /// 에디터 본문의 `line`번째 줄(0부터) 앞에 `text`(줄들)를 끼워 넣었다.
    Inserted { line: usize, text: String },
    /// 답을 기다리는 묶음 수.
    Pending(usize),
    /// 번역·다듬기를 하지 못해 그 묶음은 원문만 남는다.
    TranslationFailed(String),
}

/// 묶음을 받아 원문 아래에 붙일 줄을 돌려주는 것(번역 스레드에서 돈다).
type Translate = Box<dyn FnMut(&[String]) -> Result<Vec<String>, String> + Send>;

/// 문단 채널이 닫힐 때까지 받은 문단을 기록에 쓴다. 번역을 켰으면 남은 답까지 다 받고 끝낸다.
/// 쓰기에 실패하면 그 까닭을 돌려준다.
pub(super) fn write_out(
    record: Record,
    paragraphs: Receiver<String>,
    translation: Option<TranslationSetup>,
    send: impl FnMut(Output),
) -> io::Result<()> {
    let translate = translation.map(|setup| {
        let mut translator = Translator::new(setup);
        Box::new(move |originals: &[String]| translator.translate(originals)) as Translate
    });
    write_out_with(record, paragraphs, translate, send)
}

fn write_out_with(
    mut record: Record,
    paragraphs: Receiver<String>,
    translate: Option<Translate>,
    mut send: impl FnMut(Output),
) -> io::Result<()> {
    let Some(mut translate) = translate else {
        for paragraph in paragraphs {
            record.append(&paragraph)?;
            send(Output::Written {
                text: paragraph.clone(),
                originals: vec![paragraph],
            });
        }
        return Ok(());
    };

    // 번역 스레드: 묶음을 차례로 보내 답을 돌려준다(앞 묶음의 답이 다음 묶음의 맥락이 된다).
    let (job_tx, job_rx) = mpsc::channel::<(u64, Vec<String>)>();
    let (done_tx, done_rx) = mpsc::channel::<(u64, Result<Vec<String>, String>)>();
    let translator = std::thread::Builder::new()
        .name("writer-stt-ai".into())
        .spawn(move || {
            for (id, originals) in job_rx {
                if done_tx.send((id, translate(&originals))).is_err() {
                    break;
                }
            }
        })?;

    let mut batch: Vec<String> = Vec::new();
    let mut first_at: Option<Instant> = None;
    let mut next_id = 0;
    // 답을 기다리는 묶음과 그 답을 끼워 넣을 줄(묶음의 마지막 원문 바로 아래).
    let mut pending: Vec<(u64, usize)> = Vec::new();
    let mut ended = false;
    let result = (|| {
        loop {
            while let Ok((id, answer)) = done_rx.try_recv() {
                apply(&mut record, &mut pending, id, answer, &mut send)?;
            }
            if ended {
                if pending.is_empty() {
                    return Ok(());
                }
                match done_rx.recv() {
                    Ok((id, answer)) => apply(&mut record, &mut pending, id, answer, &mut send)?,
                    Err(_) => return Ok(()),
                }
                continue;
            }
            let wait =
                first_at.map_or(POLL, |at| BATCH_WAIT.saturating_sub(at.elapsed()).min(POLL));
            let close = match paragraphs.recv_timeout(wait) {
                Ok(paragraph) => {
                    record.append(&paragraph)?;
                    send(Output::Written {
                        text: paragraph.clone(),
                        originals: vec![paragraph.clone()],
                    });
                    batch.push(paragraph);
                    first_at.get_or_insert_with(Instant::now);
                    batch.iter().map(|p| p.chars().count()).sum::<usize>() >= BATCH_CHARS
                }
                Err(RecvTimeoutError::Timeout) => {
                    first_at.is_some_and(|at| at.elapsed() >= BATCH_WAIT)
                }
                Err(RecvTimeoutError::Disconnected) => {
                    ended = true;
                    true
                }
            };
            if close && !batch.is_empty() {
                first_at = None;
                pending.push((next_id, record.len()));
                // 번역 스레드가 끝났으면(있을 수 없다) 그 묶음은 원문만 남는다.
                if job_tx.send((next_id, std::mem::take(&mut batch))).is_err() {
                    pending.pop();
                }
                next_id += 1;
                send(Output::Pending(pending.len()));
            }
        }
    })();
    drop(job_tx);
    let _ = translator.join();
    result
}

/// 묶음 `id`의 답을 그 자리에 끼워 넣고, 뒤에서 기다리는 묶음의 자리를 그만큼 민다.
fn apply(
    record: &mut Record,
    pending: &mut Vec<(u64, usize)>,
    id: u64,
    answer: Result<Vec<String>, String>,
    send: &mut impl FnMut(Output),
) -> io::Result<()> {
    let Some(ix) = pending.iter().position(|(pending_id, _)| *pending_id == id) else {
        return Ok(());
    };
    let (_, line) = pending.remove(ix);
    match answer {
        Ok(lines) if !lines.is_empty() => {
            record.insert(line, &lines)?;
            for (_, later) in pending.iter_mut() {
                if *later >= line {
                    *later += lines.len();
                }
            }
            send(Output::Inserted {
                line,
                text: lines.join("\n"),
            });
        }
        Ok(_) => {}
        Err(reason) => send(Output::TranslationFailed(reason)),
    }
    send(Output::Pending(pending.len()));
    Ok(())
}

/// 번역 맥락(앞 문단·용어집)을 들고 문단 묶음을 번역한다.
struct Translator {
    setup: TranslationSetup,
    context: VecDeque<(String, String)>,
    glossary: Glossary,
}

impl Translator {
    fn new(setup: TranslationSetup) -> Self {
        Self {
            setup,
            context: VecDeque::new(),
            glossary: Glossary::default(),
        }
    }

    /// 묶음을 보내 원문 아래에 붙일 줄(`> 번역문`·`> 다듬은 글`)을 돌려준다. 그대로 둘 문단은 줄이 없다.
    fn translate(&mut self, originals: &[String]) -> Result<Vec<String>, String> {
        let request = TranslateRequest {
            backend: self.setup.backend.clone(),
            language: self.setup.language,
            context: self.context.iter().cloned().collect(),
            glossary: self.glossary.clone(),
            paragraphs: originals.to_vec(),
        };
        let translation = translate_blocking(&request, TRANSLATE_TIMEOUT)?;
        self.glossary.merge(translation.terms);
        let mut lines = Vec::new();
        for (original, rendered) in originals.iter().zip(translation.paragraphs) {
            let output = match rendered {
                Rendered::Translated(text) | Rendered::Cleaned(text) if !broken(&text) => text,
                _ => original.clone(),
            };
            if output != *original {
                lines.push(format!("> {output}"));
            }
            self.context.push_back((original.clone(), output));
            if self.context.len() > CONTEXT_PARAGRAPHS {
                self.context.pop_front();
            }
        }
        Ok(lines)
    }
}

/// 모델의 답에 태그 조각이 남았는지. 그런 글은 쓰지 않고 원문을 둔다.
fn broken(text: &str) -> bool {
    ["<tr ", "<fix ", "<p ", "<same ", "</"]
        .iter()
        .any(|tag| text.contains(tag))
}

/// 기록 파일: 에디터 본문의 줄(줄 하나가 문단, ADR-0011)을 들고 있다가 바뀔 때마다 마크다운으로 원자적으로
/// 다시 쓴다. 파일은 표준 마크다운이라 문단 사이에 빈 줄이 들어간다(빈 줄 없이 이으면 다시 열 때 한 문단으로
/// 합쳐진다). 녹음 중에는 이 스레드만 쓰므로 바꿔치기해도 잃는 글이 없다.
pub(super) struct Record {
    path: PathBuf,
    lines: Vec<String>,
}

impl Record {
    /// 기록 파일을 읽어 연다(없으면 빈 기록).
    pub(super) fn open(path: &Path) -> io::Result<Self> {
        let text = match crate::library::read_sheet(path) {
            Ok(text) => text,
            Err(err) if err.kind() == io::ErrorKind::NotFound => String::new(),
            Err(err) => return Err(err),
        };
        let lines = crate::text_transform::to_editor(&text)
            .lines()
            .map(str::to_string)
            .collect();
        Ok(Self {
            path: path.to_path_buf(),
            lines,
        })
    }

    /// 줄 수(끝에 덧붙일 자리).
    fn len(&self) -> usize {
        self.lines.len()
    }

    /// 문단을 끝에 한 줄로 덧붙인다.
    pub(super) fn append(&mut self, paragraph: &str) -> io::Result<()> {
        self.lines.push(paragraph.to_string());
        self.save()
    }

    /// `line`번째 줄 앞에 줄들을 끼워 넣는다.
    fn insert(&mut self, line: usize, lines: &[String]) -> io::Result<()> {
        let at = line.min(self.lines.len());
        self.lines.splice(at..at, lines.iter().cloned());
        self.save()
    }

    fn save(&self) -> io::Result<()> {
        let body = self.lines.join("\n") + "\n";
        crate::library::write_sheet(&self.path, &crate::text_transform::to_markdown(&body))
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("writer-stt-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("a.md")
    }

    #[test]
    fn record_appends_and_inserts_as_markdown_paragraphs() {
        let path = temp("record");
        std::fs::write(&path, "# 제목\n").unwrap();
        let mut record = Record::open(&path).unwrap();
        record.append("첫 문단.").unwrap();
        record.append("둘째.").unwrap();
        record
            .insert(2, &["> 다듬은 첫 문단.".to_string()])
            .unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text, "# 제목\n\n첫 문단.\n\n> 다듬은 첫 문단.\n\n둘째.\n");
        assert_eq!(
            crate::text_transform::to_editor(&text),
            "# 제목\n첫 문단.\n> 다듬은 첫 문단.\n둘째.\n"
        );
        // 다시 열어도 같은 줄로 읽는다.
        assert_eq!(Record::open(&path).unwrap().len(), 4);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn without_translation_writes_each_paragraph() {
        let path = temp("plain");
        let (tx, rx) = mpsc::channel();
        tx.send("하나".to_string()).unwrap();
        tx.send("둘".to_string()).unwrap();
        drop(tx);
        let mut written = Vec::new();
        write_out_with(Record::open(&path).unwrap(), rx, None, |output| {
            if let Output::Written { text, .. } = output {
                written.push(text);
            }
        })
        .unwrap();
        assert_eq!(written, ["하나", "둘"]);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "하나\n\n둘\n");
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    /// 답이 늦어도 원문은 바로 쓰이고, 답은 그 묶음 아래에 차례로 들어간다.
    #[test]
    fn slow_answers_go_under_their_batch_without_holding_originals() {
        let path = temp("slow");
        let (tx, rx) = mpsc::channel();
        let long = "가".repeat(BATCH_CHARS);
        let translate: Translate = Box::new(|originals: &[String]| {
            std::thread::sleep(Duration::from_millis(300));
            Ok(originals
                .iter()
                .map(|o| format!("> {}!", o.chars().count()))
                .collect())
        });
        let feeder = {
            let long = long.clone();
            std::thread::spawn(move || {
                tx.send(long.clone()).unwrap();
                // 첫 답을 기다리는 사이에 다음 원문이 온다.
                std::thread::sleep(Duration::from_millis(50));
                tx.send("짧은 말".to_string()).unwrap();
                tx.send(long).unwrap();
            })
        };
        let mut outputs = Vec::new();
        write_out_with(
            Record::open(&path).unwrap(),
            rx,
            Some(translate),
            |output| outputs.push(output),
        )
        .unwrap();
        feeder.join().unwrap();
        let written: Vec<_> = outputs
            .iter()
            .filter(|o| matches!(o, Output::Written { .. }))
            .collect();
        assert_eq!(written.len(), 3);
        // 원문 셋이 답보다 먼저 쓰였다.
        let first_insert = outputs
            .iter()
            .position(|o| matches!(o, Output::Inserted { .. }))
            .unwrap();
        let last_written = outputs
            .iter()
            .rposition(|o| matches!(o, Output::Written { .. }))
            .unwrap();
        assert!(last_written < first_insert);
        let text = crate::text_transform::to_editor(&std::fs::read_to_string(&path).unwrap());
        let lines: Vec<_> = text.lines().collect();
        let answer = format!("> {BATCH_CHARS}!");
        assert_eq!(
            lines,
            [
                long.as_str(),
                answer.as_str(),
                "짧은 말",
                long.as_str(),
                "> 4!",
                answer.as_str()
            ]
        );
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
