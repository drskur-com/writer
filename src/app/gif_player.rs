//! 에디터의 움직이는 GIF 재생(ADR-0049).
//!
//! - 디코딩 스레드가 GIF마다 하나 있고 ([`crate::gif::Frames`]), 다음 프레임 하나만 미리 풀어 표시 크기로 줄여
//!   둔다. 메모리는 원본 + 보이는 프레임 + 다음 프레임이다.
//! - 프레임은 보일 때(그려질 때)만 넘긴다. 다음 프레임 차례에 창을 다시 그리게 타이머를 건다. 화면 밖이거나 창이
//!   비활성이면 그리지 않으므로 멈춘다. 움직임 줄이기 설정이면 첫 프레임만 보인다.
//! - 한동안 그리지 않으면([`IDLE`]) 스레드와 프레임을 버린다. 다시 보이면 처음부터 푼다.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, TryRecvError, channel};
use std::time::{Duration, Instant};

use gpui_kit::gpui::*;

use super::extension_painter;
use crate::gif::{self, Repeat};

/// 처음 보일 때 첫 프레임을 기다리는 시간. 그 안에 오면 빈 칸이 한 프레임 보이지 않는다.
const FIRST_FRAME_WAIT: Duration = Duration::from_millis(30);

/// 이만큼 그리지 않은 재생기는 스레드와 프레임을 버린다.
const IDLE: Duration = Duration::from_secs(5);

/// 프레임 차례가 이보다 늦었으면(창이 비활성이었다 등) 밀린 시간을 버리고 지금부터 센다.
const MAX_LAG: Duration = Duration::from_millis(250);

/// 푼 프레임(BGRA, 표시 크기)과 표시 시간.
struct Decoded {
    image: RenderImage,
    delay: Duration,
}

/// 디코딩 스레드와 주고받는 곳. 버리면 스레드가 끝난다.
struct Decoder {
    /// 긴 변(기기 px).
    side: u32,
    request: Sender<()>,
    /// `None`이면 다 돌았다.
    reply: Receiver<Option<Decoded>>,
}

pub(super) struct GifPlayer {
    data: gif::Data,
    repeat: Repeat,
    decoder: Option<Decoder>,
    /// 보이는 프레임과 그린 창.
    shown: Option<(Arc<RenderImage>, WindowId)>,
    /// 받아 둔 다음 프레임.
    ready: Option<Decoded>,
    /// 보이는 프레임이 끝나는 때.
    due: Option<Instant>,
    /// 다 돌아 마지막 프레임에 멈췄다.
    ended: bool,
    /// 다음 차례에 창을 다시 그리는 타이머와 그 때.
    timer: Option<(Instant, Task<()>)>,
    last_paint: Instant,
}

impl GifPlayer {
    pub(super) fn new(data: gif::Data, repeat: Repeat) -> Self {
        Self {
            data,
            repeat,
            decoder: None,
            shown: None,
            ready: None,
            due: None,
            ended: false,
            timer: None,
            last_paint: Instant::now(),
        }
    }

    /// 한동안 그리지 않았으면 스레드와 프레임을 버린다.
    pub(super) fn sleep_if_idle(&mut self, now: Instant) {
        if self.decoder.is_some() && now.duration_since(self.last_paint) > IDLE {
            self.decoder = None;
            self.ready = None;
            self.due = None;
            self.ended = false;
            self.timer = None;
            if let Some((image, window)) = self.shown.take() {
                extension_painter::retire(window, image);
            }
        }
    }

    /// 지금 프레임을 `bounds`에 그린다. `side`는 표시 크기의 긴 변(논리 px)이다.
    pub(super) fn paint(
        &mut self,
        bounds: Bounds<Pixels>,
        side: Pixels,
        corners: Corners<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let window_id = window.window_handle().window_id();
        extension_painter::drop_stale(window_id, window, cx);
        let now = Instant::now();
        self.last_paint = now;
        let side = (f32::from(side) * window.scale_factor()).ceil().max(1.0) as u32;
        // 화면 배율이 바뀌면(다른 화면으로 옮김) 그 크기로 처음부터 푼다. 그동안은 보이던 프레임을 늘려 보인다.
        if self
            .decoder
            .as_ref()
            .is_none_or(|decoder| decoder.side != side)
        {
            self.decoder = Some(spawn(self.data.clone(), self.repeat, side));
            self.ready = None;
            self.due = None;
            self.ended = false;
        }
        let first = self.shown.is_none();
        self.receive(first);

        let animate = !cx.reduce_motion() && window.is_window_active();
        let due = self.due.is_none_or(|due| due <= now);
        if self.ready.is_some() && (first || (animate && due)) {
            self.advance(now, window_id, window);
        }

        if animate && !self.ended {
            match (self.ready.is_some(), self.due) {
                // 다음 프레임이 와 있다: 차례에 다시 그린다.
                (true, Some(due)) if due > now => self.wake_at(due, window, cx),
                // 차례가 됐는데 아직 풀고 있다: 다음 화면 프레임에 다시 본다.
                _ => window.request_animation_frame(),
            }
        }

        if let Some((image, _)) = &self.shown {
            window
                .paint_image(bounds, bounds, corners, image.clone(), 0, false)
                .ok();
        }
    }

