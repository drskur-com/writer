//! Nemotron 인식기: gpui-kit `speech`의 [`SpeechRecognizer`]를 transcribe.cpp로 구현한다(ADR-0054).
//!
//! 녹음 한 번마다 작업 스레드 하나가 모델 세션을 들고 스트림에 소리를 넣는다. 말이 끊기면(글이 [`PAUSE_MS`]
//! 동안 바뀌지 않으면) 그때까지를 한 문단으로 확정해 쓰기 스레드([`super::output`])로 넘기고 새 스트림을 연다.
//! 문단마다 스트림을 새로 열어 오래 켜 두어도 스트림의 글이 한없이 자라지 않는다.
//!
//! 기록 파일은 녹음을 시작하기 전에 정한다(새 기록이면 미리 만들고, 지난 기록이면 그 끝에 잇는다).
//! 파일 쓰기는 쓰기 스레드만 한다. 멈추기를 기다리다 세션이 먼저 끝나도
//! 남은 소리를 마저 인식해 저장하고, 쓰기 스레드까지 다 쓰면 [`Event::Done`]을 알린다.

use std::cell::{Cell, RefCell};
use std::fs::File;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc as std_mpsc;

use futures::StreamExt as _;
use futures::channel::mpsc;
use gpui_kit::component::speech::{RecognitionSession, SpeechError, SpeechRecognizer, SpeechSink};
use gpui_kit::gpui::*;
use transcribe_cpp::{Model, RunOptions, StreamOptions};

use super::output::{Record, write_out};

/// 모델이 받는 소리: 16 kHz 모노(gpui-kit `AudioFormat`의 기본값과 같다).
const SAMPLE_RATE: usize = 16_000;

/// 한 번에 스트림에 넣는 소리 길이(160 ms). 마이크는 더 잘게 보내 모아서 넣는다.
const FEED_SAMPLES: usize = SAMPLE_RATE * 160 / 1000;

/// 글이 이만큼(ms) 바뀌지 않으면 말이 끊긴 것으로 보고 문단을 맺는다.
const PAUSE_MS: i64 = 2_000;

/// 쉬지 않고 말해도 문단을 이 길이(ms)에서 맺는다. 말없이 켜 두었을 때도 이 길이마다 스트림을 새로 연다.
const MAX_PARAGRAPH_MS: i64 = 180_000;

/// 받아쓰기를 쓰는 쪽에 알리는 일.
pub enum Event {
    /// 파일 끝에 받아 적은 문단을 한 줄로 덧붙였다.
    Written(SharedString),
    /// 작업 스레드가 끝났다. 이제 기록 파일에 더 쓰지 않는다.
    Done,
}

/// 작업 스레드가 메인 스레드로 보내는 일.
enum WorkerEvent {
    Ready,
    Hypothesis(String),
    Written(String),
    Failed(String),
    Finished,
}

/// [`Event`]를 받는 쪽.
pub type EventHandler = Rc<dyn Fn(Event, &mut App)>;

/// Nemotron으로 받아쓰고 `target` 파일 끝에 기록하는 인식기.
pub struct NemotronRecognizer {
    /// 모델 파일(GGUF).
    pub model: PathBuf,
    /// 이번 녹음의 기록 파일. 녹음을 시작할 때 읽는다.
    pub target: Rc<RefCell<PathBuf>>,
    /// 작업 스레드를 띄웠으면 `true`가 된다. 시작하다 실패했는지(그러면 [`Event::Done`]이 오지 않는다) 본다.
    pub started: Rc<Cell<bool>>,
    pub on_event: EventHandler,
}

impl SpeechRecognizer for NemotronRecognizer {
    fn is_available(&self, _cx: &App) -> bool {
        self.model.is_file()
    }

    fn start(
        &self,
        sink: SpeechSink,
        cx: &mut App,
    ) -> Result<Box<dyn RecognitionSession>, SpeechError> {
        let (audio_tx, audio_rx) = std_mpsc::channel::<Vec<i16>>();
        let (event_tx, mut events) = mpsc::unbounded();
        let (model, target) = (self.model.clone(), self.target.borrow().clone());
        std::thread::Builder::new()
            .name("writer-stt".into())
            .spawn(move || run(&model, &target, audio_rx, event_tx))
            .map_err(SpeechError::recognizer)?;
        self.started.set(true);

        // 세션보다 오래 산다: 멈추기를 기다리다 세션이 끝나도 남은 문단과 `Done`을 넘긴다.
        let on_event = self.on_event.clone();
        cx.spawn(async move |cx| {
            while let Some(event) = events.next().await {
                let finished = matches!(event, WorkerEvent::Finished | WorkerEvent::Failed(_));
                cx.update(|cx| match event {
                    WorkerEvent::Ready => sink.ready(cx),
                    WorkerEvent::Hypothesis(text) => sink.hypothesis(text, cx),
                    WorkerEvent::Written(text) => on_event(Event::Written(text.into()), cx),
                    // 녹음 상태가 `SpeechEvent::Error`로 알린다.
                    WorkerEvent::Failed(reason) => {
                        sink.error(SpeechError::recognizer(io::Error::other(reason)), cx);
                        on_event(Event::Done, cx);
                    }
                    WorkerEvent::Finished => {
                        sink.finish(cx);
                        on_event(Event::Done, cx);
                    }
                });
                if finished {
                    break;
                }
            }
        })
        .detach();
        Ok(Box::new(Session {
            audio: Some(audio_tx),
        }))
    }
}

