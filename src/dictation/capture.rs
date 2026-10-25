//! 받아쓰기의 소리 입력: gpui-kit `speech`의 [`AudioInput`]을 cpal로 구현한다(ADR-0054).
//!
//! gpui-kit의 `Microphone`(`speech` 기능)은 cpal 0.15를 써서 rodio(cpal 0.18)와 함께 빌드할 수 없다.
//! 마이크(기본 입력 장치), 시스템 소리(기본 출력 장치의 루프백: macOS 14.6+ Core Audio 프로세스 탭,
//! Windows WASAPI 루프백), 또는 둘을 섞어 받는다. 장치마다 모노로 섞고 인식기가 바라는 형식(16 kHz)으로
//! 바꾼 뒤, 둘이면 표본끼리 더한다. 자동 음량 맞춤([`AutoGain`])을 켰으면 섞은 소리의 크기를 말소리에 맞춰
//! 키운다(파형도 키운 소리로 보인다).

use std::cell::Cell;
use std::f32::consts::TAU;
use std::io;
use std::rc::Rc;

use cpal::traits::{DeviceTrait as _, HostTrait as _, StreamTrait as _};
use cpal::{
    ErrorKind, FromSample, Sample as _, SampleFormat, SizedSample, StreamConfig,
    SupportedStreamConfig,
};
use futures::StreamExt as _;
use futures::channel::mpsc;
use gpui_kit::component::speech::{AudioFormat, AudioInput, AudioSink, SpeechError};
use gpui_kit::gpui::{App, Subscription};
use serde::{Deserialize, Serialize};

/// 받아 적을 소리.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AudioSource {
    /// 마이크(내 목소리).
    #[default]
    Microphone,
    /// 이 컴퓨터에서 나는 소리(Zoom 같은 앱의 상대방 목소리 포함).
    System,
    /// 마이크와 시스템 소리를 섞는다(회의 양쪽).
    Both,
}

impl AudioSource {
    /// 이 OS에서 고를 수 있는 것. 시스템 소리 녹음은 macOS·Windows만 된다.
    pub fn available() -> &'static [AudioSource] {
        if cfg!(any(target_os = "macos", target_os = "windows")) {
            &[
                AudioSource::Microphone,
                AudioSource::System,
                AudioSource::Both,
            ]
        } else {
            &[AudioSource::Microphone]
        }
    }

    fn uses_microphone(self) -> bool {
        self != AudioSource::System
    }

    fn uses_system(self) -> bool {
        self != AudioSource::Microphone
    }
}

/// 둘을 섞을 때 한쪽이 이만큼(16 kHz 표본, 300 ms) 앞서면 늦은 쪽은 소리가 없는 것으로 보고 내보낸다.
/// 시스템 소리 장치가 잠깐 멈춰도 마이크 소리가 막히지 않는다.
const MAX_LAG: usize = 16_000 * 300 / 1000;

/// 녹음을 시작할 때 `source`를 읽어 그 소리를 받는 입력. 창이 고른 것을 함께 들고 있다.
pub struct SourceInput {
    pub source: Rc<Cell<AudioSource>>,
}

/// 자동 음량 맞춤이 겨누는 말소리 크기(RMS, 약 -20 dBFS).
const TARGET_RMS: f32 = 0.1;

/// 이보다 조용한 구간(RMS, 약 -50 dBFS)은 잡음으로 보고 음량을 더 키우지 않는다.
const GATE_RMS: f32 = 0.003;

/// 가장 크게 키우는 배수(약 +24 dB). 줄이지는 않는다(큰 소리는 리미터가 막는다).
const MAX_GAIN: f32 = 16.0;

/// 음량을 고치는 단위(16 kHz의 10 ms).
const GAIN_BLOCK: usize = 160;

/// 자동 음량 맞춤: 말소리가 작으면 천천히 키우고, 너무 커지면 빨리 줄이며, 넘치는 표본은 부드럽게 눌러
/// 찢어지지 않게 한다. 쉬는 구간(잡음)에서는 더 키우지 않는다.
struct AutoGain {
    gain: f32,
    /// 다음 블록을 채우는 중인 표본.
    pending: Vec<i16>,
}

impl AutoGain {
    fn new() -> Self {
        Self {
            gain: 1.0,
            pending: Vec::with_capacity(GAIN_BLOCK),
        }
    }

