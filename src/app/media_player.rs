//! 에디터의 동영상·오디오 재생기(ADR-0050).
//!
//! - 오디오: rodio가 파일을 풀어(symphonia) 앱의 미디어 출력 장치로 낸다. 모든 OS에서 된다.
//! - 동영상(macOS): AVPlayer가 풀고 소리를 내며, `AVPlayerItemVideoOutput`이 프레임(`CVPixelBuffer`, 4:2:0
//!   YCbCr)을 준다. 그 프레임을 gpui `paint_surface`로 그대로 그린다(복사·변환 없음). 다른 OS는 포스터만 보인다.
//! - 재생기는 처음 재생 단추를 누를 때 만든다. 보기만 하는 시트는 파일을 열지 않는다.
//! - 재생 중에는 창을 다시 그려 시간·프레임을 바꾼다. 동영상은 화면 프레임마다, 오디오는 [`AUDIO_TICK`]마다.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use gpui_kit::gpui::*;

use crate::media::MediaKind;

/// 오디오 재생 중 시간 표시를 바꾸는 간격.
const AUDIO_TICK: Duration = Duration::from_millis(250);

/// 미디어 출력 장치의 믹서. 장치(cpal 스트림)는 스레드를 넘지 못해 따로 띄운 스레드가 끝까지 들고 있다.
/// 장치가 없으면 `None`(소리 없이 재생하지 않는다).
fn mixer() -> Option<&'static rodio::mixer::Mixer> {
    static MIXER: OnceLock<Option<rodio::mixer::Mixer>> = OnceLock::new();
    MIXER
        .get_or_init(|| {
            let (sender, receiver) = std::sync::mpsc::channel();
            std::thread::Builder::new()
                .name("media-audio".into())
                .spawn(
                    move || match rodio::DeviceSinkBuilder::open_default_sink() {
                        Ok(mut device) => {
                            device.log_on_drop(false);
                            sender.send(Some(device.mixer().clone())).ok();
                            loop {
                                std::thread::park();
                            }
                        }
                        Err(err) => {
                            eprintln!("media: no audio output device: {err}");
                            sender.send(None).ok();
                        }
                    },
                )
                .ok()?;
            receiver.recv().ok().flatten()
        })
        .as_ref()
}

/// 줄 하나의 재생기.
pub(super) struct MediaPlayer {
    path: PathBuf,
    /// 헤더에서 읽은 길이(재생기를 만들기 전에도 보인다).
    known_duration: Option<Duration>,
    backend: Option<Backend>,
    /// 재생기를 만들 수 없었다(파일을 풀 수 없음, 출력 장치 없음, macOS 밖의 동영상).
    failed: bool,
    /// 오디오 시간 표시를 바꾸는 타이머.
    tick: Option<Task<()>>,
}

enum Backend {
    Audio(AudioBackend),
    #[cfg(target_os = "macos")]
    Video(video::VideoBackend),
}

