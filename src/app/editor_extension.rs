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
use gpui_kit::component::input::EditorState;
use gpui_kit::gpui::prelude::FluentBuilder as _;
use gpui_kit::gpui::*;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use super::extension_painter::{TessCache, paint};
use super::style::Palette;
use crate::extension::{self, Host, Loaded, Region, Shape, Ui};

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
    /// 마우스 상태(ADR-0032). 바뀌면 그 상태로 다시 그린다.
    ui: RefCell<Ui>,
    /// 마지막으로 그린 캔버스 자리. 마우스 위치를 캔버스 좌표로 바꾸는 데 쓴다.
    bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// 마지막 `click`이 실패한 까닭과 그때의 확장. 확장이 다시 읽히면(고치면) 지운 것으로 본다.
    click_error: RefCell<Option<(Rc<Loaded>, String)>>,
}

#[derive(Default)]
struct PlayerState {
    /// 지금 결과를 만든 확장·테마·인자. 하나라도 바뀌면 받은 것을 버리고 새로 그린다.
    source: Option<Source>,
    /// 작업 스레드에 보내는 요청 번호. `source`가 바뀔 때마다 새 번호를 써서 옛 결과와 섞이지 않는다.
    request: u64,
    /// 기다리는 요청의 프레임 번호와 마우스 상태.
    pending: Option<(u64, Ui)>,
    /// 마지막으로 받은 프레임 번호·마우스 상태와 결과.
    shown: Option<((u64, Ui), Drawn)>,
    /// `period`가 있는 애니메이션의 한 주기 프레임(프레임 번호 → 그림). 마우스 상태가 없는 프레임만 둔다.
    frames: HashMap<u64, Shown>,
}

/// 받은 그림: 모양과 누를 수 있는 영역.
type Shown = (Rc<Vec<Shape>>, Rc<Vec<Region>>);

/// 결과를 만든 확장·테마·인자.
type Source = (Rc<Loaded>, Theme, Vec<(String, String)>);