    /// 표본을 받아 다 찬 블록만큼 음량을 맞춰 돌려준다(남은 것은 다음에 잇는다).
    fn process(&mut self, samples: &[i16]) -> Vec<i16> {
        self.pending.extend_from_slice(samples);
        let full = self.pending.len() / GAIN_BLOCK * GAIN_BLOCK;
        let mut output = Vec::with_capacity(full);
        for block in self.pending[..full].chunks(GAIN_BLOCK) {
            let rms = (block
                .iter()
                .map(|&s| (f32::from(s) / 32768.0).powi(2))
                .sum::<f32>()
                / block.len() as f32)
                .sqrt();
            let wanted = if rms < GATE_RMS {
                // 잡음은 키우지 않는다(줄이지도 않아 말이 다시 시작될 때 작게 들어오지 않는다).
                self.gain.min((TARGET_RMS / GATE_RMS).min(MAX_GAIN))
            } else {
                (TARGET_RMS / rms).clamp(1.0, MAX_GAIN)
            };
            // 줄일 때는 빨리(블록마다 절반쯤), 키울 때는 천천히(약 1초에 걸쳐) 다가간다.
            let rate = if wanted < self.gain { 0.5 } else { 0.01 };
            let next = self.gain + (wanted - self.gain) * rate;
            let step = (next - self.gain) / block.len() as f32;
            for (ix, &sample) in block.iter().enumerate() {
                let gain = self.gain + step * ix as f32;
                output.push(limit(f32::from(sample) / 32768.0 * gain));
            }
            self.gain = next;
        }
        self.pending.drain(..full);
        output
    }
}

/// 넘치는 표본을 부드럽게 누른다: 0.9까지는 그대로, 그 위는 1.0에 다가가게 굽힌다.
fn limit(x: f32) -> i16 {
    const KNEE: f32 = 0.9;
    let magnitude = x.abs();
    let y = if magnitude <= KNEE {
        magnitude
    } else {
        KNEE + (1.0 - KNEE) * ((magnitude - KNEE) / (1.0 - KNEE)).tanh()
    };
    (y.copysign(x) * f32::from(i16::MAX)) as i16
}

impl AudioInput for SourceInput {
    fn start(
        &self,
        format: AudioFormat,
        sink: AudioSink,
        cx: &mut App,
    ) -> Result<Subscription, SpeechError> {
        let source = self.source.get();
        let auto_gain = crate::config::Config::load().dictation.auto_gain;
        #[cfg(target_os = "macos")]
        if source.uses_microphone() && macos::is_microphone_denied() {
            return Err(SpeechError::PermissionDenied);
        }

        let host = cpal::default_host();
        let (tx, mut rx) = mpsc::unbounded();
        let mut streams = Vec::new();
        let mut rates = Vec::new();
        if source.uses_microphone() {
            let device = host
                .default_input_device()
                .ok_or(SpeechError::NoInputDevice)?;
            let config = device.default_input_config().map_err(input_error)?;
            rates.push(config.sample_rate());
            streams.push(open(&device, config, rates.len() - 1, tx.clone())?);
        }
        if source.uses_system() {
            // 출력 장치로 입력 스트림을 열면 cpal이 루프백으로 그 장치에 나가는 소리를 받는다.
            let device = host.default_output_device().ok_or_else(no_output_error)?;
            let config = device.default_output_config().map_err(system_error)?;
            rates.push(config.sample_rate());
            streams.push(
                open(&device, config, rates.len() - 1, tx.clone()).map_err(
                    |error| match error {
                        SpeechError::Input(error) => system_error(error),
                        // `open`은 장치가 사라졌을 때(DeviceNotAvailable)만 이 오류를 낸다.
                        SpeechError::NoInputDevice => no_output_error(),
                        other => other,
                    },
                )?,
            );
        }
        drop(tx);

        let task = cx.spawn(async move |cx| {
            let mut converters: Vec<_> = rates
                .iter()
                .map(|&rate| Converter::new(rate, format))
                .collect();
            let mut mixer = Mixer::new(converters.len());
            let mut gain = auto_gain.then(AutoGain::new);
            let mut mono: Vec<Vec<f32>> = vec![Vec::new(); converters.len()];
            while let Some(first) = rx.next().await {
                // 장치는 10 ms 남짓씩 보낸다. 쌓인 것을 한 번에 넘겨 녹음 상태가 묶음마다 한 번만 바뀌게 한다.
                let mut error = None;
                for (track, capture) in
                    std::iter::once(first).chain(std::iter::from_fn(|| rx.try_recv().ok()))
                {
                    match capture {
                        Capture::Samples(samples) => mono[track].extend_from_slice(&samples),
                        Capture::Error(message) => error = Some(message),
                    }
                }
                for (track, (converter, samples)) in
                    converters.iter_mut().zip(&mut mono).enumerate()
                {
                    mixer.push(track, &converter.convert(samples));
                    samples.clear();
                }
                let samples = mixer.take();
                let samples = match gain.as_mut() {
                    Some(gain) => gain.process(&samples),
                    None => samples,
                };
                cx.update(|cx| {
                    if !samples.is_empty() {
                        sink.push(samples, cx);
                    }
                    if let Some(message) = error.take() {
                        sink.error(SpeechError::input(io::Error::other(message)), cx);
                    }
                });
            }
        });

        Ok(Subscription::new(move || {
            drop(streams);
            drop(task);
        }))
    }
}

