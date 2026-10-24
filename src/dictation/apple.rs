//! macOS 받아쓰기 인식기(ADR-0056): 기기 안의 `SFSpeechRecognizer`로 말하는 대로 받아 적는다.
//!
//! 인식기가 발화 끝을 알리면(결과에 `speechRecognitionMetadata`가 붙는다) 그때까지를 한 문단으로 맺어 쓰기
//! 스레드([`Pipeline`])로 넘기고 인식 요청을 새로 연다. 문단마다 요청을 새로 열어, 그 요청에 넣은 소리가 꼭
//! 그 문단의 소리가 된다(Cohere가 다시 받아 적는다). 발화 끝이 오지 않아도 [`MAX_PARAGRAPH`]에서 맺는다.
//!
//! 음성은 기기 밖으로 보내지 않는다(`requiresOnDeviceRecognition`). 그 언어의 기기 안 받아쓰기가 없으면
//! 녹음을 시작하지 않는다. 권한을 묻는 데 `Info.plist`의 `NSSpeechRecognitionUsageDescription`이 있어야 해서
//! (없이 물으면 프로세스가 끝난다) 번들 없이 실행하면(`cargo run`) 쓸 수 없다.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use block2::RcBlock;
use futures::StreamExt as _;
use futures::channel::mpsc;
use gpui_kit::component::speech::{
    AudioFormat, RecognitionSession, SpeechError, SpeechRecognizer, SpeechSink,
};
use gpui_kit::gpui::*;
use objc2::AnyThread as _;
use objc2::rc::Retained;
use objc2::runtime::NSObjectProtocol as _;
use objc2::sel;
use objc2_avf_audio::{AVAudioCommonFormat, AVAudioFormat, AVAudioPCMBuffer};
use objc2_foundation::{NSBundle, NSError, NSLocale, NSString};
use objc2_speech::{
    SFSpeechAudioBufferRecognitionRequest, SFSpeechRecognitionResult, SFSpeechRecognitionTask,
    SFSpeechRecognizer, SFSpeechRecognizerAuthorizationStatus,
};

use super::pipeline::{
    EventHandler, Pipeline, RecordingSetup, SAMPLE_RATE, WorkerEvent, forward_events,
};

/// 이 키가 없이 권한을 물으면 프로세스가 끝난다.
const USAGE_DESCRIPTION_KEY: &str = "NSSpeechRecognitionUsageDescription";

/// 발화 끝이 오지 않아도 문단을 이 길이에서 맺는다. 요청 하나에 소리가 너무 쌓이지 않게 한다.
const MAX_PARAGRAPH: usize = SAMPLE_RATE * 60;

/// macOS 받아쓰기로 받아 적고 기록 파일 끝에 쓰는 인식기.
pub struct AppleRecognizer {
    /// 받아쓰는 언어의 로캘(`ko-KR` 등).
    pub locale: String,
    /// 이번 녹음의 설정. 녹음을 시작할 때 읽는다.
    pub setup: Rc<RefCell<RecordingSetup>>,
    /// 작업을 띄웠으면 `true`가 된다. 시작하다 실패했는지(그러면 `Done`이 오지 않는다) 본다.
    pub started: Rc<Cell<bool>>,
    pub on_event: EventHandler,
}

impl AppleRecognizer {
    /// 녹음을 시작할 수 없는 까닭. 없으면 시작할 수 있다.
    fn unavailable(&self) -> Option<SpeechError> {
        if !has_usage_description() {
            return Some(error(t!("dictation.error.apple_bundle")));
        }
        if is_denied(authorization()) {
            return Some(error(t!("dictation.error.apple_permission")));
        }
        match recognizer(&self.locale) {
            // SAFETY: 살아 있는 인식기의 속성을 읽는다.
            Some(recognizer) if unsafe { recognizer.supportsOnDeviceRecognition() } => None,
            _ => Some(error(t!("dictation.error.apple_offline"))),
        }
    }
}

impl SpeechRecognizer for AppleRecognizer {
    fn audio_format(&self) -> AudioFormat {
        AudioFormat::default()
    }

    fn is_available(&self, _cx: &App) -> bool {
        // 쓸 수 없으면 녹음을 시작할 때 그 까닭을 알린다(`start`).
        true
    }