impl MediaPlayer {
    pub(super) fn new(path: PathBuf, duration: Option<Duration>) -> Self {
        Self {
            path,
            known_duration: duration,
            backend: None,
            failed: false,
            tick: None,
        }
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    pub(super) fn failed(&self) -> bool {
        self.failed
    }

    /// 재생 중인지.
    pub(super) fn playing(&self) -> bool {
        match &self.backend {
            Some(Backend::Audio(audio)) => audio.playing(),
            #[cfg(target_os = "macos")]
            Some(Backend::Video(video)) => video.playing(),
            None => false,
        }
    }

    /// 재생을 시작했던 적이 있는지(동영상은 그때부터 포스터 대신 프레임을 그린다).
    pub(super) fn started(&self) -> bool {
        self.backend.is_some()
    }

    pub(super) fn position(&self) -> Duration {
        match &self.backend {
            Some(Backend::Audio(audio)) => audio.position(),
            #[cfg(target_os = "macos")]
            Some(Backend::Video(video)) => video.position(),
            None => Duration::ZERO,
        }
    }

    pub(super) fn duration(&self) -> Option<Duration> {
        let live = match &self.backend {
            #[cfg(target_os = "macos")]
            Some(Backend::Video(video)) => video.duration(),
            _ => None,
        };
        live.or(self.known_duration)
    }

    /// 재생·일시 정지. 처음 누르면 재생기를 만든다. 끝까지 재생했으면 처음부터 다시 한다.
    pub(super) fn toggle(&mut self, kind: MediaKind) {
        if self.playing() {
            self.pause();
            return;
        }
        if !self.ensure_backend(kind) {
            return;
        }
        #[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
        let duration = self.duration();
        match &mut self.backend {
            Some(Backend::Audio(audio)) => audio.play(),
            #[cfg(target_os = "macos")]
            Some(Backend::Video(video)) => video.play(duration),
            None => {}
        }
    }

    pub(super) fn pause(&mut self) {
        match &self.backend {
            Some(Backend::Audio(audio)) => audio.pause(),
            #[cfg(target_os = "macos")]
            Some(Backend::Video(video)) => video.pause(),
            None => {}
        }
    }

    /// 길이의 `fraction`(0~1) 자리로 옮긴다. 재생기가 없으면 만든다(멈춘 채로).
    pub(super) fn seek(&mut self, kind: MediaKind, fraction: f32) {
        let Some(duration) = self.duration() else {
            return;
        };
        if !self.ensure_backend(kind) {
            return;
        }
        let at = duration.mul_f32(fraction.clamp(0., 1.));
        match &mut self.backend {
            Some(Backend::Audio(audio)) => audio.seek(at),
            #[cfg(target_os = "macos")]
            Some(Backend::Video(video)) => video.seek(at),
            None => {}
        }
    }

    fn ensure_backend(&mut self, kind: MediaKind) -> bool {
        if self.backend.is_some() {
            return true;
        }
        if self.failed {
            return false;
        }
        self.backend = match kind {
            MediaKind::Audio => AudioBackend::open(&self.path).map(Backend::Audio),
            #[cfg(target_os = "macos")]
            MediaKind::Video => video::VideoBackend::open(&self.path).map(Backend::Video),
            #[cfg(not(target_os = "macos"))]
            MediaKind::Video => None,
        };
        self.failed = self.backend.is_none();
        !self.failed
    }

    /// 동영상의 지금 프레임을 `bounds`에 그린다. 그렸으면 `true`(아니면 포스터를 그대로 둔다).
    pub(super) fn paint_video(&mut self, bounds: Bounds<Pixels>, window: &mut Window) -> bool {
        match &mut self.backend {
            #[cfg(target_os = "macos")]
            Some(Backend::Video(video)) => video.paint(bounds, window),
            _ => {
                let _ = (bounds, window);
                false
            }
        }
    }

    /// 재생 중이면 창을 다시 그리게 한다(시간·진행 막대). 동영상은 `paint_video`가 화면 프레임마다 한다.
    pub(super) fn schedule_refresh(&mut self, window: &Window, cx: &App) {
        if !matches!(self.backend, Some(Backend::Audio(_))) || !self.playing() {
            self.tick = None;
            return;
        }
        // 그릴 때마다 새로 건다(지난 타이머는 버린다). 그사이 다른 일로 다시 그려도 간격이 같다.
        self.tick = Some(window.spawn(cx, async move |cx| {
            cx.background_executor().timer(AUDIO_TICK).await;
            cx.update(|window, _| window.refresh()).ok();
        }));
    }
}

/// rodio 재생기. 끝까지 재생하면 비고, 다음 재생에서 파일을 다시 연다.
struct AudioBackend {
    path: PathBuf,
    player: rodio::Player,
    /// 비어 있는 재생기에 들어갈 다음 재생을 위해 멈춘 자리.
    paused_at: Option<Duration>,
}

impl AudioBackend {
    fn open(path: &Path) -> Option<Self> {
        let player = rodio::Player::connect_new(mixer()?);
        player.pause();
        let backend = Self {
            path: path.to_path_buf(),
            player,
            paused_at: None,
        };
        backend.load().then_some(backend)
    }

    /// 파일을 다시 열어 재생 목록에 넣는다.
    fn load(&self) -> bool {
        let decoder = std::fs::File::open(&self.path)
            .ok()
            .and_then(|file| rodio::Decoder::try_from(file).ok());
        match decoder {
            Some(decoder) => {
                self.player.append(decoder);
                true
            }
            None => false,
        }
    }

    fn playing(&self) -> bool {
        !self.player.is_paused() && !self.player.empty()
    }

    fn position(&self) -> Duration {
        if self.player.empty() {
            self.paused_at.unwrap_or_default()
        } else {
            self.player.get_pos()
        }
    }

    fn play(&mut self) {
        if self.player.empty() {
            if !self.load() {
                return;
            }
            if let Some(at) = self.paused_at.take() {
                self.player.try_seek(at).ok();
            }
        }
        self.player.play();
    }

    fn pause(&self) {
        self.player.pause();
    }

    fn seek(&mut self, at: Duration) {
        if self.player.empty() {
            self.paused_at = Some(at);
            return;
        }
        self.player.try_seek(at).ok();
    }
}

#[cfg(target_os = "macos")]
mod video {
    use std::path::Path;
    use std::time::{Duration, Instant};

    use gpui_kit::gpui::{Bounds, Pixels, Window};
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2::{AllocAnyThread as _, MainThreadMarker};
    use objc2_av_foundation::{
        AVPlayer, AVPlayerItem, AVPlayerItemStatus, AVPlayerItemVideoOutput,
    };
    use objc2_core_foundation::{CFRetained, CFString};
    use objc2_core_media::CMTime;
    use objc2_core_video::{
        CVPixelBuffer, kCVPixelBufferIOSurfacePropertiesKey, kCVPixelBufferMetalCompatibilityKey,
        kCVPixelBufferPixelFormatTypeKey, kCVPixelFormatType_420YpCbCr8BiPlanarFullRange,
    };
    use objc2_foundation::{NSDictionary, NSNumber, NSString, NSURL};

    /// 옮긴 뒤 새 프레임을 기다리는 시간. 멈춘 채로 옮겨도 그 자리 프레임이 보이게 한다.
    const SEEK_FRAME_WAIT: Duration = Duration::from_secs(1);