fn input_error(error: cpal::Error) -> SpeechError {
    match error.kind() {
        ErrorKind::DeviceNotAvailable => SpeechError::NoInputDevice,
        _ => SpeechError::input(io::Error::other(error.to_string())),
    }
}

/// 시스템 소리를 열 수 없을 때. 흔한 까닭(macOS는 버전·권한, Windows는 기본 출력 장치)을 함께 알린다.
fn system_error(error: impl std::fmt::Display) -> SpeechError {
    #[cfg(target_os = "windows")]
    let message = t!(
        "dictation.error.system_audio_windows",
        err = error.to_string()
    );
    #[cfg(not(target_os = "windows"))]
    let message = t!("dictation.error.system_audio", err = error.to_string());
    SpeechError::input(io::Error::other(message.into_owned()))
}

/// 시스템 소리를 받을 기본 출력 장치가 없을 때. 마이크 오류("마이크를 찾을 수 없습니다")로 보이면 안 된다.
fn no_output_error() -> SpeechError {
    SpeechError::input(io::Error::other(
        t!("dictation.error.system_no_output").into_owned(),
    ))
}

/// 이 스트림 오류가 녹음을 끝내야 하는 것인지. 한순간 끊김(`Xrun`)·기본 장치 바뀜(`DeviceChanged`, cpal이
/// 새 장치로 따라간다)·실시간 우선순위 거절(`RealtimeDenied`, 지연만 늘어난다)은 소리가 이어지므로 넘긴다.
fn fatal(error: &cpal::Error) -> bool {
    !matches!(
        error.kind(),
        ErrorKind::Xrun | ErrorKind::DeviceChanged | ErrorKind::RealtimeDenied
    )
}

enum Capture {
    Samples(Vec<f32>),
    Error(String),
}

/// `device`의 입력 스트림을 열어 재생한다. 소리는 `track` 번호를 붙여 `tx`로 보낸다.
fn open(
    device: &cpal::Device,
    supported: SupportedStreamConfig,
    track: usize,
    tx: mpsc::UnboundedSender<(usize, Capture)>,
) -> Result<cpal::Stream, SpeechError> {
    let sample_format = supported.sample_format();
    let config: StreamConfig = supported.into();
    let stream = match sample_format {
        SampleFormat::F32 => build_stream::<f32>(device, config, track, tx),
        SampleFormat::I16 => build_stream::<i16>(device, config, track, tx),
        SampleFormat::U16 => build_stream::<u16>(device, config, track, tx),
        SampleFormat::I32 => build_stream::<i32>(device, config, track, tx),
        other => Err(SpeechError::input(io::Error::other(format!(
            "unsupported sample format {other}"
        )))),
    }?;
    stream.play().map_err(input_error)?;
    Ok(stream)
}

/// 입력 스트림을 만든다. 오디오 스레드에서는 모노로 섞어 보내기만 한다.
fn build_stream<T>(
    device: &cpal::Device,
    config: StreamConfig,
    track: usize,
    tx: mpsc::UnboundedSender<(usize, Capture)>,
) -> Result<cpal::Stream, SpeechError>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let channels = usize::from(config.channels.max(1));
    let error_tx = tx.clone();
    device
        .build_input_stream(
            config,
            move |data: &[T], _| {
                let mono = data
                    .chunks(channels)
                    .map(|frame| {
                        frame
                            .iter()
                            .map(|&sample| f32::from_sample(sample))
                            .sum::<f32>()
                            / frame.len() as f32
                    })
                    .collect();
                let _ = tx.unbounded_send((track, Capture::Samples(mono)));
            },
            move |error| {
                if fatal(&error) {
                    let _ = error_tx.unbounded_send((track, Capture::Error(error.to_string())));
                }
            },
            None,
        )
        .map_err(input_error)
}

/// 여러 입력(같은 형식으로 바꾼 것)을 표본끼리 더해 하나로 만든다. 입력이 하나면 그대로 넘긴다.
struct Mixer {
    tracks: Vec<Vec<i16>>,
}