    fn start(
        &self,
        sink: SpeechSink,
        cx: &mut App,
    ) -> Result<Box<dyn RecognitionSession>, SpeechError> {
        if let Some(error) = self.unavailable() {
            return Err(error);
        }
        let recognizer = recognizer(&self.locale).ok_or(SpeechError::Unsupported)?;
        let (worker, worker_events) = mpsc::unbounded();
        let written = worker.clone();
        let pipeline = Pipeline::start(self.setup.borrow().clone(), move |text| {
            let _ = written.unbounded_send(WorkerEvent::Written(text));
        })
        .map_err(|err| SpeechError::recognizer(std::io::Error::other(err)))?;
        forward_events(
            worker_events,
            sink,
            self.on_event.clone(),
            &self.started,
            cx,
        );

        let (events, mut apple_events) = mpsc::unbounded();
        let recognition = Rc::new(RefCell::new(Recognition::new(
            recognizer, pipeline, worker, events,
        )?));
        if authorization() == SFSpeechRecognizerAuthorizationStatus::Authorized {
            recognition.borrow_mut().begin();
        } else {
            request_authorization(recognition.borrow().events.clone());
        }
        // 세션보다 오래 산다: 멈춘 뒤 마지막 결과를 받아 문단을 맺고 쓰기 스레드를 마친다.
        cx.spawn({
            let recognition = recognition.clone();
            async move |_| {
                while let Some((id, event)) = apple_events.next().await {
                    if recognition.borrow_mut().on_event(id, event) {
                        break;
                    }
                }
            }
        })
        .detach();
        Ok(Box::new(Session { recognition }))
    }
}

fn error(message: impl Into<String>) -> SpeechError {
    SpeechError::recognizer(std::io::Error::other(message.into()))
}

fn has_usage_description() -> bool {
    NSBundle::mainBundle()
        .objectForInfoDictionaryKey(&NSString::from_str(USAGE_DESCRIPTION_KEY))
        .is_some()
}

fn authorization() -> SFSpeechRecognizerAuthorizationStatus {
    // SAFETY: 읽기만 하고 묻지 않아 사용 설명 없이도 안전하다.
    unsafe { SFSpeechRecognizer::authorizationStatus() }
}

fn is_denied(status: SFSpeechRecognizerAuthorizationStatus) -> bool {
    status == SFSpeechRecognizerAuthorizationStatus::Denied
        || status == SFSpeechRecognizerAuthorizationStatus::Restricted
}

/// `locale`의 인식기. 그 언어의 인식기가 없으면 `None`.
fn recognizer(locale: &str) -> Option<Retained<SFSpeechRecognizer>> {
    let locale = NSLocale::initWithLocaleIdentifier(NSLocale::alloc(), &NSString::from_str(locale));
    // SAFETY: 살아 있는 `NSLocale`. 인식기가 없는 로캘이면 nil을 돌려준다.
    unsafe { SFSpeechRecognizer::initWithLocale(SFSpeechRecognizer::alloc(), &locale) }
}

/// 권한을 묻는다. 답은 [`AppleEvent::Authorized`]로 온다.
fn request_authorization(events: mpsc::UnboundedSender<(u64, AppleEvent)>) {
    let handler = RcBlock::new(move |status: SFSpeechRecognizerAuthorizationStatus| {
        let granted = status == SFSpeechRecognizerAuthorizationStatus::Authorized;
        let _ = events.unbounded_send((0, AppleEvent::Authorized(granted)));
    });
    // SAFETY: 사용 설명이 있을 때만 온다(`unavailable`). 블록은 채널로 보내기만 해 어느 큐에서 돌아도 된다.
    unsafe { SFSpeechRecognizer::requestAuthorization(&handler) };
}

/// 프레임워크가 알리는 일. 요청 번호와 함께 메인 스레드로 온다.
enum AppleEvent {
    Authorized(bool),
    Result {
        text: String,
        is_final: bool,
        /// 발화가 끝났다(결과에 메타데이터가 있다).
        ends_utterance: bool,
    },
    Error(String),
}

struct Session {
    recognition: Rc<RefCell<Recognition>>,
}

impl RecognitionSession for Session {
    fn push_audio(&mut self, samples: &[i16], _cx: &mut App) {
        self.recognition.borrow_mut().push_audio(samples);
    }

    fn finish(&mut self, _cx: &mut App) {
        self.recognition.borrow_mut().finish();
    }
}

/// 지금 소리를 넣는 인식 요청(문단 하나).
struct Request {
    id: u64,
    request: Retained<SFSpeechAudioBufferRecognitionRequest>,
    task: Retained<SFSpeechRecognitionTask>,
    /// 이 요청에 넣은 소리(표본 수).
    samples: usize,
}

/// 녹음 한 번의 상태(메인 스레드).
struct Recognition {
    recognizer: Retained<SFSpeechRecognizer>,
    format: Retained<AVAudioFormat>,
    events: mpsc::UnboundedSender<(u64, AppleEvent)>,
    worker: mpsc::UnboundedSender<WorkerEvent>,
    /// 끝날 때 쓰기 스레드를 마치러 가져간다.
    pipeline: Option<Pipeline>,
    current: Option<Request>,
    next_id: u64,
    /// 권한을 받아 요청을 열었는지. 그 전의 소리는 `pending`에 모은다.
    begun: bool,
    pending: Vec<i16>,
    /// 지금 문단의 글(마지막 부분 결과).
    hypothesis: String,
    finishing: bool,
    done: bool,
}

