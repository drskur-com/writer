//! 받아쓰기의 소리 입력: gpui-kit `speech`의 [`AudioInput`]을 cpal로 구현한다(ADR-0054).
//!
//! gpui-kit의 `Microphone`(`speech` 기능)은 cpal 0.15를 써서 rodio(cpal 0.18)와 함께 빌드할 수 없다.
//! 마이크(기본 입력 장치), 시스템 소리(기본 출력 장치의 루프백: macOS 14.6+ Core Audio 프로세스 탭,
//! Windows WASAPI 루프백), 또는 둘을 섞어 받는다. 장치마다 모노로 섞고 인식기가 바라는 형식(16 kHz)으로
//! 바꾼 뒤, 둘이면 표본끼리 더한다.

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

impl AudioInput for SourceInput {
    fn start(
        &self,
        format: AudioFormat,
        sink: AudioSink,
        cx: &mut App,
    ) -> Result<Subscription, SpeechError> {
        let source = self.source.get();
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
            let device = host
                .default_output_device()
                .ok_or(SpeechError::NoInputDevice)?;
            let config = device.default_output_config().map_err(system_error)?;
            rates.push(config.sample_rate());
            streams.push(
                open(&device, config, rates.len() - 1, tx.clone()).map_err(
                    |error| match error {
                        SpeechError::Input(error) => system_error(error),
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

/// 시스템 소리를 열 수 없을 때. 흔한 까닭(macOS 버전·권한)을 함께 알린다.
fn system_error(error: impl std::fmt::Display) -> SpeechError {
    SpeechError::input(io::Error::other(
        t!("dictation.error.system_audio", err = error.to_string()).into_owned(),
    ))
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
                // 기본 장치가 바뀌면 cpal이 알아서 따라간다. 스트림은 그대로라 실패가 아니다.
                if error.kind() != ErrorKind::DeviceChanged {
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
    use super::{AudioFormat, Converter, MAX_LAG, Mixer};

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