/// 진행 중인 녹음. 소리를 작업 스레드로 넘긴다.
struct Session {
    audio: Option<std_mpsc::Sender<Vec<i16>>>,
}

impl RecognitionSession for Session {
    fn push_audio(&mut self, samples: &[i16], _cx: &mut App) {
        if let Some(audio) = &self.audio {
            let _ = audio.send(samples.to_vec());
        }
    }

    /// 소리 채널을 닫는다. 작업 스레드는 남은 소리를 인식해 마지막 문단을 쓰고 `Finished`를 보낸다.
    fn finish(&mut self, _cx: &mut App) {
        self.audio = None;
    }
}

/// 작업 스레드: 소리 채널이 닫힐 때까지 문단 단위로 인식해 쓰기 스레드로 넘기고, 쓰기 스레드가 다 쓰면 끝낸다.
fn run(
    model: &Path,
    target: &Path,
    audio: std_mpsc::Receiver<Vec<i16>>,
    events: mpsc::UnboundedSender<WorkerEvent>,
) {
    let send = |event| {
        let _ = events.unbounded_send(event);
    };
    let (paragraph_tx, paragraph_rx) = std_mpsc::channel::<String>();
    let mut writer = None;
    let result = (|| {
        // 모델은 녹음마다 불러(메모리 매핑이라 금방이다) 녹음이 끝나면 놓는다. 앱이 끝날 때까지 들고 있으면
        // 종료 중 ggml Metal 장치를 정리하다 멈춘다(GGML_ASSERT).
        let model = Model::load(model).map_err(|err| err.to_string())?;
        let mut session = model.session().map_err(|err| err.to_string())?;
        send(WorkerEvent::Ready);
        let record = Record::open(target).map_err(|err| err.to_string())?;
        let output_events = events.clone();
        writer = Some(
            std::thread::Builder::new()
                .name("writer-stt-out".into())
                .spawn(move || {
                    write_out(record, paragraph_rx, |text| {
                        let _ = output_events.unbounded_send(WorkerEvent::Written(text));
                    })
                })
                .map_err(|err| err.to_string())?,
        );
        // 언어는 모델이 알아낸다(한국어·영어 섞인 말도 읽는다).
        let run_options = RunOptions::default();
        let mut pending: Vec<f32> = Vec::with_capacity(FEED_SAMPLES * 2);
        let mut ended = false;
        while !ended {
            let mut stream = session
                .stream(&run_options, &StreamOptions::default())
                .map_err(|err| err.to_string())?;
            let mut shown = String::new();
            let mut changed_ms = 0;
            loop {
                match audio.recv() {
                    Ok(samples) => pending.extend(samples.iter().map(|&s| f32::from(s) / 32768.0)),
                    Err(_) => {
                        ended = true;
                        break;
                    }
                }
                if pending.len() < FEED_SAMPLES {
                    continue;
                }
                let update = stream.feed(&pending).map_err(|err| err.to_string())?;
                pending.clear();
                if update.committed_changed || update.tentative_changed {
                    changed_ms = update.input_received_ms;
                    let text = stream.text().display();
                    if text != shown {
                        send(WorkerEvent::Hypothesis(text.clone()));
                        shown = text;
                    }
                }
                let spoken = !shown.trim().is_empty();
                let paused = update.input_received_ms - changed_ms >= PAUSE_MS;
                if (spoken && paused) || update.input_received_ms >= MAX_PARAGRAPH_MS {
                    break;
                }
            }
            if ended && !pending.is_empty() {
                stream.feed(&pending).map_err(|err| err.to_string())?;
                pending.clear();
            }
            stream.finalize().map_err(|err| err.to_string())?;
            let text = stream.text().full;
            drop(stream);
            let paragraph = text.trim();
            // 쓰기 스레드가 이미 끝났으면(쓰기 실패) 아래에서 그 까닭을 알린다.
            if !paragraph.is_empty() && paragraph_tx.send(paragraph.to_string()).is_err() {
                break;
            }
            send(WorkerEvent::Hypothesis(String::new()));
        }
        Ok::<_, String>(())
    })();
    // 문단 채널을 닫고 쓰기 스레드가 남은 문단을 다 쓸 때까지 기다린다.
    drop(paragraph_tx);
    let written = match writer.map(std::thread::JoinHandle::join) {
        Some(Ok(Err(err))) => Err(err.to_string()),
        Some(Err(_)) => Err("record writer panicked".to_string()),
        _ => Ok(()),
    };
    send(match result.and(written) {
        Ok(()) => WorkerEvent::Finished,
        Err(reason) => WorkerEvent::Failed(reason),
    });
}

