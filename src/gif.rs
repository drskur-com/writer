//! 움직이는 GIF를 프레임 하나씩 푼다(ADR-0049).
//!
//! 모든 프레임을 펼쳐 두면 프레임 수 × 픽셀만큼 메모리를 쓴다(500×500, 100프레임이면 약 100MB). 그래서 원본은
//! 압축된 채 들고, 보일 때마다 다음 프레임만 푼다. 끝에 닿으면 처음부터 다시 푼다.
//!
//! - 프레임 수는 헤더에 없어서 파일을 훑어 센다([`scan`]). 그림 데이터는 풀지 않고 블록 길이만 따라 건너뛴다.
//! - 반복 횟수는 NETSCAPE2.0 블록을 따른다. 없으면 한 번만 재생한다(브라우저와 같다).
//! - 프레임 합성(일부 사각형만 바뀌는 프레임, 지우기 방식)은 `image` 크레이트가 한다.
//! - 프레임마다의 표시 시간도 훑을 때 모은다. 그래서 시간 → 프레임 번호는 풀지 않고 구한다([`frame_at`]).

use std::io::Cursor;
use std::sync::Arc;
use std::time::Duration;

use image::AnimationDecoder as _;
use image::codecs::gif::GifDecoder;

/// 이보다 짧은 지연(0·1·10ms로 적힌 GIF가 많다)은 [`SHORT_DELAY_FALLBACK`]으로 고친다(브라우저와 같다).
const SHORT_DELAY: Duration = Duration::from_millis(10);
const SHORT_DELAY_FALLBACK: Duration = Duration::from_millis(100);

/// GIF 원본. 에디터와 확장이 읽은 파일을 그대로 함께 쓴다.
pub type Data = Arc<Vec<u8>>;

/// `Cursor`가 읽는 원본(`Arc<Vec<u8>>`는 `AsRef<[u8]>`가 아니다).
struct Shared(Data);

impl AsRef<[u8]> for Shared {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

/// 몇 번 재생하는지.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Repeat {
    /// NETSCAPE2.0 블록이 없다.
    Once,
    /// 첫 재생 뒤 이만큼 더 돈다.
    Times(u16),
    Forever,
}