impl Recognition {
    fn new(
        recognizer: Retained<SFSpeechRecognizer>,
        pipeline: Pipeline,
        worker: mpsc::UnboundedSender<WorkerEvent>,
        events: mpsc::UnboundedSender<(u64, AppleEvent)>,
    ) -> Result<Self, SpeechError> {
        let format = AudioFormat::default();
        // SAFETY: 16비트 모노 PCM 형식(요청이 받는 형식이라 바꾸지 않고 넣는다).
        let format = unsafe {
            AVAudioFormat::initWithCommonFormat_sampleRate_channels_interleaved(
                AVAudioFormat::alloc(),
                AVAudioCommonFormat::PCMFormatInt16,
                format.sample_rate().into(),
                format.channels().into(),
                false,
            )
        }
        .ok_or_else(|| error("cannot create the audio format"))?;
        Ok(Self {
            recognizer,
            format,
            events,
            worker,
            pipeline: Some(pipeline),
            current: None,
            next_id: 1,
            begun: false,
            pending: Vec::new(),
            hypothesis: String::new(),
            finishing: false,
            done: false,
        })
    }

    /// 권한을 받았다: 첫 요청을 열고 모아 둔 소리를 넣는다.
    fn begin(&mut self) {
        self.begun = true;
        let _ = self.worker.unbounded_send(WorkerEvent::Ready);
        self.open();
        let pending = std::mem::take(&mut self.pending);
        self.append(&pending);
        if self.finishing {
            self.end_audio();
        }
    }

    /// 새 인식 요청(다음 문단)을 연다.
    fn open(&mut self) {
        let id = self.next_id;
        self.next_id += 1;
        // SAFETY: 새 요청의 초기화와 속성 설정.
        let request = unsafe {
            let request = SFSpeechAudioBufferRecognitionRequest::new();
            request.setShouldReportPartialResults(true);
            // 소리를 Apple 서버로 보내지 않는다.
            request.setRequiresOnDeviceRecognition(true);
            // 문장 부호는 macOS 13부터.
            if request.respondsToSelector(sel!(setAddsPunctuation:)) {
                request.setAddsPunctuation(true);
            }
            request
        };
        let events = self.events.clone();
        let handler = RcBlock::new(
            move |result: *mut SFSpeechRecognitionResult, error: *mut NSError| {
                // SAFETY: 프레임워크는 nil 또는 이 호출 동안 살아 있는 객체를 준다.
                let (result, error) = unsafe { (result.as_ref(), error.as_ref()) };
                if let Some(result) = result {
                    // SAFETY: 살아 있는 결과의 속성을 읽는다.
                    let event = unsafe {
                        AppleEvent::Result {
                            text: result.bestTranscription().formattedString().to_string(),
                            is_final: result.isFinal(),
                            ends_utterance: result.speechRecognitionMetadata().is_some(),
                        }
                    };
                    let _ = events.unbounded_send((id, event));
                }
                if let Some(error) = error {
                    let message = format!(
                        "{} ({} {})",
                        error.localizedDescription(),
                        error.domain(),
                        error.code()
                    );
                    let _ = events.unbounded_send((id, AppleEvent::Error(message)));
                }
            },
        );
        // SAFETY: 소리 버퍼 요청이고, 블록은 채널로 보내기만 해 어느 큐에서 돌아도 된다.
        let task = unsafe {
            self.recognizer
                .recognitionTaskWithRequest_resultHandler(&request, &handler)
        };
        self.current = Some(Request {
            id,
            request,
            task,
            samples: 0,
        });
    }

    fn push_audio(&mut self, samples: &[i16]) {
        if self.done || self.finishing {
            return;
        }
        if self.begun {
            self.append(samples);
        } else {
            self.pending.extend_from_slice(samples);
        }
    }

