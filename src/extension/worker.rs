//! 확장의 작업 스레드. `draw`를 UI 스레드 밖에서 돌려, 느린 확장이 편집을 막지 않게 한다.
//!
//! 확장마다 스레드 하나가 Lua 상태를 만들어 끝까지 들고 있고(Lua 상태는 스레드를 옮기지 않는다), UI
//! 스레드와는 요청과 결과(모양 목록)만 주고받는다. 요청 통로가 끊기면(캐시에서 버리면) 스레드가 끝난다.
//!
//! 그리는 쪽은 블록마다 번호를 붙여 요청하고, 결과가 오면 [`Worker::take`]로 그 블록의 것을 가져간다.
//! 결과가 오기 전까지는 지난 결과를 그린다.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use super::{Clicked, Drawing, Extension, Theme, Ui};

/// UI 스레드가 읽기를 기다리는 최대 시간. Lua 쪽 제한([`super::LOAD_TIMEOUT`])보다 조금 길다(스레드를 띄우고
/// 파일을 해석하는 시간).
const LOAD_WAIT: Duration = Duration::from_millis(400);

/// 앱이 알아야 할 확장 정보.
#[derive(Clone, Debug, PartialEq)]
pub struct Info {
    /// 캔버스 크기(논리 단위). 블록 인자로 정하면 `None`이다([`Worker::size`]).
    pub size: Option<(f32, f32)>,
    pub animated: bool,
    pub period: Option<f32>,
    pub draws: bool,
    /// 영역을 누르면 `click`을 부른다.
    pub clicks: bool,
    /// 인라인 확장으로 글을 보인다(`value`).
    pub values: bool,
    /// 창에서 실행한다(`play`).
    pub plays: bool,
    /// 책 미리보기·내보내기에 넣는 것이 기본인지(반환 표의 `export`).
    pub export: bool,
    pub commands: Vec<String>,
}

enum Request {
    Draw {
        id: u64,
        t: f32,
        args: Vec<(String, String)>,
        theme: Theme,
        ui: Ui,
    },
    /// 영역 `region`을 눌렀다.
    Click {
        id: u64,
        region: String,
        args: Vec<(String, String)>,
    },
    /// `value(args, ctx)`를 묻는다.
    Value {
        id: u64,
        args: Vec<(String, String)>,
        sheet: String,
    },
    /// `size(args)`를 묻는다.
    Size {
        id: u64,
        args: Vec<(String, String)>,
    },
}

/// 크기 답(요청 번호, 크기).
type SizeReply = (u64, Result<(f32, f32), String>);

/// 누른 답(요청 번호, 결과).
type ClickReply = (u64, Result<Clicked, String>);

/// 글 답(요청 번호, 글).
type ValueReply = (u64, Result<Option<String>, String>);

/// 크기는 블록 인자마다 한 번 묻는다.
type SizeCache = HashMap<Vec<(String, String)>, Result<(f32, f32), String>>;

struct Reply {
    id: u64,
    drawing: Result<Drawing, String>,
}

/// 작업 스레드에 띄운 확장. UI 스레드에서만 쓴다.
pub struct Worker {
    pub info: Info,
    requests: Sender<Request>,
    replies: Receiver<Reply>,
    sizes: Receiver<SizeReply>,
    clicks: Receiver<ClickReply>,
    values: Receiver<ValueReply>,
    /// 받았지만 아직 가져가지 않은 결과(블록 번호별 최신 것).
    inbox: RefCell<HashMap<u64, Result<Drawing, String>>>,
    size_cache: RefCell<SizeCache>,
    /// 다음 크기·누름 요청 번호. 늦게 온 옛 답을 걸러 낸다.
    next_size: std::cell::Cell<u64>,
}

impl std::fmt::Debug for Worker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Worker")
            .field("info", &self.info)
            .finish_non_exhaustive()
    }
}