    /// 시간 단위(CMTime timescale).
    const TIMESCALE: i32 = 600;

    pub(super) struct VideoBackend {
        player: Retained<AVPlayer>,
        item: Retained<AVPlayerItem>,
        output: Retained<AVPlayerItemVideoOutput>,
        frame: Option<CFRetained<CVPixelBuffer>>,
        /// 이때까지 새 프레임을 기다린다(옮긴 직후).
        wait_frame_until: Option<Instant>,
    }

    fn key(key: &CFString) -> &NSString {
        // CFString과 NSString은 같은 객체다(toll-free bridging).
        unsafe { &*(key as *const CFString).cast::<NSString>() }
    }

    impl VideoBackend {
        pub(super) fn open(path: &Path) -> Option<Self> {
            let mtm = MainThreadMarker::new()?;
            let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
            // gpui `paint_surface`는 4:2:0 YCbCr(전체 범위)이고 Metal 텍스처로 쓸 수 있는 프레임만 그린다.
            let format = NSNumber::new_u32(kCVPixelFormatType_420YpCbCr8BiPlanarFullRange);
            let metal = NSNumber::new_bool(true);
            let surface = NSDictionary::<NSString, AnyObject>::new();
            let attributes = unsafe {
                NSDictionary::<NSString, AnyObject>::from_slices(
                    &[
                        key(kCVPixelBufferPixelFormatTypeKey),
                        key(kCVPixelBufferMetalCompatibilityKey),
                        key(kCVPixelBufferIOSurfacePropertiesKey),
                    ],
                    &[
                        format.as_ref() as &AnyObject,
                        metal.as_ref() as &AnyObject,
                        surface.as_ref() as &AnyObject,
                    ],
                )
            };
            unsafe {
                let output = AVPlayerItemVideoOutput::initWithPixelBufferAttributes(
                    AVPlayerItemVideoOutput::alloc(),
                    Some(&attributes),
                );
                let item = AVPlayerItem::playerItemWithURL(&url, mtm);
                item.addOutput(&output);
                let player = AVPlayer::playerWithPlayerItem(Some(&item), mtm);
                Some(Self {
                    player,
                    item,
                    output,
                    frame: None,
                    wait_frame_until: Some(Instant::now() + SEEK_FRAME_WAIT),
                })
            }
        }

        pub(super) fn playing(&self) -> bool {
            unsafe { self.player.rate() != 0.0 }
        }

        pub(super) fn position(&self) -> Duration {
            seconds(unsafe { self.item.currentTime() }).unwrap_or_default()
        }

        pub(super) fn duration(&self) -> Option<Duration> {
            if unsafe { self.item.status() } != AVPlayerItemStatus::ReadyToPlay {
                return None;
            }
            seconds(unsafe { self.item.duration() })
        }

        pub(super) fn play(&mut self, duration: Option<Duration>) {
            // 끝에 멈춰 있으면 처음부터.
            if let Some(duration) = duration
                && self.position() + Duration::from_millis(100) >= duration
            {
                self.seek(Duration::ZERO);
            }
            unsafe { self.player.play() };
        }

        pub(super) fn pause(&self) {
            unsafe { self.player.pause() };
        }

        pub(super) fn seek(&mut self, at: Duration) {
            unsafe {
                let zero = CMTime::with_seconds(0., TIMESCALE);
                self.player.seekToTime_toleranceBefore_toleranceAfter(
                    CMTime::with_seconds(at.as_secs_f64(), TIMESCALE),
                    zero,
                    zero,
                );
            }
            self.wait_frame_until = Some(Instant::now() + SEEK_FRAME_WAIT);
        }

        /// 새 프레임이 있으면 받아 두고 받아 둔 프레임을 그린다. 재생 중이거나 옮긴 직후면 다음 화면 프레임에
        /// 다시 그린다.
        pub(super) fn paint(&mut self, bounds: Bounds<Pixels>, window: &mut Window) -> bool {
            unsafe {
                let now = self.item.currentTime();
                if self.output.hasNewPixelBufferForItemTime(now)
                    && let Some(frame) = self
                        .output
                        .copyPixelBufferForItemTime_itemTimeForDisplay(now, std::ptr::null_mut())
                {
                    // objc2-av-foundation과 gpui가 같은 objc2-core-video 판을 쓴다.
                    self.frame = Some(CFRetained::from(&*frame));
                    self.wait_frame_until = None;
                }
            }
            let waiting = self
                .wait_frame_until
                .is_some_and(|until| Instant::now() < until);
            if self.playing() || waiting {
                window.request_animation_frame();
            }
            match &self.frame {
                Some(frame) => {
                    window.paint_surface(bounds, frame.clone());
                    true
                }
                None => false,
            }
        }
    }

    /// CMTime → 길이. 모르거나 무한이면 `None`.
    fn seconds(time: CMTime) -> Option<Duration> {
        let seconds = unsafe { time.seconds() };
        (seconds.is_finite() && seconds >= 0.).then(|| Duration::from_secs_f64(seconds))
    }
}
