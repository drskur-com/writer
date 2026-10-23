//! 에디터의 확장 블록을 캔버스로 그린다(ADR-0026).
//!
//! `` ```ext <이름> `` 펜스는 그대로 고칠 수 있는 글자로 두고, 닫는 줄 아래에 gpui-base 줄 블록으로
//! 확장의 `draw`가 그린 모양을 그린다. 펜스 줄은 커서가 블록에 닿을 때만 보인다(표와 같다, ADR-0017).
//! 캔버스는 본문 폭보다 크면 비율을 지켜 줄이고 가운데에 둔다. 확장을 읽거나 그리다 실패하면 블록 자리에
//! 까닭을 보인다.
//!
//! `draw`는 확장의 작업 스레드에서 돈다([`extension::Worker`]). 그릴 때는 요청만 보내고 마지막으로 받은
//! 결과를 그리므로, 느린 확장은 그 블록만 늦게 바뀌고 편집은 막히지 않는다([`Player`]).
//! - 애니메이션은 초당 [`FPS`]번만 다시 그리고, 창이 비활성이거나 움직임 줄이기 설정이면 멈춘다.
//! - `period`가 있는 애니메이션은 한 주기의 프레임을 캐시해, 한 바퀴 돈 뒤에는 Lua를 부르지 않는다.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::gpui::*;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use super::extension_painter::{TessCache, paint};
use super::style::Palette;
use crate::extension::{self, Host, Loaded, Shape};

/// 캔버스 위·아래 여백(이미지 블록과 같다).
const PADDING_TOP: f32 = 4.0;
const PADDING_BOTTOM: f32 = 16.0;

/// 오류 안내 줄 높이.
const ERROR_HEIGHT: f32 = 28.0;

/// 애니메이션을 다시 그리는 횟수(초당).
pub(super) const FPS: f32 = 30.0;

type Theme = extension::Theme;

/// 그릴 것: 아직 없음(첫 결과를 기다린다), 모양, 실패한 까닭.
pub(super) enum Frame {
    Pending,
    Shapes(Rc<Vec<Shape>>),
    Error(String),
}

/// 캔버스 하나(에디터 블록, 대화상자 미리보기)의 그리기 상태. 작업 스레드에 요청을 보내고 결과를 받아 둔다.
pub(super) struct Player {
    /// 만든 때. 애니메이션의 `t`는 여기서 잰다.
    started: Instant,
    /// 모양별 삼각형 캐시. 같은 프레임·같은 모양을 다시 나누지 않는다.
    cache: Rc<RefCell<TessCache>>,
    state: RefCell<PlayerState>,
}

#[derive(Default)]
struct PlayerState {
    /// 지금 결과를 만든 확장·테마·인자. 하나라도 바뀌면 받은 것을 버리고 새로 그린다.
    source: Option<Source>,
    /// 작업 스레드에 보내는 요청 번호. `source`가 바뀔 때마다 새 번호를 써서 옛 결과와 섞이지 않는다.
    request: u64,
    /// 기다리는 요청의 프레임 번호.
    pending: Option<u64>,
    /// 마지막으로 받은 프레임 번호와 결과.
    shown: Option<(u64, Drawn)>,
    /// `period`가 있는 애니메이션의 한 주기 프레임(프레임 번호 → 모양).
    frames: HashMap<u64, Rc<Vec<Shape>>>,
}

/// 결과를 만든 확장·테마·인자.
type Source = (Rc<Loaded>, Theme, Vec<(String, String)>);

/// 받은 결과: 모양, 또는 실패한 까닭.
type Drawn = Result<Rc<Vec<Shape>>, String>;

/// 요청 번호(앱 전체에서 겹치지 않는다).
fn next_request() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

impl Player {
    pub(super) fn new() -> Self {
        Self {
            started: Instant::now(),
            cache: Rc::default(),
            state: RefCell::default(),
        }
    }

    pub(super) fn cache(&self) -> Rc<RefCell<TessCache>> {
        self.cache.clone()
    }