impl Worker {
    /// 스레드를 띄워 `source`를 읽는다. 읽기가 끝날 때까지 기다린다(캔버스 크기가 있어야 배치한다).
    pub fn spawn(name: &str, source: String) -> Result<Self, String> {
        let (request_tx, request_rx) = channel::<Request>();
        let (reply_tx, reply_rx) = channel::<Reply>();
        let (size_tx, size_rx) = channel::<SizeReply>();
        let (click_tx, click_rx) = channel::<ClickReply>();
        let (value_tx, value_rx) = channel::<ValueReply>();
        let (info_tx, info_rx) = channel::<Result<Info, String>>();
        let thread_name = name.to_string();
        std::thread::Builder::new()
            .name(format!("extension-{name}"))
            .spawn(move || {
                let extension = match Extension::load(&thread_name, &source) {
                    Ok(extension) => extension,
                    Err(err) => {
                        info_tx.send(Err(err)).ok();
                        return;
                    }
                };
                if info_tx.send(Ok(extension.info())).is_err() {
                    return;
                }
                while let Ok(request) = request_rx.recv() {
                    let sent = match request {
                        Request::Draw {
                            id,
                            t,
                            args,
                            theme,
                            ui,
                        } => {
                            let drawing = extension.draw_ui(t, &args, &theme, &ui);
                            reply_tx.send(Reply { id, drawing }).is_ok()
                        }
                        Request::Click { id, region, args } => {
                            click_tx.send((id, extension.click(&region, &args))).is_ok()
                        }
                        Request::Value { id, args, sheet } => {
                            value_tx.send((id, extension.value(&args, &sheet))).is_ok()
                        }
                        Request::Size { id, args } => {
                            size_tx.send((id, extension.size_for(&args))).is_ok()
                        }
                    };
                    if !sent {
                        break;
                    }
                }
            })
            .map_err(|err| format!("{name}.lua: could not start: {err}"))?;
        let info = info_rx
            .recv_timeout(LOAD_WAIT)
            .map_err(|_| format!("{name}.lua: took too long to load"))??;
        Ok(Self {
            info,
            requests: request_tx,
            replies: reply_rx,
            sizes: size_rx,
            clicks: click_rx,
            values: value_rx,
            inbox: RefCell::default(),
            size_cache: RefCell::default(),
            next_size: std::cell::Cell::new(1),
        })
    }

    /// 블록 `id`의 `t`초 그림을 요청한다. 결과는 [`Self::take`]로 가져간다.
    pub fn request(&self, id: u64, t: f32, args: &[(String, String)], theme: Theme, ui: Ui) {
        self.requests
            .send(Request::Draw {
                id,
                t,
                args: args.to_vec(),
                theme,
                ui,
            })
            .ok();
    }