impl Mixer {
    fn new(tracks: usize) -> Self {
        Self {
            tracks: vec![Vec::new(); tracks],
        }
    }

    fn push(&mut self, track: usize, samples: &[i16]) {
        self.tracks[track].extend_from_slice(samples);
    }

    /// 모든 입력이 다 온 만큼(한쪽이 [`MAX_LAG`]보다 앞서면 그만큼 더) 섞어 꺼낸다.
    fn take(&mut self) -> Vec<i16> {
        if let [only] = self.tracks.as_mut_slice() {
            return std::mem::take(only);
        }
        let shortest = self.tracks.iter().map(Vec::len).min().unwrap_or(0);
        let longest = self.tracks.iter().map(Vec::len).max().unwrap_or(0);
        let n = shortest.max(longest.saturating_sub(MAX_LAG));
        let mixed = (0..n)
            .map(|ix| {
                let sum: i32 = self
                    .tracks
                    .iter()
                    .map(|track| i32::from(track.get(ix).copied().unwrap_or(0)))
                    .sum();
                sum.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
            })
            .collect();
        for track in &mut self.tracks {
            track.drain(..n.min(track.len()));
        }
        mixed
    }
}

/// 장치 표본율의 모노 `f32`를 인식기 형식의 `i16`으로 바꾼다(조각을 넘어 상태를 잇는다).
struct Converter {
    /// 출력 표본 하나에 해당하는 입력 표본 수.
    step: f64,
    /// 다음 출력 표본의 위치(입력 표본 단위, `0.0`은 앞 조각의 마지막 표본).
    position: f64,
    previous: f32,
    low_pass: Option<LowPass>,
    channels: usize,
}

impl Converter {
    fn new(source_rate: u32, format: AudioFormat) -> Self {
        let target_rate = format.sample_rate().max(1);
        let source_rate = source_rate.max(1);
        Self {
            step: f64::from(source_rate) / f64::from(target_rate),
            position: 1.0,
            previous: 0.0,
            // 낮출 때는 새 나이퀴스트 위의 소리가 말소리 대역으로 접혀 들어오지 않게 먼저 거른다.
            low_pass: (source_rate > target_rate)
                .then(|| LowPass::new(target_rate as f32 * 0.45, source_rate as f32)),
            channels: usize::from(format.channels().max(1)),
        }
    }

    fn convert(&mut self, input: &[f32]) -> Vec<i16> {
        if input.is_empty() {
            return Vec::new();
        }
        let filtered;
        let input = match self.low_pass.as_mut() {
            Some(low_pass) => {
                filtered = input
                    .iter()
                    .map(|&x| low_pass.process(x))
                    .collect::<Vec<_>>();
                &filtered[..]
            }
            None => input,
        };
        let previous = self.previous;
        let sample = |ix: usize| if ix == 0 { previous } else { input[ix - 1] };
        let last = input.len();
        let len = last as f64;
        let mut output = Vec::with_capacity(((len / self.step) as usize + 1) * self.channels);
        while self.position <= len {
            let ix = self.position.floor() as usize;
            let frac = (self.position - ix as f64) as f32;
            let value = sample(ix) * (1.0 - frac) + sample((ix + 1).min(last)) * frac;
            let value = (value.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16;
            output.extend(std::iter::repeat_n(value, self.channels));
            self.position += self.step;
        }
        self.position -= len;
        self.previous = input[last - 1];
        output
    }
}

/// 1차 저역 통과 필터 두 단(옥타브당 12 dB).
struct LowPass {
    alpha: f32,
    stages: [f32; 2],
}

impl LowPass {
    fn new(cutoff: f32, sample_rate: f32) -> Self {
        Self {
            alpha: 1.0 - (-TAU * cutoff / sample_rate).exp(),
            stages: [0.0; 2],
        }
    }

