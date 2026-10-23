//! 확장의 모양 목록을 GPUI로 그린다(ADR-0026, ADR-0042).
//!
//! - 모양은 vello_cpu로 기기 픽셀 크기의 이미지에 그리고([`extension::render`]) 그 이미지를 올린다. 그래서
//!   화면, 어시스턴트가 보는 PNG, DOCX가 같은 그림이다.
//! - 캔버스마다 마지막 이미지를 (모양, 픽셀 크기)로 기억한다. 같으면(스크롤, 입력, 정지 그림) 다시 그리지 않는다.
//! - 바꾸거나 캔버스가 사라진 이미지는 그 창의 이미지 아틀라스에서 지운다. 지우려면 창이 있어야 하므로, 다음에
//!   그 창에서 그릴 때 지운다([`STALE`]).

use std::cell::RefCell;
use std::sync::Arc;
use std::time::Instant;

use gpui_kit::gpui::*;

use crate::extension::{self, Shape, render::Renderer};

/// 그림 하나의 처리 시간이 이보다 길면 알린다(개발 빌드에서만).
const SLOW_RENDER_MS: f64 = 8.0;

/// 캔버스(에디터 블록, 대화상자 미리보기, 책 미리보기, 플레이 창)마다 하나: 마지막으로 그린 이미지.
#[derive(Default)]
pub(super) struct RasterCache {
    /// (모양 열쇠, 픽셀 폭, 픽셀 높이), 그 이미지와 그린 창.
    shown: Option<((u64, u32, u32), Arc<RenderImage>, WindowId)>,
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
    /// 렌더 문맥. UI 스레드에서 모든 캔버스가 같이 쓴다.
    static RENDERER: RefCell<Renderer> = RefCell::new(Renderer::new());
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
    let image = match &cache.shown {
        Some((shown, image, _)) if *shown == key => image.clone(),
        _ => {
            let started = Instant::now();
            let pixels = RENDERER.with(|renderer| {
                let mut renderer = renderer.borrow_mut();
                renderer.render(shapes, scale * density * fit, width, height, None)
            });
            let data = extension::render::to_bgra(pixels);
            let Some(buffer) = image::RgbaImage::from_raw(width, height, data) else {
                return;
            };
            let image = Arc::new(RenderImage::new([image::Frame::new(buffer)]));
            if cfg!(debug_assertions) {
                let ms = started.elapsed().as_secs_f64() * 1000.0;
                if ms > SLOW_RENDER_MS {
                    eprintln!(
                        "확장 그림 {width}×{height}px, 모양 {}개: {ms:.1}ms",
                        shapes.len()
                    );
                }
            }
            if let Some((_, old, old_window)) = cache.shown.replace((key, image.clone(), window_id))
            {
                if old_window == window_id {
                    window.drop_image(old).ok();
                } else {
                    STALE.with(|stale| stale.borrow_mut().push((old_window, old)));
                }
            }
            image
        }
    };
    window
        .paint_image(bounds, bounds, Corners::default(), image, 0, false)
        .ok();
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