    /// 영역 `region`을 누른 결과(블록 인자 변경과 동작). 작업 스레드의 답을 기다린다(누름은 드물고 짧다).
    pub fn click(&self, region: &str, args: &[(String, String)]) -> Result<Clicked, String> {
        if !self.info.clicks {
            return Ok(Clicked::default());
        }
        let id = self.next_size.replace(self.next_size.get() + 1);
        self.requests
            .send(Request::Click {
                id,
                region: region.to_string(),
                args: args.to_vec(),
            })
            .ok();
        let deadline = std::time::Instant::now() + LOAD_WAIT;
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match self.clicks.recv_timeout(left) {
                Ok((reply, changes)) if reply == id => return changes,
                Ok(_) => continue,
                Err(_) => return Err("click took too long".into()),
            }
        }
    }

    /// 인라인 확장의 글(`value(args, ctx)`). 작업 스레드의 답을 기다린다. `value`가 없으면 `None`.
    pub fn value(&self, args: &[(String, String)], sheet: &str) -> Result<Option<String>, String> {
        if !self.info.values {
            return Ok(None);
        }
        let id = self.next_size.replace(self.next_size.get() + 1);
        self.requests
            .send(Request::Value {
                id,
                args: args.to_vec(),
                sheet: sheet.to_string(),
            })
            .ok();
        let deadline = std::time::Instant::now() + LOAD_WAIT;
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match self.values.recv_timeout(left) {
                Ok((reply, value)) if reply == id => return value,
                Ok(_) => continue,
                Err(_) => return Err("value took too long".into()),
            }
        }
    }

    /// 블록 인자 `args`일 때의 캔버스 크기. `size`가 함수면 작업 스레드에 묻고 답을 기다린다(배치에
    /// 필요하다). 인자마다 한 번만 묻는다.
    pub fn size(&self, args: &[(String, String)]) -> Result<(f32, f32), String> {
        if let Some(size) = self.info.size {
            return Ok(size);
        }
        if let Some(size) = self.size_cache.borrow().get(args) {
            return size.clone();
        }
        let id = self.next_size.replace(self.next_size.get() + 1);
        self.requests
            .send(Request::Size {
                id,
                args: args.to_vec(),
            })
            .ok();
        let deadline = std::time::Instant::now() + LOAD_WAIT;
        let size = loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match self.sizes.recv_timeout(left) {
                Ok((reply, size)) if reply == id => break size,
                Ok(_) => continue,
                Err(_) => break Err("size(args) took too long".into()),
            }
        };
        self.size_cache
            .borrow_mut()
            .insert(args.to_vec(), size.clone());
        size
    }

    /// 블록 `id`에 온 결과가 있으면 가져간다.
    pub fn take(&self, id: u64) -> Option<Result<Drawing, String>> {
        let mut inbox = self.inbox.borrow_mut();
        for reply in self.replies.try_iter() {
            inbox.insert(reply.id, reply.drawing);
        }
        inbox.remove(&id)
    }

    /// 결과를 기다린다(테스트용).
    #[cfg(test)]
    pub fn wait(&self, id: u64) -> Result<Drawing, String> {
        loop {
            if let Some(shapes) = self.take(id) {
                return shapes;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

/// 플레이 창의 입력(LÖVE 콜백 이름을 따른다, ADR-0038). 좌표는 캔버스 좌표다.
#[derive(Clone, Debug, PartialEq)]
pub enum PlayEvent {
    KeyPressed(String),
    KeyReleased(String),
    MousePressed { x: f32, y: f32, button: u8 },
    MouseReleased { x: f32, y: f32, button: u8 },
    MouseMoved { x: f32, y: f32 },
}

struct PlayFrame {
    dt: f32,
    events: Vec<PlayEvent>,
    theme: Theme,
}

/// 창에서 실행하는 확장(ADR-0038): 블록과 따로 Lua 상태를 하나 들고 있는 작업 스레드. UI는 한 프레임마다 그 사이의
/// 입력과 지난 시간을 보내고, 확장은 입력 콜백 → `update(dt)` → `draw(g)` 순으로 돌아 모양을 돌려준다. 버리면(창을
/// 닫으면) 스레드가 끝난다.
pub struct PlayWorker {
    /// 캔버스 크기.
    pub size: (f32, f32),
    frames: Sender<PlayFrame>,
    replies: Receiver<Result<Vec<super::Shape>, String>>,
}

impl PlayWorker {
    /// `source`를 읽고 `play.load(args)`를 부른다. 끝날 때까지 기다린다.
    pub fn spawn(name: &str, source: String, args: Vec<(String, String)>) -> Result<Self, String> {
        let (frame_tx, frame_rx) = channel::<PlayFrame>();
        let (reply_tx, reply_rx) = channel::<Result<Vec<super::Shape>, String>>();
        let (ready_tx, ready_rx) = channel::<Result<(f32, f32), String>>();
        let thread_name = name.to_string();
        std::thread::Builder::new()
            .name(format!("play-{name}"))
            .spawn(move || {
                let started = Extension::load(&thread_name, &source).and_then(|extension| {
                    if !extension.plays {
                        return Err(format!("{thread_name}.lua has no play table"));
                    }
                    let size = extension.play_size(&args)?;
                    extension.play_load(&args)?;
                    Ok((extension, size))
                });
                let (extension, size) = match started {
                    Ok(started) => started,
                    Err(err) => {
                        ready_tx.send(Err(err)).ok();
                        return;
                    }
                };
                if ready_tx.send(Ok(size)).is_err() {
                    return;
                }
                while let Ok(frame) = frame_rx.recv() {
                    let result = (|| {
                        for event in frame.events {
                            match event {
                                PlayEvent::KeyPressed(key) => {
                                    extension.play_call("keypressed", key)?
                                }
                                PlayEvent::KeyReleased(key) => {
                                    extension.play_call("keyreleased", key)?
                                }
                                PlayEvent::MousePressed { x, y, button } => {
                                    extension.play_call("mousepressed", (x, y, button))?
                                }
                                PlayEvent::MouseReleased { x, y, button } => {
                                    extension.play_call("mousereleased", (x, y, button))?
                                }
                                PlayEvent::MouseMoved { x, y } => {
                                    extension.play_call("mousemoved", (x, y))?
                                }
                            }
                        }
                        extension.play_call("update", frame.dt)?;
                        extension.play_draw(size, &frame.theme)
                    })();
                    let failed = result.is_err();
                    if reply_tx.send(result).is_err() || failed {
                        break;
                    }
                }
            })
            .map_err(|err| format!("{name}.lua: could not start: {err}"))?;
        let size = ready_rx
            .recv_timeout(LOAD_WAIT)
            .map_err(|_| format!("{name}.lua: took too long to start"))??;
        Ok(Self {
            size,
            frames: frame_tx,
            replies: reply_rx,
        })
    }

    /// 다음 프레임을 요청한다. 답은 [`Self::take`]로 가져간다.
    pub fn frame(&self, dt: f32, events: Vec<PlayEvent>, theme: Theme) {
        self.frames.send(PlayFrame { dt, events, theme }).ok();
    }

    /// 온 프레임이 있으면 가장 새 것을 가져간다.
    pub fn take(&self) -> Option<Result<Vec<super::Shape>, String>> {
        self.replies.try_iter().last()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_on_its_own_thread() {
        let worker = Worker::spawn(
            "dot",
            "return { size = { 10, 20 }, animated = true, period = 2, \
             draw = function(g, t) g.circle('fill', t, 0, 1) end }"
                .into(),
        )
        .unwrap();
        assert_eq!(worker.info.size, Some((10.0, 20.0)));
        assert_eq!(worker.info.period, Some(2.0));
        worker.request(1, 0.0, &[], Theme::default(), Ui::default());
        worker.request(2, 5.0, &[], Theme::default(), Ui::default());
        let second = worker.wait(2).unwrap();
        let first = worker.wait(1).unwrap();
        assert_ne!(first, second);
    }

    #[test]
    fn draws_regions_and_answers_clicks() {
        let worker = Worker::spawn(
            "button",
            "return { draw = function(g, t, args, ui) \
               g.setColor(ui.hover == 'b' and '#ff0000' or '#000000') \
               g.rectangle('fill', 0, 0, 10, 10) g.region('b', 0, 0, 10, 10) end, \
             click = function(id, args) return { on = not args.on, n = (args.n or 0) + 1 } end }"
                .into(),
        )
        .unwrap();
        assert!(worker.info.clicks);
        worker.request(1, 0.0, &[], Theme::default(), Ui::default());
        let (plain, regions) = worker.wait(1).unwrap();
        assert_eq!(regions.len(), 1);
        let hover = Ui {
            hover: Some("b".into()),
            ..Ui::default()
        };
        worker.request(2, 0.0, &[], Theme::default(), hover);
        assert_ne!(worker.wait(2).unwrap().0, plain);
        assert_eq!(
            worker
                .click("b", &[("on".into(), "true".into())])
                .map(|clicked| clicked.args),
            Ok(vec![
                ("n".into(), "1".into()),
                ("on".into(), "false".into())
            ])
        );
    }

    #[test]
    fn plays_with_state_and_input() {
        let worker = PlayWorker::spawn(
            "counter",
            "local n = 0 \
             return { draw = function() end, play = { size = { 30, 10 }, \
               load = function(args) n = args.start or 0 end, \
               keypressed = function(key) if key == 'space' then n = n + 10 end end, \
               update = function(dt) n = n + 1 end, \
               draw = function(g) for i = 1, n do g.rectangle('fill', i, 0, 1, 1) end end } }"
                .into(),
            vec![("start".into(), "2".into())],
        )
        .unwrap();
        assert_eq!(worker.size, (30.0, 10.0));
        let next = |events: Vec<PlayEvent>| {
            worker.frame(0.016, events, Theme::default());
            loop {
                if let Some(shapes) = worker.take() {
                    return shapes.unwrap().len();
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        };
        assert_eq!(next(Vec::new()), 3);
        assert_eq!(next(vec![PlayEvent::KeyPressed("space".into())]), 14);
        let err = PlayWorker::spawn(
            "none",
            "return { draw = function() end }".into(),
            Vec::new(),
        )
        .err()
        .unwrap();
        assert!(err.contains("no play table"), "{err}");
    }

    #[test]
    fn asks_size_for_block_args() {
        let worker = Worker::spawn(
            "s",
            "return { size = function(args) return { args.w or 10, 5 } end, draw = function() end }"
                .into(),
        )
        .unwrap();
        assert_eq!(worker.info.size, None);
        assert_eq!(worker.size(&[("w".into(), "30".into())]), Ok((30.0, 5.0)));
        assert_eq!(worker.size(&[]), Ok((10.0, 5.0)));
    }

    #[test]
    fn slow_loads_fail_without_blocking_long() {
        let started = std::time::Instant::now();
        let err = Worker::spawn("slow", "while true do end return {}".into()).unwrap_err();
        assert!(err.contains("took too long"), "{err}");
        assert!(started.elapsed() < LOAD_WAIT + Duration::from_millis(200));
        let err = Worker::spawn("bad", "return 1".into()).unwrap_err();
        assert!(err.contains("must return a table"), "{err}");
    }
}