    /// 지금 요청과 쓰기 스레드에 같은 소리를 넣는다. 문단이 너무 길어지면 맺는다.
    fn append(&mut self, samples: &[i16]) {
        let Some(current) = &mut self.current else {
            return;
        };
        let Ok(frames) = u32::try_from(samples.len()) else {
            return;
        };
        if frames == 0 {
            return;
        }
        if let Some(pipeline) = &self.pipeline {
            pipeline.audio(samples);
        }
        // SAFETY: PCM 형식이라 `u32` 안의 크기면 만들어진다.
        let Some(buffer) = (unsafe {
            AVAudioPCMBuffer::initWithPCMFormat_frameCapacity(
                AVAudioPCMBuffer::alloc(),
                &self.format,
                frames,
            )
        }) else {
            return;
        };
        // SAFETY: 16비트 모노라 0번 채널에 `frames`개를 쓸 수 있고, `frames`는 용량 안이다.
        unsafe {
            let channel = (*buffer.int16ChannelData()).as_ptr();
            std::ptr::copy_nonoverlapping(samples.as_ptr(), channel, samples.len());
            buffer.setFrameLength(frames);
            current.request.appendAudioPCMBuffer(&buffer);
        }
        current.samples += samples.len();
        if current.samples >= MAX_PARAGRAPH {
            let text = std::mem::take(&mut self.hypothesis);
            self.cut(text);
        }
    }

    /// 지금 문단을 `text`로 맺고 그 요청을 닫는다. 멈추는 중이 아니면 다음 문단의 요청을 연다.
    fn cut(&mut self, text: String) {
        if let Some(current) = self.current.take() {
            // SAFETY: 살아 있는 요청·작업을 닫는다. 그 뒤의 결과는 번호가 달라 버린다.
            unsafe {
                current.request.endAudio();
                current.task.cancel();
            }
        }
        self.hypothesis.clear();
        let _ = self
            .worker
            .unbounded_send(WorkerEvent::Hypothesis(String::new()));
        let alive = self
            .pipeline
            .as_ref()
            .is_some_and(|pipeline| pipeline.cut(text));
        if !alive {
            // 쓰기 스레드가 이미 끝났다(쓰기 실패): 그 까닭을 알리고 끝낸다.
            self.complete();
        } else if !self.finishing {
            self.open();
        }
    }

    /// 멈춘다: 지금 요청의 소리를 닫고 마지막 결과를 기다린다.
    fn finish(&mut self) {
        if self.finishing {
            return;
        }
        self.finishing = true;
        if self.begun {
            self.end_audio();
        }
    }

    fn end_audio(&mut self) {
        match &self.current {
            // SAFETY: 살아 있는 요청의 소리를 닫는다.
            Some(current) => unsafe { current.request.endAudio() },
            None => self.complete(),
        }
    }

    /// 프레임워크의 일 하나를 처리한다. 녹음이 끝났으면 `true`.
    fn on_event(&mut self, id: u64, event: AppleEvent) -> bool {
        if self.done {
            return true;
        }
        match event {
            AppleEvent::Authorized(true) => self.begin(),
            AppleEvent::Authorized(false) => {
                let _ = self.worker.unbounded_send(WorkerEvent::Failed(
                    t!("dictation.error.apple_permission").into(),
                ));
                self.pipeline = None;
                self.done = true;
            }
            _ if self.current.as_ref().is_none_or(|current| current.id != id) => {}
            AppleEvent::Result {
                text,
                is_final,
                ends_utterance,
            } => {
                if is_final || ends_utterance {
                    self.cut(text);
                    if self.finishing {
                        self.complete();
                    }
                } else {
                    self.hypothesis = text.clone();
                    let _ = self.worker.unbounded_send(WorkerEvent::Hypothesis(text));
                }
            }
            AppleEvent::Error(message) => {
                // 말이 없어 끝난 요청(`kAFAssistantErrorDomain 1110`)도 여기로 온다: 받아 둔 글로 맺고 이어 간다.
                // 소리를 받기도 전에 실패하면(언어 자산이 없는 등) 되풀이하지 않고 끝낸다.
                let heard = self.current.as_ref().is_some_and(|c| c.samples > 0);
                let text = std::mem::take(&mut self.hypothesis);
                if !heard && !self.finishing && text.is_empty() {
                    eprintln!("macOS 받아쓰기가 시작하지 못했습니다: {message}");
                    self.finishing = true;
                    self.cut(String::new());
                    self.pipeline = None;
                    let _ = self.worker.unbounded_send(WorkerEvent::Failed(message));
                    self.done = true;
                    return true;
                }
                self.cut(text);
                if self.finishing {
                    self.complete();
                }
            }
        }
        self.done
    }

    /// 쓰기 스레드가 남은 문단을 다 쓰면 끝을 알린다(메인 스레드를 막지 않게 다른 스레드에서 기다린다).
    fn complete(&mut self) {
        if let Some(current) = self.current.take() {
            // SAFETY: 살아 있는 작업을 그친다.
            unsafe { current.task.cancel() };
        }
        self.done = true;
        let Some(pipeline) = self.pipeline.take() else {
            return;
        };
        let worker = self.worker.clone();
        std::thread::spawn(move || {
            let _ = worker.unbounded_send(match pipeline.finish() {
                Ok(()) => WorkerEvent::Finished,
                Err(reason) => WorkerEvent::Failed(reason),
            });
        });
    }
}
