//! 확장의 모양 목록을 GPUI로 그린다(ADR-0026, ADR-0042, ADR-0046).
//!
//! - 모양은 vello_cpu로 기기 픽셀 크기의 이미지에 그리고([`extension::render`]) 그 이미지를 올린다. 그래서
//!   화면, 어시스턴트가 보는 PNG, DOCX가 같은 그림이다.
//! - 그리기는 렌더 스레드들이 한다([`submit`]). UI는 기다리지 않고 지난 이미지를 보이다가, 새 이미지가 오면 다음
//!   프레임에 바꾼다. 캔버스마다 한 번에 하나만 맡기고, 끝나면 그때의 최신 모양을 맡긴다(느리면 프레임을 건너뛴다).
//! - 캔버스마다 마지막 이미지를 (모양, 픽셀 크기)로 기억한다. 같으면(스크롤, 입력, 정지 그림) 다시 그리지 않는다.
//! - 바꾸거나 캔버스가 사라진 이미지는 그 창의 이미지 아틀라스에서 지운다. 지우려면 창이 있어야 하므로, 다음에
//!   그 창에서 그릴 때 지운다([`STALE`]).

use std::cell::RefCell;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use gpui_kit::gpui::*;

use crate::extension::{self, Shape, render::Renderer};

/// 그림 하나의 처리 시간이 이보다 길면 알린다(개발 빌드에서만).
const SLOW_RENDER_MS: f64 = 8.0;

/// 렌더 스레드 수 상한.
const MAX_THREADS: usize = 4;

/// (모양 열쇠, 픽셀 폭, 픽셀 높이).
type Key = (u64, u32, u32);

/// 캔버스(에디터 블록, 대화상자 미리보기, 책 미리보기, 플레이 창)마다 하나: 마지막으로 그린 이미지와 맡긴 일.
#[derive(Default)]
pub(super) struct RasterCache {
    /// 보이는 이미지, 그 열쇠와 그린 창.
    shown: Option<(Key, Arc<RenderImage>, WindowId)>,
    /// 렌더 스레드에 맡긴 일(열쇠, 결과를 받을 곳).
    pending: Option<(Key, Receiver<Option<RenderImage>>)>,
}

impl Drop for RasterCache {
    fn drop(&mut self) {
        if let Some((_, image, window)) = self.shown.take() {
            STALE.with(|stale| stale.borrow_mut().push((window, image)));
        }
    }
}

thread_local! {
    /// 아틀라스에서 지울 이미지와 그 창.
    static STALE: RefCell<Vec<(WindowId, Arc<RenderImage>)>> = const { RefCell::new(Vec::new()) };
}