/// 받은 결과: 그림, 또는 실패한 까닭.
type Drawn = Result<Shown, String>;

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
            ui: RefCell::default(),
            bounds: Rc::default(),
            click_error: RefCell::default(),
        }
    }

    /// 마지막으로 받은 그림의 영역.
    fn regions(&self) -> Rc<Vec<Region>> {
        match &self.state.borrow().shown {
            Some((_, Ok((_, regions)))) => regions.clone(),
            _ => Rc::default(),
        }
    }

    /// 창 좌표 `position` 위의 영역.
    fn region_at(&self, position: Point<Pixels>, canvas_width: f32) -> Option<String> {
        let bounds = self.bounds.get()?;
        if !bounds.contains(&position) {
            return None;
        }
        let scale = f32::from(bounds.size.width) / canvas_width.max(1.0);
        let local = position - bounds.origin;
        let point = [f32::from(local.x) / scale, f32::from(local.y) / scale];
        extension::region_at(&self.regions(), point).map(ToString::to_string)
    }

    /// 마우스 상태를 바꾼다. 바뀌었으면 `true`(다시 그려야 한다).
    fn set_ui(&self, ui: Ui) -> bool {
        let mut current = self.ui.borrow_mut();
        if *current == ui {
            return false;
        }
        *current = ui;
        true
    }

    pub(super) fn cache(&self) -> Rc<RefCell<TessCache>> {
        self.cache.clone()
    }

    /// 그린 자리를 적어 둘 곳(마우스 위치를 캔버스 좌표로 바꾼다).
    pub(super) fn bounds(&self) -> Rc<Cell<Option<Bounds<Pixels>>>> {
        self.bounds.clone()
    }

    /// 마지막으로 받은 그림에 영역이 있는지.
    pub(super) fn has_regions(&self) -> bool {
        !self.regions().is_empty()
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
            // 받은 그림은 새 그림이 올 때까지 그대로 보인다(누르기로 인자가 바뀔 때 깜빡이지 않게).
            // 다시 요청하도록 프레임 번호는 지운다.
            let shown = state
                .shown
                .take()
                .map(|(_, drawn)| ((u64::MAX, Ui::default()), drawn));
            *state = PlayerState {
                source: Some((loaded.clone(), theme, args.to_vec())),
                request: next_request(),
                shown,
                ..PlayerState::default()
            };
        }
        // 받은 결과를 들인다.
        if state.pending.is_some()
            && let Some(result) = worker.take(state.request)
            && let Some(pending) = state.pending.take()
        {
            let result = result.map(|(shapes, regions)| (Rc::new(shapes), Rc::new(regions)));
            if let (Some(_), Ok(drawn)) = (worker.info.period, &result)
                && pending.1.is_empty()
            {
                state.frames.insert(pending.0, drawn.clone());
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
        let ui = self.ui.borrow().clone();
        let key = (frame, ui);
        let cached = key
            .1
            .is_empty()
            .then(|| state.frames.get(&frame).cloned())
            .flatten();
        if let Some(drawn) = cached {
            state.shown = Some((key, Ok(drawn)));
        } else if state.pending.is_none()
            && state.shown.as_ref().is_none_or(|(shown, _)| *shown != key)
        {
            worker.request(
                state.request,
                frame as f32 / FPS,
                args,
                theme,
                key.1.clone(),
            );
            state.pending = Some(key);
        }
        // 기다리는 결과가 있으면 다음 화면 갱신 때 다시 본다. 애니메이션은 다음 프레임 시각에 다시 그린다.
        if state.pending.is_some() {
            window.request_animation_frame();
        } else if animate && window.is_window_active() {
            ticker.schedule(window, cx);
        }
        match &state.shown {
            None => Frame::Pending,
            Some((_, Ok((shapes, _)))) => Frame::Shapes(shapes.clone()),
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
    /// 본문 폭 안의 자리(`align:`).
    align: extension::Align,
    /// 같은 자리의 블록이 고쳐져도(누르기로 인자가 바뀌어도) 이어 쓴다.
    player: Rc<Player>,
}

/// 누르기(`click`)가 돌려준 인자 변경을 쓰는 곳. 에디터 블록은 블록 본문에, 편집 대화상자는 인자 칸에 쓴다.
pub(super) type OnChanges = Rc<dyn Fn(&[(String, String)], &mut Window, &mut App)>;

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
                        let player = old
                            .iter()
                            .find(|kept| {
                                kept.block.name == block.name
                                    && kept.block.rows.start == block.rows.start
                            })
                            .map_or_else(|| Rc::new(Player::new()), |kept| kept.player.clone());
                        Rc::new(EditorBlock {
                            args: block.extension_args(),
                            scale: block.scale(),
                            align: extension::align_of(&block.args),
                            block,
                            player,
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

    /// 블록 닫는 줄 아래에 캔버스를 그린다. 블록이 아니면 `None`이다. 영역(`g.region`)이 있으면 마우스를
    /// 받는다: 올라가면 그 상태로 다시 그리고, 누르면 `click`이 돌려준 인자를 `editor`의 블록 본문에 쓴다.
    ///
    /// 캔버스를 누르면 커서를 옮기지 않는다(펜스가 펼쳐지지 않는다). 영역 밖을 두 번 누르거나 보조 키(⌘)와
    /// 누르면 `app`의 블록 편집 대화상자를 연다.
    pub(super) fn render(
        &self,
        row: usize,
        size: Size<Pixels>,
        editor: &WeakEntity<EditorState>,
        app: &WeakEntity<super::WriterApp>,
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
        let left = match block.align {
            extension::Align::Left => px(0.),
            extension::Align::Center => (size.width - width) / 2.,
            extension::Align::Right => size.width - width,
        };
        let shapes = match frame {
            Frame::Shapes(shapes) => Some(shapes),
            Frame::Pending => None,
            Frame::Error(err) => return Some(error_box(&block.block.name, &err, size, pal)),
        };
        let click_error = block
            .player
            .click_error
            .borrow()
            .as_ref()
            .filter(|(failed, _)| Rc::ptr_eq(failed, &loaded))
            .map(|(_, err)| err.clone());
        if let Some(err) = click_error {
            return Some(error_box(&block.block.name, &err, size, pal));
        }
        let canvas = div().size_full().child(shapes_canvas(
            shapes,
            block.player.cache(),
            Some(block.player.bounds.clone()),
            canvas_size,
            width,
            height,
        ));
        let canvas = if block.player.regions().is_empty() {
            canvas.into_any_element()
        } else {
            let (editor, rows) = (editor.clone(), block.block.rows.clone());
            let on_changes: OnChanges = Rc::new(move |changes, window, cx| {
                let rows = rows.clone();
                editor
                    .update(cx, |state, cx| write_args(state, rows, changes, window, cx))
                    .ok();
            });
            interactive(
                canvas,
                ("extension-block", block.block.rows.start).into(),
                block.player.clone(),
                loaded,
                block.args.clone(),
                canvas_size.0,
                on_changes,
            )
        };
        let (app, first_row) = (app.clone(), block.block.rows.start);
        Some(
            div()
                .w(size.width)
                .h(size.height)
                .pt(px(PADDING_TOP))
                .pl(left)
                .child(
                    div()
                        .id(("extension-canvas", first_row))
                        .w(width)
                        .h(height)
                        // 영역을 누른 것은 안쪽(`interactive`)이 먼저 받아 여기까지 오지 않는다.
                        .on_mouse_down(MouseButton::Left, move |event, window, cx| {
                            window.prevent_default();
                            cx.stop_propagation();
                            if event.click_count >= 2 || event.modifiers.secondary() {
                                let app = app.clone();
                                window.defer(cx, move |window, cx| {
                                    app.update(cx, |app, cx| {
                                        app.edit_extension_block(first_row, window, cx);
                                    })
                                    .ok();
                                });
                            }
                        })
                        .child(canvas),
                )
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

/// 영역이 있는 캔버스(에디터 블록, 편집 대화상자의 미리보기): 포인터가 영역에 오가면 그 상태로 다시 그리고,
/// 영역을 눌렀다 떼면 `click`을 불러 돌려준 인자를 `on_changes`로 넘긴다. 영역 밖을 누른 것은 바깥이 받는다.
pub(super) fn interactive(
    canvas: Div,
    id: ElementId,
    player: Rc<Player>,
    loaded: Rc<Loaded>,
    args: Vec<(String, String)>,
    canvas_width: f32,
    on_changes: OnChanges,
) -> AnyElement {
    let hovering = player.ui.borrow().hover.is_some();
    let (moved, left, pressed, released) = (player.clone(), player.clone(), player.clone(), player);
    canvas
        .id(id)
        .when(hovering, |this| this.cursor_pointer())
        .on_mouse_move(move |event, window, _| {
            let ui = Ui {
                hover: moved.region_at(event.position, canvas_width),
                pressed: moved.ui.borrow().pressed.clone(),
            };
            if moved.set_ui(ui) {
                window.refresh();
            }
        })
        .on_hover(move |hovered, window, _| {
            if !hovered && left.set_ui(Ui::default()) {
                window.refresh();
            }
        })
        .on_mouse_down(MouseButton::Left, move |event, window, cx| {
            let Some(region) = pressed.region_at(event.position, canvas_width) else {
                return;
            };
            window.prevent_default();
            cx.stop_propagation();
            pressed.set_ui(Ui {
                hover: Some(region.clone()),
                pressed: Some(region),
            });
            window.refresh();
        })
        .on_mouse_up(MouseButton::Left, move |event, window, cx| {
            let player = &released;
            let Some(region) = player.ui.borrow().pressed.clone() else {
                return;
            };
            cx.stop_propagation();
            let hover = player.region_at(event.position, canvas_width);
            player.set_ui(Ui {
                hover: hover.clone(),
                pressed: None,
            });
            window.refresh();
            // 누른 영역에서 뗐을 때만 누른 것이다.
            if hover.as_deref() != Some(region.as_str()) {
                return;
            }
            let Ok(worker) = &*loaded else {
                return;
            };
            match worker.click(&region, &args) {
                Ok(changes) if !changes.is_empty() => on_changes(&changes, window, cx),
                Ok(_) => {}
                Err(err) => {
                    *player.click_error.borrow_mut() = Some((loaded.clone(), err));
                }
            }
        })
        .into_any_element()
}

/// 줄마다 시작 바이트(첫 줄은 0). 줄 번호를 바이트 위치로 바꾸는 데 쓴다.
pub(super) fn line_starts(text: &str) -> Vec<usize> {
    std::iter::once(0)
        .chain(text.match_indices('\n').map(|(ix, _)| ix + 1))
        .collect()
}

/// 확장 블록(`rows`, 펜스 포함)의 바이트 범위. 닫는 줄 끝의 줄바꿈은 넣지 않는다.
pub(super) fn block_range(text: &str, rows: &Range<usize>) -> Option<Range<usize>> {
    let starts = line_starts(text);
    let start = *starts.get(rows.start)?;
    let end = starts.get(rows.end).map_or(text.len(), |next| next - 1);
    (start <= end).then_some(start..end)
}

/// 블록(`rows`, 펜스 포함) 본문에 인자 변경을 쓴다. 되돌리기 한 번으로 돌아가고, 커서는 그대로 둔다.
fn write_args(
    state: &mut EditorState,
    rows: Range<usize>,
    changes: &[(String, String)],
    window: &mut Window,
    cx: &mut Context<EditorState>,
) {
    let text = state.value();
    let starts = line_starts(&text);
    // 본문: 여는 줄 다음 줄부터 닫는 줄 앞까지.
    let (Some(&start), Some(&end)) = (starts.get(rows.start + 1), starts.get(rows.end - 1)) else {
        return;
    };
    if start > end {
        return;
    }
    let body: Vec<&str> = text[start..end].lines().collect();
    let mut new_body = extension::set_args(&body, changes).join("\n");
    new_body.push('\n');
    if new_body != text[start..end] {
        state.replace_range_keeping_selections(start..end, &new_body, window, cx);
    }
}

/// 모양을 `width` × `height`에 맞춰 그리는 캔버스. `canvas_size`는 배율을 곱하기 전 크기다(모양의
/// 좌표가 이 크기 기준이다). 모양이 아직 없으면 빈 자리다. `bounds`가 있으면 그린 자리를 적어 둔다.
pub(super) fn shapes_canvas(
    shapes: Option<Rc<Vec<Shape>>>,
    cache: Rc<RefCell<TessCache>>,
    bounds_out: Option<Rc<Cell<Option<Bounds<Pixels>>>>>,
    canvas_size: (f32, f32),
    width: Pixels,
    height: Pixels,
) -> impl IntoElement {
    let scale = f32::from(width) / canvas_size.0;
    canvas(
        |_, _, _| (),
        move |bounds, (), window, cx| {
            if let Some(out) = &bounds_out {
                out.set(Some(bounds));
            }
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

#[cfg(test)]
mod tests {
    use super::{block_range, line_starts};
    use crate::extension;

    #[test]
    fn block_range_covers_the_fence_without_the_last_newline() {
        let text = "앞\n```ext dot\nr: 5\n```\n뒤";
        let block = &extension::blocks(text)[0];
        let range = block_range(text, &block.rows).unwrap();
        assert_eq!(&text[range], "```ext dot\nr: 5\n```");
        // 문서 끝의 블록.
        let text = "```ext dot\n```";
        let block = &extension::blocks(text)[0];
        assert_eq!(block_range(text, &block.rows), Some(0..text.len()));
        assert_eq!(line_starts("a\nbc\n"), [0, 2, 5]);
    }
}