    /// 지금 그릴 프레임. 필요하면 작업 스레드에 다음 프레임을 요청하고, 다시 그릴 때를 예약한다.
    pub(super) fn frame(
        &self,
        loaded: &Rc<Loaded>,
        args: &[(String, String)],
        theme: Theme,
        ticker: &Ticker,
        window: &mut Window,
        cx: &mut App,
    ) -> Frame {
        let Ok(worker) = &**loaded else {
            return Frame::Pending;
        };
        let mut state = self.state.borrow_mut();
        let same = state
            .source
            .as_ref()
            .is_some_and(|(drawn, drawn_theme, drawn_args)| {
                Rc::ptr_eq(drawn, loaded) && *drawn_theme == theme && drawn_args == args
            });
        if !same {
            *state = PlayerState {
                source: Some((loaded.clone(), theme, args.to_vec())),
                request: next_request(),
                ..PlayerState::default()
            };
        }
        // 받은 결과를 들인다.
        if let Some(pending) = state.pending
            && let Some(result) = worker.take(state.request)
        {
            state.pending = None;
            let result = result.map(Rc::new);
            if let (Some(_), Ok(shapes)) = (worker.info.period, &result) {
                state.frames.insert(pending, shapes.clone());
            }
            state.shown = Some((pending, result));
        }
        // 이번에 보일 프레임.
        let animate = worker.info.animated && !cx.reduce_motion();
        let mut frame = 0;
        if animate {
            frame = (self.started.elapsed().as_secs_f32() * FPS) as u64;
            if let Some(period) = worker.info.period {
                frame %= ((period * FPS).round() as u64).max(1);
            }
        }
        let cached = state.frames.get(&frame).cloned();
        if let Some(shapes) = cached {
            state.shown = Some((frame, Ok(shapes)));
        } else if state.pending.is_none()
            && state
                .shown
                .as_ref()
                .is_none_or(|(shown, _)| *shown != frame)
        {
            worker.request(state.request, frame as f32 / FPS, args, theme);
            state.pending = Some(frame);
        }
        // 기다리는 결과가 있으면 다음 화면 갱신 때 다시 본다. 애니메이션은 다음 프레임 시각에 다시 그린다.
        if state.pending.is_some() {
            window.request_animation_frame();
        } else if animate && window.is_window_active() {
            ticker.schedule(window, cx);
        }
        match &state.shown {
            None => Frame::Pending,
            Some((_, Ok(shapes))) => Frame::Shapes(shapes.clone()),
            Some((_, Err(err))) => Frame::Error(err.clone()),
        }
    }
}

/// 애니메이션의 다음 프레임 예약. 한 화면(뷰)에 블록이 여럿이어도 예약은 하나만 둔다.
#[derive(Default)]
pub(super) struct Ticker {
    scheduled: Rc<Cell<bool>>,
}

impl Ticker {
    fn schedule(&self, window: &mut Window, cx: &mut App) {
        if self.scheduled.replace(true) {
            return;
        }
        let view = window.current_view();
        let scheduled = self.scheduled.clone();
        cx.spawn(async move |cx| {
            cx.background_executor()
                .timer(Duration::from_secs_f32(1.0 / FPS))
                .await;
            scheduled.set(false);
            cx.update(|cx| cx.notify(view));
        })
        .detach();
    }
}

/// 본문의 확장 블록 하나.
struct EditorBlock {
    block: extension::Block,
    /// 확장에 넘길 인자(`scale` 뺌)와 앱이 적용할 배율.
    args: Vec<(String, String)>,
    scale: f32,
    player: Player,
}

/// 열린 시트의 확장 블록 목록. 줄 스타일·줄 블록의 측정·그리기에서 함께 쓴다.
pub(super) struct ExtensionBlocks {
    host: Host,
    /// 확장 폴더의 파일과 수정 시각(마지막으로 본 것).
    files: RefCell<Vec<(String, Option<std::time::SystemTime>)>>,
    /// 줄 순서대로.
    blocks: RefCell<Vec<Rc<EditorBlock>>>,
    ticker: Ticker,
}

impl ExtensionBlocks {
    pub(super) fn new(text: &str, host: Host) -> Self {
        let blocks = Self {
            files: RefCell::new(extension::scan(host.root())),
            host,
            blocks: RefCell::default(),
            ticker: Ticker::default(),
        };
        blocks.sync(text);
        blocks
    }

    /// 본문에 맞춰 블록을 다시 찾는다. 바뀐 블록이 닿았던 줄과 닿는 줄을 돌려준다(다시 배치할 줄).
    pub(super) fn sync(&self, text: &str) -> Vec<Range<usize>> {
        let old = self.blocks.take();
        let new: Vec<Rc<EditorBlock>> = extension::blocks(text)
            .into_iter()
            .map(|block| {
                old.iter()
                    .find(|kept| kept.block == block)
                    .cloned()
                    .unwrap_or_else(|| {
                        Rc::new(EditorBlock {
                            args: block.extension_args(),
                            scale: block.scale(),
                            block,
                            player: Player::new(),
                        })
                    })
            })
            .collect();
        let changed = |from: &[Rc<EditorBlock>], to: &[Rc<EditorBlock>]| {
            from.iter()
                .filter(|block| !to.iter().any(|other| Rc::ptr_eq(block, other)))
                .map(|block| block.block.rows.clone())
                .collect::<Vec<_>>()
        };
        let mut rows = changed(&old, &new);
        rows.extend(changed(&new, &old));
        self.blocks.replace(new);
        rows
    }

    /// 확장 폴더가 지난번에 본 뒤로 바뀌었는지.
    pub(super) fn files_changed(&self) -> bool {
        let files = extension::scan(self.host.root());
        if *self.files.borrow() == files {
            return false;
        }
        self.files.replace(files);
        true
    }

    /// 줄이 확장 블록(펜스 포함)인지.
    pub(super) fn contains(&self, row: usize) -> bool {
        self.block_at(row).is_some()
    }

