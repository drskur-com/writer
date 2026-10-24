//! 받아 적은 문단을 기록 파일에 쓰는 스레드(ADR-0054, ADR-0055).
//!
//! 녹음 중 기록 파일에 쓰는 것은 이 스레드 하나뿐이다. 사용 언어를 고르지 않았으면 문단을 받는 대로 덧붙이고,
//! 골랐으면 문단을 모아([`BATCH_CHARS`]·[`BATCH_WAIT`]) 모델에 보낸다: 다른 언어로 말한 문단은 번역해
//! "번역문, `> 원문`" 쌍으로, 사용 언어로 말한 문단은 다듬어 원문 대신 덧붙인다. 모델이 실패하거나
//! [`TRANSLATE_TIMEOUT`]을 넘기면 원문만 쓰고 넘어간다(받아쓰기는 끊기지 않는다).

use std::collections::VecDeque;
use std::fs::File;
use std::io::{self, Write as _};
use std::path::Path;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use crate::agent::Backend;
use crate::agent::dictation::{Glossary, Language, Rendered, TranslateRequest, translate_blocking};

/// 모은 글이 이만큼(글자)을 넘으면 번역을 보낸다.
const BATCH_CHARS: usize = 200;

/// 첫 문단을 받고 이만큼 지나면 모은 글이 적어도 번역을 보낸다.
const BATCH_WAIT: Duration = Duration::from_secs(15);

/// 번역 답을 이만큼 기다린다. 넘으면 원문만 쓴다.
const TRANSLATE_TIMEOUT: Duration = Duration::from_secs(30);

/// 번역 맥락으로 함께 보낼 앞 문단 수.
const CONTEXT_PARAGRAPHS: usize = 4;

/// 녹음 중 번역·다듬기 설정.
#[derive(Clone, Debug)]
pub struct TranslationSetup {
    pub backend: Backend,
    pub language: &'static Language,
}

/// 쓰기 스레드가 알리는 일.
pub(super) enum Output {
    /// 번역을 기다리는 문단(아직 파일에 쓰지 않았다).
    Queued(String),
    /// 파일에 덧붙였다. `text`는 덧붙인 줄들(에디터 본문의 줄), `originals`는 그 안의 받아쓰기 원문.
    Written {
        text: String,
        originals: Vec<String>,
    },
    /// 번역하지 못해 원문만 썼다.
    TranslationFailed(String),
}

/// 문단 채널이 닫힐 때까지 받은 문단을 기록에 쓴다. 쓰기에 실패하면 그 까닭을 돌려준다.
pub(super) fn write_out(
    mut record: Record,
    paragraphs: Receiver<String>,
    translation: Option<TranslationSetup>,
    mut send: impl FnMut(Output),
) -> io::Result<()> {
    let Some(setup) = translation else {
        for paragraph in paragraphs {
            record.append(&paragraph)?;
            send(Output::Written {
                text: paragraph.clone(),
                originals: vec![paragraph],
            });
        }
        return Ok(());
    };

    let mut translator = Translator::new(setup);
    let mut batch: Vec<String> = Vec::new();
    let mut first_at: Option<Instant> = None;
    loop {
        let received = match first_at {
            None => paragraphs
                .recv()
                .map_err(|_| RecvTimeoutError::Disconnected),
            Some(at) => paragraphs.recv_timeout(BATCH_WAIT.saturating_sub(at.elapsed())),
        };
        let ended = match received {
            Ok(paragraph) => {
                send(Output::Queued(paragraph.clone()));
                batch.push(paragraph);
                first_at.get_or_insert_with(Instant::now);
                if batch.iter().map(|p| p.chars().count()).sum::<usize>() < BATCH_CHARS {
                    continue;
                }
                false
            }
            Err(RecvTimeoutError::Timeout) => false,
            Err(RecvTimeoutError::Disconnected) => true,
        };
        if !batch.is_empty() {
            let originals = std::mem::take(&mut batch);
            first_at = None;
            let lines = match translator.translate(&originals) {
                Ok(lines) => lines,
                Err(reason) => {
                    send(Output::TranslationFailed(reason));
                    originals.clone()
                }
            };
            for line in &lines {
                record.append(line)?;
            }
            send(Output::Written {
                text: lines.join("\n"),
                originals,
            });
        }
        if ended {
            return Ok(());
        }
    }
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

    /// 묶음을 보내 기록에 쓸 줄을 돌려준다: 번역했으면 번역문과 `> 원문`, 다듬었으면 다듬은 글, 아니면 원문.
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
                Rendered::Translated(text) if !broken(&text) => {
                    lines.push(text.clone());
                    lines.push(format!("> {original}"));
                    text
                }
                Rendered::Cleaned(text) if !broken(&text) => {
                    lines.push(text.clone());
                    text
                }
                _ => {
                    lines.push(original.clone());
                    original.clone()
                }
            };
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

