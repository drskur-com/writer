//! 받아쓰기(ADR-0054): 음성 인식 모델 내려받기, Nemotron 인식기(`recognizer`), 받아 적을 소리(`capture`).
//! 화면은 메인 창의 받아쓰기 그룹이다(`app::dictation`).
//!
//! 모델은 handy-computer가 transcribe.cpp용으로 바꾼 NVIDIA Nemotron 3.5 ASR Streaming 0.6B GGUF다.
//! Hugging Face 캐시(`HF_HUB_CACHE` → `$HF_HOME/hub` → `~/.cache/huggingface/hub`)에 내려받아
//! `hf` CLI·Handy 같은 다른 앱과 같은 파일을 함께 쓴다. 이미 캐시에 있으면 내려받지 않는다.
//!
//! 내려받기는 앱 전역 [`ModelStore`]가 맡아 설정 창을 닫아도 이어진다.

mod capture;
mod output;
mod recognizer;

pub use capture::{AudioSource, SourceInput};
pub use output::TranslationSetup;
pub use recognizer::{
    Event as RecordEvent, NemotronRecognizer, create_record, ensure_record_title,
};

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use futures::StreamExt as _;
use futures::channel::mpsc;
use gpui_kit::gpui::*;
use hf_hub::progress::{DownloadEvent as HubEvent, Progress, ProgressEvent, ProgressHandler};

/// 모델 저장소(`handy-computer/nemotron-3.5-asr-streaming-0.6b-gguf`).
const REPO_OWNER: &str = "handy-computer";
const REPO_NAME: &str = "nemotron-3.5-asr-streaming-0.6b-gguf";

/// 내려받는 파일. 양자화는 고르게 하지 않고 정확도가 원본(F32)과 같은 Q8_0 하나만 쓴다.
pub const MODEL_FILE: &str = "nemotron-3.5-asr-streaming-0.6b-Q8_0.gguf";

/// 모델 파일 크기(바이트). 내려받기 전에 보이고, 받는 중 전체 크기를 모를 때 쓴다.
pub const MODEL_SIZE: u64 = 751_094_240;

/// 모델이 받아쓰는 언어 수(한국어 포함).
pub const MODEL_LANGUAGES: usize = 32;

/// Hugging Face 캐시 폴더.
pub fn cache_dir() -> PathBuf {
    hf_hub::resolve_cache_dir()
}

/// 캐시 안의 모델 저장소 폴더.
fn repo_dir() -> PathBuf {
    cache_dir().join(format!("models--{REPO_OWNER}--{REPO_NAME}"))
}

/// 캐시의 어느 스냅샷에든 `file`이 있으면 그 경로. 예전 커밋으로 받은 것도 쓴다.
/// 받다 만 파일은 스냅샷에 링크되지 않아(`blobs/*.incomplete`) 걸리지 않는다.
pub fn cached(file: &str) -> Option<PathBuf> {
    std::fs::read_dir(repo_dir().join("snapshots"))
        .ok()?
        .flatten()
        .map(|entry| entry.path().join(file))
        .find(|path| path.is_file())
}

/// Hugging Face에 로그인했는지(`hf auth login`의 토큰 파일 또는 `HF_TOKEN`). 로그인하면 더 빨리 받는다.
pub fn hf_logged_in() -> bool {
    if std::env::var("HF_TOKEN").is_ok_and(|token| !token.trim().is_empty()) {
        return true;
    }
    let path = std::env::var_os("HF_TOKEN_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| hf_hub::hf_home().join("token"));
    std::fs::read_to_string(path).is_ok_and(|token| !token.trim().is_empty())
}

/// 내려받기 소식.
enum Event {
    Progress { done: u64, total: u64 },
    Finished(Result<PathBuf, String>),
}

/// tokio 런타임. 처음 내려받을 때 만든다.
fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("writer-hf")
            .enable_all()
            .build()
            .expect("tokio 런타임을 만들 수 없습니다")
    })
}

/// 버리면 내려받기를 멈춘다.
struct AbortOnDrop(tokio::task::AbortHandle);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// 모델을 캐시에 내려받는다. 마지막 소식은 언제나 `Finished`다(멈추면 오지 않는다).
fn download() -> (mpsc::UnboundedReceiver<Event>, AbortOnDrop) {
    let (tx, rx) = mpsc::unbounded();
    let handle = runtime().spawn(async move {
        let reporter = Reporter {
            tx: tx.clone(),
            file: MODEL_FILE,
            size: MODEL_SIZE,
            last_permille: AtomicU64::new(u64::MAX),
        };
        let result = async {
            let client = hf_hub::HFClient::builder()
                .user_agent(concat!("writer/", env!("CARGO_PKG_VERSION")))
                .build()?;
            client
                .model(REPO_OWNER, REPO_NAME)
                .download_file()
                .filename(MODEL_FILE)
                .progress(Progress::new(reporter))
                .send()
                .await
        }
        .await
        .map_err(|err| err.to_string());
        let _ = tx.unbounded_send(Event::Finished(result));
    });
    (rx, AbortOnDrop(handle.abort_handle()))
}