    /// 줄 아래 캔버스 높이. 블록의 닫는 줄이 아니면 0이다.
    pub(super) fn measure(&self, row: usize, wrap_width: Option<Pixels>) -> Pixels {
        let Some(block) = self.last_row_block(row) else {
            return px(0.);
        };
        let width = wrap_width.unwrap_or(px(super::editor::EDITOR_MAX_WIDTH));
        match &*self.host.get(&block.block.name) {
            Ok(worker) => match worker.size(&block.args) {
                Ok(size) => {
                    fit(scaled(size, block.scale), width).1 + px(PADDING_TOP + PADDING_BOTTOM)
                }
                Err(_) => px(ERROR_HEIGHT + PADDING_TOP + PADDING_BOTTOM),
            },
            Err(_) => px(ERROR_HEIGHT + PADDING_TOP + PADDING_BOTTOM),
        }
    }

    /// 블록 닫는 줄 아래에 캔버스를 그린다. 블록이 아니면 `None`이다.
    pub(super) fn render(
        &self,
        row: usize,
        size: Size<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        let block = self.last_row_block(row)?;
        let pal = Palette::from_theme(cx.theme());
        let loaded = self.host.get(&block.block.name);
        let canvas_size = match &*loaded {
            Ok(worker) => worker.size(&block.args),
            Err(err) => Err(err.clone()),
        };
        let canvas_size = match canvas_size {
            Ok(canvas_size) => canvas_size,
            Err(err) => return Some(error_box(&block.block.name, &err, size, pal)),
        };
        let frame = block
            .player
            .frame(&loaded, &block.args, theme(pal), &self.ticker, window, cx);
        let (width, height) = fit(scaled(canvas_size, block.scale), size.width);
        let left = (size.width - width) / 2.;
        let shapes = match frame {
            Frame::Shapes(shapes) => Some(shapes),
            Frame::Pending => None,
            Frame::Error(err) => return Some(error_box(&block.block.name, &err, size, pal)),
        };
        Some(
            div()
                .w(size.width)
                .h(size.height)
                .pt(px(PADDING_TOP))
                .pl(left)
                .child(shapes_canvas(
                    shapes,
                    block.player.cache(),
                    canvas_size,
                    width,
                    height,
                ))
                .into_any_element(),
        )
    }

    fn block_at(&self, row: usize) -> Option<Rc<EditorBlock>> {
        let blocks = self.blocks.borrow();
        let ix = blocks.partition_point(|block| block.block.rows.end <= row);
        blocks
            .get(ix)
            .filter(|block| block.block.rows.contains(&row))
            .cloned()
    }

    fn last_row_block(&self, row: usize) -> Option<Rc<EditorBlock>> {
        self.block_at(row)
            .filter(|block| block.block.rows.end == row + 1)
    }
}

/// 모양을 `width` × `height`에 맞춰 그리는 캔버스. `canvas_size`는 배율을 곱하기 전 크기다(모양의
/// 좌표가 이 크기 기준이다). 모양이 아직 없으면 빈 자리다.
pub(super) fn shapes_canvas(
    shapes: Option<Rc<Vec<Shape>>>,
    cache: Rc<RefCell<TessCache>>,
    canvas_size: (f32, f32),
    width: Pixels,
    height: Pixels,
) -> impl IntoElement {
    let scale = f32::from(width) / canvas_size.0;
    canvas(
        |_, _, _| (),
        move |bounds, (), window, cx| {
            if let Some(shapes) = shapes {
                window.with_content_mask(Some(ContentMask { bounds }), |window| {
                    paint(&shapes, bounds.origin, scale, &cache, window, cx);
                });
            }
        },
    )
    .w(width)
    .h(height)
}

/// 배율 `scale`을 곱한 캔버스 크기.
pub(super) fn scaled((w, h): (f32, f32), scale: f32) -> (f32, f32) {
    (w * scale, h * scale)
}

/// 캔버스 크기를 폭 `width` 안에 맞춘다(키우지는 않는다).
pub(super) fn fit((w, h): (f32, f32), width: Pixels) -> (Pixels, Pixels) {
    let scale = (f32::from(width) / w).min(1.0);
    (px(w * scale), px(h * scale))
}

pub(super) fn theme(pal: Palette) -> Theme {
    let color = |hsla: Hsla| {
        let rgba = Rgba::from(hsla);
        [rgba.r, rgba.g, rgba.b, rgba.a]
    };
    Theme {
        foreground: color(pal.foreground),
        background: color(pal.background),
        muted: color(pal.muted),
        accent: color(pal.accent),
        border: color(pal.border),
    }
}

fn error_box(name: &str, err: &str, size: Size<Pixels>, pal: Palette) -> AnyElement {
    div()
        .w(size.width)
        .h(size.height)
        .pt(px(PADDING_TOP))
        .child(
            div()
                .h(px(ERROR_HEIGHT))
                .px_2()
                .flex()
                .items_center()
                .rounded_md()
                .bg(pal.tint)
                .text_xs()
                .text_color(pal.danger)
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(t!("extensions.block_error", name = name, err = err)),
        )
        .into_any_element()
}