    fn process(&mut self, mut x: f32) -> f32 {
        for stage in &mut self.stages {
            *stage += self.alpha * (x - *stage);
            x = *stage;
        }
        x
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use objc2_av_foundation::{AVAuthorizationStatus, AVCaptureDevice, AVMediaTypeAudio};

    /// 사용자나 관리 정책이 마이크를 막았는지. 묻기 전(`NotDetermined`)이면 아니다(열 때 macOS가 묻는다).
    /// 시스템 소리 녹음 권한은 읽는 공개 API가 없다(막혀 있으면 무음이 온다).
    pub(super) fn is_microphone_denied() -> bool {
        // SAFETY: 프레임워크의 변하지 않는 상수를 읽는다.
        let Some(audio) = (unsafe { AVMediaTypeAudio }) else {
            return false;
        };
        // SAFETY: `audio`는 이 메서드가 받는 미디어 종류다. 상태를 읽기만 하고 묻지 않는다.
        let status = unsafe { AVCaptureDevice::authorizationStatusForMediaType(audio) };
        status == AVAuthorizationStatus::Denied || status == AVAuthorizationStatus::Restricted
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AudioFormat, AutoGain, Converter, ErrorKind, GAIN_BLOCK, MAX_GAIN, MAX_LAG, Mixer,
        TARGET_RMS, fatal,
    };

    fn rms(samples: &[i16]) -> f32 {
        (samples
            .iter()
            .map(|&s| (f32::from(s) / 32768.0).powi(2))
            .sum::<f32>()
            / samples.len() as f32)
            .sqrt()
    }

    fn tone(amplitude: f32, len: usize) -> Vec<i16> {
        (0..len)
            .map(|ix| ((ix as f32 * 0.2).sin() * amplitude * 32767.0) as i16)
            .collect()
    }

    #[test]
    fn auto_gain_raises_quiet_speech_toward_target() {
        let mut gain = AutoGain::new();
        // 약 -37 dBFS의 작은 말소리 3초.
        let quiet = tone(0.02, 16_000 * 3);
        let output: Vec<i16> = quiet
            .chunks(173)
            .flat_map(|chunk| gain.process(chunk))
            .collect();
        assert_eq!(output.len(), quiet.len() / GAIN_BLOCK * GAIN_BLOCK);
        let last = rms(&output[output.len() - 1_600..]);
        assert!(last > TARGET_RMS * 0.8 && last < TARGET_RMS * 1.2, "{last}");
    }

    #[test]
    fn auto_gain_does_not_boost_noise_or_clip_loud_sound() {
        let mut gain = AutoGain::new();
        let noise = tone(0.001, 16_000 * 3);
        let output = gain.process(&noise);
        assert!(gain.gain < 1.5, "{}", gain.gain);
        assert!(rms(&output) < 0.002);
        // 크게 키운 뒤 갑자기 큰 소리가 와도 넘치지 않는다.
        gain.gain = MAX_GAIN;
        let loud = gain.process(&tone(0.9, 1_600));
        assert!(loud.iter().all(|&s| s > i16::MIN));
        assert!(gain.gain < 2.0, "{}", gain.gain);
    }

    #[test]
    fn stream_glitches_do_not_end_the_recording() {
        for kind in [
            ErrorKind::Xrun,
            ErrorKind::DeviceChanged,
            ErrorKind::RealtimeDenied,
        ] {
            assert!(!fatal(&cpal::Error::new(kind)), "{kind:?}");
        }
        for kind in [
            ErrorKind::DeviceNotAvailable,
            ErrorKind::StreamInvalidated,
            ErrorKind::BackendError,
        ] {
            assert!(fatal(&cpal::Error::new(kind)), "{kind:?}");
        }
    }

    #[test]
    fn mixer_passes_single_track() {
        let mut mixer = Mixer::new(1);
        mixer.push(0, &[1, 2, 3]);
        assert_eq!(mixer.take(), vec![1, 2, 3]);
        assert!(mixer.take().is_empty());
    }

    #[test]
    fn mixer_sums_aligned_samples_and_keeps_the_rest() {
        let mut mixer = Mixer::new(2);
        mixer.push(0, &[100, 200, 300]);
        mixer.push(1, &[10, i16::MAX]);
        assert_eq!(mixer.take(), vec![110, i16::MAX]);
        mixer.push(1, &[30]);
        assert_eq!(mixer.take(), vec![330]);
    }

    #[test]
    fn mixer_does_not_wait_forever_for_a_silent_track() {
        let mut mixer = Mixer::new(2);
        mixer.push(0, &vec![1; MAX_LAG + 10]);
        let mixed = mixer.take();
        assert_eq!(mixed.len(), 10);
        assert!(mixed.iter().all(|&s| s == 1));
    }

    #[test]
    fn downsamples_across_chunks() {
        let mut converter = Converter::new(48_000, AudioFormat::default());
        let total: usize = (0..10).map(|_| converter.convert(&[0.0; 480]).len()).sum();
        // 48 kHz의 100 ms는 어떻게 잘라 넣어도 16 kHz의 100 ms다.
        assert_eq!(total, 1_600);
    }

    #[test]
    fn keeps_rate() {
        let mut converter = Converter::new(16_000, AudioFormat::default());
        assert_eq!(converter.convert(&[0.0, 0.5, -0.5]), vec![0, 16383, -16383]);
    }
}
