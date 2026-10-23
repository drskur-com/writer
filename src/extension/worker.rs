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

use super::{Extension, Shape, Theme};

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
    pub commands: Vec<String>,
}

enum Request {
    Draw {
        id: u64,
        t: f32,
        args: Vec<(String, String)>,
        theme: Theme,
    },
    /// `size(args)`를 묻는다.
    Size {
        id: u64,
        args: Vec<(String, String)>,
    },
}

/// 크기 답(요청 번호, 크기).
type SizeReply = (u64, Result<(f32, f32), String>);

/// 크기는 블록 인자마다 한 번 묻는다.
type SizeCache = HashMap<Vec<(String, String)>, Result<(f32, f32), String>>;

struct Reply {
    id: u64,
    shapes: Result<Vec<Shape>, String>,
}

/// 작업 스레드에 띄운 확장. UI 스레드에서만 쓴다.
pub struct Worker {
    pub info: Info,
    requests: Sender<Request>,
    replies: Receiver<Reply>,
    sizes: Receiver<SizeReply>,
    /// 받았지만 아직 가져가지 않은 결과(블록 번호별 최신 것).
    inbox: RefCell<HashMap<u64, Result<Vec<Shape>, String>>>,
    size_cache: RefCell<SizeCache>,
    /// 다음 크기 요청 번호. 늦게 온 옛 답을 걸러 낸다.
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
                        Request::Draw { id, t, args, theme } => {
                            let shapes = extension.draw(t, &args, &theme);
                            reply_tx.send(Reply { id, shapes }).is_ok()
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
            inbox: RefCell::default(),
            size_cache: RefCell::default(),
            next_size: std::cell::Cell::new(1),
        })
    }

    /// 블록 `id`의 `t`초 그림을 요청한다. 결과는 [`Self::take`]로 가져간다.
    pub fn request(&self, id: u64, t: f32, args: &[(String, String)], theme: Theme) {
        self.requests
            .send(Request::Draw {
                id,
                t,
                args: args.to_vec(),
                theme,
            })
            .ok();
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
    pub fn take(&self, id: u64) -> Option<Result<Vec<Shape>, String>> {
        let mut inbox = self.inbox.borrow_mut();
        for reply in self.replies.try_iter() {
            inbox.insert(reply.id, reply.shapes);
        }
        inbox.remove(&id)
    }

    /// 결과를 기다린다(테스트용).
    #[cfg(test)]
    pub fn wait(&self, id: u64) -> Result<Vec<Shape>, String> {
        loop {
            if let Some(shapes) = self.take(id) {
                return shapes;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
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
        worker.request(1, 0.0, &[], Theme::default());
        worker.request(2, 5.0, &[], Theme::default());
        let second = worker.wait(2).unwrap();
        let first = worker.wait(1).unwrap();
        assert_ne!(first, second);
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