/// 모양 목록을 `bounds`(창 좌표)에 그린다. 모양 좌표에 `scale`을 곱하면 `bounds` 안의 자리다.
pub(super) fn paint(
    shapes: &[Shape],
    bounds: Bounds<Pixels>,
    scale: f32,
    cache: &RefCell<RasterCache>,
    window: &mut Window,
    cx: &mut App,
) {
    let window_id = window.window_handle().window_id();
    drop_stale(window_id, window, cx);
    let density = window.scale_factor();
    let width = (f32::from(bounds.size.width) * density).ceil() as u32;
    let height = (f32::from(bounds.size.height) * density).ceil() as u32;
    if width == 0 || height == 0 {
        return;
    }
    // 아주 큰 캔버스는 해상도를 낮춰 그리고 늘려 보인다.
    let fit = (extension::render::MAX_PIXELS as f32 / width.max(height) as f32).min(1.0);
    let (width, height) = (
        ((width as f32 * fit) as u32).max(1),
        ((height as f32 * fit) as u32).max(1),
    );
    let key = (extension::render::shapes_key(shapes), width, height);
    let mut cache = cache.borrow_mut();
    // 맡긴 일이 끝났으면 그 이미지로 바꾼다.
    if let Some((pending_key, reply)) = &cache.pending {
        match reply.try_recv() {
            Ok(image) => {
                let pending_key = *pending_key;
                cache.pending = None;
                if let Some(image) = image {
                    let replaced = cache
                        .shown
                        .replace((pending_key, Arc::new(image), window_id));
                    if let Some((_, old, old_window)) = replaced {
                        if old_window == window_id {
                            window.drop_image(old).ok();
                        } else {
                            STALE.with(|stale| stale.borrow_mut().push((old_window, old)));
                        }
                    }
                }
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => cache.pending = None,
        }
    }
    let current = cache.shown.as_ref().is_some_and(|(shown, ..)| *shown == key);
    if !current && cache.pending.is_none() {
        let reply = submit(shapes.to_vec(), scale * density * fit, width, height);
        cache.pending = Some((key, reply));
    }
    // 기다리는 동안은 프레임마다 다시 와서 받았는지 본다.
    if !current {
        window.request_animation_frame();
    }
    if let Some((_, image, _)) = &cache.shown {
        window
            .paint_image(bounds, bounds, Corners::default(), image.clone(), 0, false)
            .ok();
    }
}

/// 렌더 스레드에 맡길 일.
struct Job {
    shapes: Vec<Shape>,
    scale: f32,
    width: u32,
    height: u32,
    reply: Sender<Option<RenderImage>>,
}

/// 그리기를 렌더 스레드에 맡긴다. 결과(BGRA 이미지)는 돌려준 곳으로 온다.
fn submit(shapes: Vec<Shape>, scale: f32, width: u32, height: u32) -> Receiver<Option<RenderImage>> {
    static QUEUE: OnceLock<Sender<Job>> = OnceLock::new();
    let queue = QUEUE.get_or_init(|| {
        let (tx, rx) = channel::<Job>();
        let rx = Arc::new(Mutex::new(rx));
        let threads = std::thread::available_parallelism()
            .map_or(1, |n| n.get().saturating_sub(1))
            .clamp(1, MAX_THREADS);
        for ix in 0..threads {
            let rx = rx.clone();
            std::thread::Builder::new()
                .name(format!("extension-render-{ix}"))
                .spawn(move || {
                    let mut renderer = Renderer::new();
                    loop {
                        let job = match rx.lock() {
                            Ok(rx) => rx.recv(),
                            Err(_) => return,
                        };
                        let Ok(job) = job else {
                            return;
                        };
                        job.reply.send(render(&mut renderer, &job)).ok();
                    }
                })
                .ok();
        }
        tx
    });
    let (reply, result) = channel();
    queue
        .send(Job {
            shapes,
            scale,
            width,
            height,
            reply,
        })
        .ok();
    result
}

fn render(renderer: &mut Renderer, job: &Job) -> Option<RenderImage> {
    let started = Instant::now();
    let pixels = renderer.render(&job.shapes, job.scale, job.width, job.height, None);
    let data = extension::render::to_bgra(pixels);
    let buffer = image::RgbaImage::from_raw(job.width, job.height, data)?;
    if cfg!(debug_assertions) {
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        if ms > SLOW_RENDER_MS {
            eprintln!(
                "확장 그림 {}×{}px, 모양 {}개: {ms:.1}ms",
                job.width,
                job.height,
                job.shapes.len()
            );
        }
    }
    Some(RenderImage::new([image::Frame::new(buffer)]))
}

/// 이 창의 지울 이미지를 지우고, 닫힌 창의 것은 버린다(아틀라스도 함께 사라졌다).
fn drop_stale(window_id: WindowId, window: &mut Window, cx: &App) {
    let stale = STALE.with(|stale| std::mem::take(&mut *stale.borrow_mut()));
    if stale.is_empty() {
        return;
    }
    let open: Vec<WindowId> = cx.windows().iter().map(|w| w.window_id()).collect();
    let mut keep = Vec::new();
    for (id, image) in stale {
        if id == window_id {
            window.drop_image(image).ok();
        } else if open.contains(&id) {
            keep.push((id, image));
        }
    }
    STALE.with(|stale| stale.borrow_mut().extend(keep));
}