    /// 디코딩 스레드가 보낸 프레임을 받아 둔다. `wait`이면 잠깐 기다린다.
    fn receive(&mut self, wait: bool) {
        if self.ready.is_some() || self.ended {
            return;
        }
        let Some(decoder) = &self.decoder else {
            return;
        };
        let reply = if wait {
            decoder
                .reply
                .recv_timeout(FIRST_FRAME_WAIT)
                .map_err(|err| matches!(err, RecvTimeoutError::Disconnected))
        } else {
            decoder
                .reply
                .try_recv()
                .map_err(|err| matches!(err, TryRecvError::Disconnected))
        };
        match reply {
            Ok(Some(decoded)) => self.ready = Some(decoded),
            Ok(None) | Err(true) => self.ended = true,
            Err(false) => {}
        }
    }

    /// 받아 둔 프레임을 보이고 그다음 프레임을 부탁한다.
    fn advance(&mut self, now: Instant, window_id: WindowId, window: &mut Window) {
        let Some(decoded) = self.ready.take() else {
            return;
        };
        let start = match self.due {
            Some(due) if now.duration_since(due) <= MAX_LAG => due,
            _ => now,
        };
        self.due = Some(start + decoded.delay);
        let replaced = self.shown.replace((Arc::new(decoded.image), window_id));
        if let Some((old, old_window)) = replaced {
            if old_window == window_id {
                window.drop_image(old).ok();
            } else {
                extension_painter::retire(old_window, old);
            }
        }
        if let Some(decoder) = &self.decoder {
            decoder.request.send(()).ok();
        }
    }

    /// `at`에 창을 다시 그린다. 그때까지 걸린 타이머가 있으면 그대로 둔다.
    fn wake_at(&mut self, at: Instant, window: &Window, cx: &App) {
        if self
            .timer
            .as_ref()
            .is_some_and(|(when, _)| *when > Instant::now() && *when <= at)
        {
            return;
        }
        let wait = at.saturating_duration_since(Instant::now());
        let task = window.spawn(cx, async move |cx| {
            cx.background_executor().timer(wait).await;
            cx.update(|window, _| window.refresh()).ok();
        });
        self.timer = Some((at, task));
    }
}

impl Drop for GifPlayer {
    fn drop(&mut self) {
        if let Some((image, window)) = self.shown.take() {
            extension_painter::retire(window, image);
        }
    }
}

/// 디코딩 스레드를 띄운다. 첫 프레임은 바로 풀고, 그다음부터는 부탁받을 때마다 하나씩 푼다.
fn spawn(data: gif::Data, repeat: Repeat, side: u32) -> Decoder {
    let (request, requests) = channel::<()>();
    let (replies, reply) = channel();
    std::thread::Builder::new()
        .name("gif-decode".into())
        .spawn(move || {
            let mut frames = gif::Frames::new(data, repeat);
            loop {
                let decoded = frames.next().and_then(|frame| to_decoded(frame, side));
                let done = decoded.is_none();
                if replies.send(decoded).is_err() || done || requests.recv().is_err() {
                    return;
                }
            }
        })
        .ok();
    Decoder {
        side,
        request,
        reply,
    }
}

/// 합성된 프레임을 긴 변 `side` 안으로 줄이고 BGRA로 바꾼다.
fn to_decoded(frame: image::Frame, side: u32) -> Option<Decoded> {
    let delay = gif::delay(&frame);
    let mut buffer = frame.into_buffer();
    let (width, height) = buffer.dimensions();
    if width.max(height) > side {
        let fit = side as f32 / width.max(height) as f32;
        let w = ((width as f32 * fit).round() as u32).max(1);
        let h = ((height as f32 * fit).round() as u32).max(1);
        buffer = image::imageops::resize(&buffer, w, h, image::imageops::FilterType::Triangle);
    }
    // gpui는 BGRA를 받는다.
    for pixel in buffer.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    Some(Decoded {
        image: RenderImage::new([image::Frame::new(buffer)]),
        delay,
    })
}