/// 기록 파일. 문단마다 끝에 덧붙인다.
///
/// 파일은 표준 마크다운이라 문단 사이에 빈 줄을 둔다(에디터에서는 줄 하나가 문단, ADR-0011).
/// 빈 줄 없이 이으면 다시 열 때 한 문단으로 합쳐지고, 인용(`> 원문`) 뒤의 번역문이 인용에 붙는다.
pub(super) struct Record {
    file: File,
    /// 다음 문단 앞에 넣을 줄바꿈.
    separator: &'static str,
}

impl Record {
    /// 기록 파일을 이어 쓰게 연다. 이미 쓴 글이 있으면 그 끝과 다음 문단 사이에 빈 줄 하나가 오게 한다.
    pub(super) fn open(path: &Path) -> io::Result<Self> {
        let bytes = std::fs::read(path).unwrap_or_default();
        let trailing = bytes
            .iter()
            .rev()
            .take_while(|&&byte| byte == b'\n')
            .count();
        let separator = match (bytes.is_empty(), trailing) {
            (true, _) => "",
            (false, 0) => "\n\n",
            (false, 1) => "\n",
            _ => "",
        };
        let file = File::options().create(true).append(true).open(path)?;
        Ok(Self { file, separator })
    }

    /// 문단을 한 줄로 덧붙인다.
    pub(super) fn append(&mut self, paragraph: &str) -> io::Result<()> {
        writeln!(self.file, "{}{paragraph}", self.separator)?;
        self.separator = "\n";
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_separates_paragraphs_with_blank_lines() {
        let dir = std::env::temp_dir().join(format!("writer-stt-out-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.md");
        std::fs::write(&path, "# 제목\n").unwrap();
        let mut record = Record::open(&path).unwrap();
        record.append("첫 문단.").unwrap();
        record.append("> 원문").unwrap();
        record.append("둘째.").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text, "# 제목\n\n첫 문단.\n\n> 원문\n\n둘째.\n");
        assert_eq!(
            crate::text_transform::to_editor(&text),
            "# 제목\n첫 문단.\n> 원문\n둘째.\n"
        );
        // 밖에서 끝 줄바꿈 없이 고쳤거나 빈 줄로 끝나도 빈 줄 하나로 잇는다.
        for (before, after) in [("A", "A\n\nB\n"), ("A\n\n", "A\n\nB\n"), ("", "B\n")] {
            std::fs::write(&path, before).unwrap();
            Record::open(&path).unwrap().append("B").unwrap();
            assert_eq!(std::fs::read_to_string(&path).unwrap(), after);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn without_translation_writes_each_paragraph() {
        let dir = std::env::temp_dir().join(format!("writer-stt-plain-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.md");
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send("하나".to_string()).unwrap();
        tx.send("둘".to_string()).unwrap();
        drop(tx);
        let mut written = Vec::new();
        write_out(Record::open(&path).unwrap(), rx, None, |output| {
            if let Output::Written { text, originals } = output {
                assert_eq!(originals, vec![text.clone()]);
                written.push(text);
            }
        })
        .unwrap();
        assert_eq!(written, ["하나", "둘"]);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "하나\n\n둘\n");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