/// 기록의 제목 줄(`# 받아쓰기 YYYY-MM-DD HH:MM`, 지금 시각).
fn title_line() -> String {
    let started = chrono::Local::now().format("%Y-%m-%d %H:%M");
    format!("# {}\n", t!("dictation.file_title", time = started))
}

/// 새 기록 파일(`<dir>/<ULID>.md`)을 제목 줄과 함께 만든다.
pub fn create_record(dir: &Path) -> io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("{}.md", ulid::Ulid::generate()));
    let mut file = File::options().create_new(true).write(true).open(&path)?;
    file.write_all(title_line().as_bytes())?;
    Ok(path)
}

/// 비어 있는 기록(시트 목록의 작성 버튼으로 만든 시트)에 녹음하기 전에 제목 줄을 넣는다. 넣었으면 `true`.
/// 제목이 없으면 첫 문단이 기록 이름이 되고, 요약을 넣으면 "요약"이 이름이 된다.
pub fn ensure_record_title(path: &Path) -> io::Result<bool> {
    if !std::fs::read_to_string(path)?.trim().is_empty() {
        return Ok(false);
    }
    std::fs::write(path, title_line())?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    // `super::*`는 gpui의 `test` 매크로까지 들여와 `#[test]`를 가린다.
    use super::{SAMPLE_RATE, WorkerEvent, create_record, ensure_record_title, run};
    use futures::StreamExt as _;

    #[test]
    fn create_record_writes_title_line() {
        let dir = std::env::temp_dir().join(format!("writer-stt-{}", std::process::id()));
        let path = create_record(&dir).unwrap();
        assert!(path.extension().is_some_and(|ext| ext == "md"));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# ") && text.ends_with('\n') && text.lines().count() == 1);
        // 제목이 있으면 그대로, 비었으면 제목 줄을 넣는다.
        assert!(!ensure_record_title(&path).unwrap());
        std::fs::write(&path, " \n").unwrap();
        assert!(ensure_record_title(&path).unwrap());
        assert!(std::fs::read_to_string(&path).unwrap().starts_with("# "));
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// 실제 모델로 말 → 3초 쉼 → 말을 넣으면 문단 두 개가 파일에 쌓인다.
    /// `WRITER_STT_MODEL`(GGUF)과 `WRITER_STT_WAV`(16 kHz 모노 16비트)를 주고 `--ignored`로 돌린다.
    #[test]
    #[ignore]
    fn pause_splits_paragraphs_with_real_model() {
        let (Ok(model), Ok(wav)) = (
            std::env::var("WRITER_STT_MODEL"),
            std::env::var("WRITER_STT_WAV"),
        ) else {
            return;
        };
        let bytes = std::fs::read(wav).unwrap();
        let data = bytes.windows(4).position(|w| w == b"data").unwrap() + 8;
        let speech: Vec<i16> = bytes[data..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&b| i16::from_le_bytes(b))
            .collect();
        let silence = vec![0i16; SAMPLE_RATE * 3];
        let dir = std::env::temp_dir().join(format!("writer-stt-real-{}", std::process::id()));
        let file = create_record(&dir).unwrap();
        let (audio_tx, audio_rx) = std::sync::mpsc::channel();
        let (event_tx, events) = futures::channel::mpsc::unbounded();
        let worker = {
            let file = file.clone();
            std::thread::spawn(move || run(model.as_ref(), &file, audio_rx, event_tx))
        };
        // 마이크처럼 10 ms씩 보낸다.
        for chunk in speech
            .iter()
            .chain(&silence)
            .chain(&speech)
            .chain(&silence)
            .copied()
            .collect::<Vec<_>>()
            .chunks(160)
        {
            audio_tx.send(chunk.to_vec()).unwrap();
        }
        drop(audio_tx);
        worker.join().unwrap();
        let events: Vec<_> = futures::executor::block_on(events.collect());
        let paragraphs: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                WorkerEvent::Written(text) => Some(text.clone()),
                WorkerEvent::Failed(reason) => panic!("{reason}"),
                _ => None,
            })
            .collect();
        assert!(matches!(events.last(), Some(WorkerEvent::Finished)));
        eprintln!("{paragraphs:#?}");
        assert_eq!(paragraphs.len(), 2);
        let text = std::fs::read_to_string(&file).unwrap();
        eprintln!("{text}");
        // 제목, 빈 줄, 문단, 빈 줄, 문단.
        assert_eq!(text.lines().count(), 5);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