/// hf-hub의 진행 소식을 받은 바이트로 바꿔 보낸다. 화면이 넘치지 않게 0.1%가 바뀔 때만 보낸다.
struct Reporter {
    tx: mpsc::UnboundedSender<Event>,
    file: &'static str,
    size: u64,
    last_permille: AtomicU64,
}

impl Reporter {
    fn send(&self, done: u64, total: u64) {
        let total = if total == 0 { self.size } else { total };
        let permille = done.saturating_mul(1000) / total.max(1);
        if self.last_permille.swap(permille, Ordering::Relaxed) != permille {
            let _ = self.tx.unbounded_send(Event::Progress { done, total });
        }
    }
}

impl ProgressHandler for Reporter {
    fn on_progress(&self, event: &ProgressEvent) {
        let ProgressEvent::Download(event) = event else {
            return;
        };
        match event {
            HubEvent::Start { total_bytes, .. } => self.send(0, *total_bytes),
            // Xet 저장소는 묶음 전체의 바이트만 알린다(파일 하나만 받으니 같은 뜻이다).
            HubEvent::AggregateProgress {
                bytes_completed,
                total_bytes,
                ..
            } => self.send(*bytes_completed, *total_bytes),
            HubEvent::Progress { files } => {
                for file in files.iter().filter(|file| file.filename == self.file) {
                    self.send(file.bytes_completed, file.total_bytes);
                }
            }
            HubEvent::Complete => {}
        }
    }
}

/// 받는 중인 모델.
pub struct ActiveDownload {
    pub done: u64,
    pub total: u64,
    _abort: AbortOnDrop,
    _task: Task<()>,
}

/// 모델 캐시 상태와 내려받기(앱 전역).
pub struct ModelStore {
    /// 캐시에 있는 모델 파일.
    downloaded: Option<PathBuf>,
    active: Option<ActiveDownload>,
    /// 마지막 내려받기가 실패한 이유.
    failed: Option<SharedString>,
}

struct GlobalModelStore(Entity<ModelStore>);

impl Global for GlobalModelStore {}

impl ModelStore {
    /// 앱 전역 저장소. 처음 부르면 캐시를 살펴 만든다.
    pub fn global(cx: &mut App) -> Entity<Self> {
        if let Some(GlobalModelStore(store)) = cx.try_global::<GlobalModelStore>() {
            return store.clone();
        }
        let store = cx.new(|_| Self {
            downloaded: cached(MODEL_FILE),
            active: None,
            failed: None,
        });
        cx.set_global(GlobalModelStore(store.clone()));
        store
    }

    /// 캐시를 다시 살핀다(`hf` CLI 등 밖에서 받거나 지웠을 수 있다).
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let downloaded = cached(MODEL_FILE);
        if downloaded != self.downloaded {
            self.downloaded = downloaded;
            cx.notify();
        }
    }

    /// 캐시에 있는 모델 파일.
    pub fn downloaded(&self) -> Option<&PathBuf> {
        self.downloaded.as_ref()
    }

    pub fn active(&self) -> Option<&ActiveDownload> {
        self.active.as_ref()
    }

    pub fn failure(&self) -> Option<&SharedString> {
        self.failed.as_ref()
    }

    /// 내려받기 시작한다. 이미 받는 중이면 아무것도 하지 않는다.
    pub fn start(&mut self, cx: &mut Context<Self>) {
        if self.active.is_some() {
            return;
        }
        let (mut events, abort) = download();
        let task = cx.spawn(async move |this, cx| {
            while let Some(event) = events.next().await {
                let finished = matches!(event, Event::Finished(_));
                let updated = this.update(cx, |this, cx| this.on_event(event, cx));
                if finished || updated.is_err() {
                    break;
                }
            }
        });
        self.failed = None;
        self.active = Some(ActiveDownload {
            done: 0,
            total: MODEL_SIZE,
            _abort: abort,
            _task: task,
        });
        cx.notify();
    }

    /// 받던 것을 멈춘다. 다시 받으면 처음부터 받는다.
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        if self.active.take().is_some() {
            cx.notify();
        }
    }

    fn on_event(&mut self, event: Event, cx: &mut Context<Self>) {
        let Some(active) = self.active.as_mut() else {
            return;
        };
        match event {
            Event::Progress { done, total } => {
                active.done = done;
                active.total = total;
            }
            Event::Finished(result) => {
                self.active = None;
                match result {
                    Ok(path) => self.downloaded = Some(path),
                    Err(reason) => self.failed = Some(reason.into()),
                }
            }
        }
        cx.notify();
    }
}