impl Repeat {
    /// `plays`번 다 돈 뒤에 한 번 더 도는지.
    fn allows(self, plays: u32) -> bool {
        match self {
            Repeat::Once => plays == 0,
            Repeat::Times(more) => plays <= u32::from(more),
            Repeat::Forever => true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Info {
    /// 프레임마다의 표시 시간([`fix_delay`]를 거친 값). 길이가 프레임 수다.
    pub delays: Vec<Duration>,
    pub repeat: Repeat,
}

impl Info {
    pub fn frames(&self) -> usize {
        self.delays.len()
    }

    /// 한 번 재생하는 시간.
    pub fn duration(&self) -> Duration {
        self.delays.iter().sum()
    }
}

/// GIF 파일을 훑어 프레임 수·표시 시간과 반복 횟수를 구한다. GIF가 아니면 `None`. 뒤가 잘린 파일은 거기까지 센다.
pub fn scan(data: &[u8]) -> Option<Info> {
    if !(data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a")) {
        return None;
    }
    let packed = *data.get(10)?;
    let mut pos = 13 + color_table(packed);
    let mut delays = Vec::new();
    // 그래픽 제어 블록의 지연은 바로 다음 그림에 붙는다(1/100초).
    let mut pending = Duration::ZERO;
    let mut repeat = Repeat::Once;
    // 블록 하나를 읽고 다음 자리를 낸다. 끝(0x3B)이거나 잘렸으면 `None`.
    let mut block = |pos: usize| -> Option<usize> {
        match *data.get(pos)? {
            // 확장 블록: 이름표 다음에 하위 블록들.
            0x21 => {
                let label = *data.get(pos + 1)?;
                let pos = pos + 2;
                if label == 0xF9
                    && let Some(&[4, _, lo, hi]) = data.get(pos..pos + 4)
                {
                    pending = Duration::from_millis(u64::from(u16::from_le_bytes([lo, hi])) * 10);
                }
                if label == 0xFF
                    && data.get(pos..pos + 12) == Some(b"\x0bNETSCAPE2.0")
                    && let Some(&[3, 1, lo, hi]) = data.get(pos + 12..pos + 16)
                {
                    let count = u16::from_le_bytes([lo, hi]);
                    repeat = if count == 0 {
                        Repeat::Forever
                    } else {
                        Repeat::Times(count)
                    };
                }
                skip_sub_blocks(data, pos)
            }
            // 그림: 설명 10바이트, 지역 색표, LZW 최소 코드 크기 1바이트, 하위 블록들. 끝까지 있어야 센다.
            0x2C => {
                let packed = *data.get(pos + 9)?;
                let next = skip_sub_blocks(data, pos + 10 + color_table(packed) + 1)?;
                delays.push(fix_delay(std::mem::take(&mut pending)));
                Some(next)
            }
            _ => None,
        }
    };
    while let Some(next) = block(pos) {
        pos = next;
    }
    Some(Info { delays, repeat })
}

/// 색표가 있으면 그 바이트 수.
fn color_table(packed: u8) -> usize {
    if packed & 0x80 == 0 {
        0
    } else {
        3 << ((packed & 0x07) + 1)
    }
}

/// 하위 블록들(길이 바이트 + 내용, 길이 0이 끝)을 건너뛴 자리. 잘렸으면 `None`.
fn skip_sub_blocks(data: &[u8], mut pos: usize) -> Option<usize> {
    loop {
        let size = usize::from(*data.get(pos)?);
        pos += 1;
        if size == 0 {
            return Some(pos);
        }
        pos += size;
    }
}

/// 프레임의 표시 시간.
pub fn delay(frame: &image::Frame) -> Duration {
    fix_delay(Duration::from(frame.delay()))
}

/// 적힌 지연을 보일 시간으로. 너무 짧은 값은 고친다.
pub fn fix_delay(duration: Duration) -> Duration {
    if duration <= SHORT_DELAY {
        SHORT_DELAY_FALLBACK
    } else {
        duration
    }
}

/// 재생을 시작한 지 `t`초에 보일 프레임 번호(0부터). 반복을 다 돌았으면 마지막 프레임이다.
pub fn frame_at(info: &Info, t: f64) -> usize {
    let last = info.frames().saturating_sub(1);
    let total = info.duration().as_secs_f64();
    if last == 0 || total <= 0.0 || !t.is_finite() || t <= 0.0 {
        return 0;
    }
    let plays = (t / total).floor();
    let done = match info.repeat {
        Repeat::Once => plays >= 1.0,
        Repeat::Times(more) => plays > f64::from(more),
        Repeat::Forever => false,
    };
    if done {
        return last;
    }
    let mut left = t - plays * total;
    for (ix, delay) in info.delays.iter().enumerate() {
        left -= delay.as_secs_f64();
        if left < 0.0 {
            return ix;
        }
    }
    last
}

/// 반복 횟수를 지키며 합성된 프레임(RGBA, GIF 전체 크기)을 차례로 낸다. 다 돌면 끝난다.
pub struct Frames {
    data: Data,
    repeat: Repeat,
    plays: u32,
    current: Option<image::Frames<'static>>,
    /// 이번 재생에서 프레임을 하나라도 냈는지. 못 냈으면 다시 돌지 않는다(깨진 파일에서 헛돌지 않게).
    yielded: bool,
}

impl Frames {
    pub fn new(data: Data, repeat: Repeat) -> Self {
        Self {
            data,
            repeat,
            plays: 0,
            current: None,
            yielded: false,
        }
    }
}

impl Iterator for Frames {
    type Item = image::Frame;

    fn next(&mut self) -> Option<image::Frame> {
        loop {
            if self.current.is_none() {
                if !self.repeat.allows(self.plays) {
                    return None;
                }
                let decoder = GifDecoder::new(Cursor::new(Shared(self.data.clone()))).ok()?;
                self.current = Some(decoder.into_frames());
                self.yielded = false;
            }
            match self.current.as_mut()?.next() {
                Some(Ok(frame)) => {
                    self.yielded = true;
                    return Some(frame);
                }
                // 끝이거나 뒤가 깨졌다: 이번 재생을 마친다.
                _ => {
                    self.current = None;
                    self.plays += 1;
                    if !self.yielded {
                        return None;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::codecs::gif::{GifEncoder, Repeat as EncoderRepeat};
    use image::{Delay, Frame, Rgba, RgbaImage};

    /// 색이 다른 `count`프레임짜리 4×4 GIF.
    fn gif(count: u8, repeat: Option<EncoderRepeat>, delay_ms: u32) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = GifEncoder::new(&mut out);
            if let Some(repeat) = repeat {
                encoder.set_repeat(repeat).unwrap();
            }
            for ix in 0..count {
                let image = RgbaImage::from_pixel(4, 4, Rgba([ix * 40, 0, 0, 255]));
                let delay = Delay::from_numer_denom_ms(delay_ms, 1);
                encoder
                    .encode_frame(Frame::from_parts(image, 0, 0, delay))
                    .unwrap();
            }
        }
        out
    }

    #[test]
    fn scan_counts_frames_and_reads_repeat() {
        let forever = gif(3, Some(EncoderRepeat::Infinite), 50);
        assert_eq!(
            scan(&forever),
            Some(Info {
                delays: vec![Duration::from_millis(50); 3],
                repeat: Repeat::Forever
            })
        );
        let twice = gif(2, Some(EncoderRepeat::Finite(2)), 50);
        assert_eq!(scan(&twice).unwrap().repeat, Repeat::Times(2));
        let once = gif(1, None, 0);
        assert_eq!(
            scan(&once),
            Some(Info {
                delays: vec![SHORT_DELAY_FALLBACK],
                repeat: Repeat::Once
            })
        );
        assert_eq!(scan(b"\x89PNG\r\n"), None);
        // 뒤가 잘리면 다 읽은 프레임까지만 센다.
        assert_eq!(scan(&forever[..forever.len() - 20]).unwrap().frames(), 2);
    }

    #[test]
    fn frames_follow_repeat_count() {
        let data: Data = Arc::new(gif(3, Some(EncoderRepeat::Finite(1)), 50));
        let repeat = scan(&data).unwrap().repeat;
        let reds: Vec<u8> = Frames::new(data, repeat)
            .map(|frame| frame.buffer().get_pixel(0, 0).0[0])
            .collect();
        // 한 번 재생 + 한 번 더.
        assert_eq!(reds, [0, 40, 80, 0, 40, 80]);

        let data: Data = Arc::new(gif(2, None, 50));
        assert_eq!(Frames::new(data, Repeat::Once).count(), 2);

        let data: Data = Arc::new(gif(2, Some(EncoderRepeat::Infinite), 50));
        assert_eq!(Frames::new(data, Repeat::Forever).take(7).count(), 7);

        // 깨진 파일은 헛돌지 않는다.
        let broken: Data = Arc::new(b"GIF89a broken".to_vec());
        assert_eq!(Frames::new(broken, Repeat::Forever).count(), 0);
    }

    #[test]
    fn frame_at_follows_delays_and_repeat() {
        let info = |repeat| Info {
            delays: [100, 200, 300].map(Duration::from_millis).to_vec(),
            repeat,
        };
        let forever = info(Repeat::Forever);
        let at = |info: &Info, t: f64| frame_at(info, t);
        assert_eq!(at(&forever, -1.0), 0);
        assert_eq!(at(&forever, 0.0), 0);
        assert_eq!(at(&forever, 0.099), 0);
        assert_eq!(at(&forever, 0.1), 1);
        assert_eq!(at(&forever, 0.35), 2);
        assert_eq!(at(&forever, 0.65), 0);
        assert_eq!(at(&forever, f64::NAN), 0);
        // 한 번이면 끝난 뒤 마지막 프레임.
        assert_eq!(at(&info(Repeat::Once), 0.65), 2);
        // 한 번 더 돌고 멈춘다.
        let twice = info(Repeat::Times(1));
        assert_eq!(at(&twice, 0.65), 0);
        assert_eq!(at(&twice, 1.25), 2);
    }

    #[test]
    fn short_delays_are_slowed_down() {
        let data: Data = Arc::new(gif(1, None, 0));
        let frame = Frames::new(data, Repeat::Once).next().unwrap();
        assert_eq!(delay(&frame), SHORT_DELAY_FALLBACK);
        let data: Data = Arc::new(gif(1, None, 70));
        let frame = Frames::new(data, Repeat::Once).next().unwrap();
        assert_eq!(delay(&frame), Duration::from_millis(70));
    }
}
