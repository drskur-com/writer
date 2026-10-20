//! 라이브러리 폴더 감시.
//!
//! 라이브러리 루트를 재귀로 감시하고, 변경이 생기면 신호(`()`)만 보낸다.
//! 무엇이 바뀌었는지는 따지지 않는다. 받는 쪽이 짧게 모았다가(디바운스) 다시 스캔한다.

use futures::channel::mpsc::{UnboundedReceiver, unbounded};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::Path;

use crate::library::MARKER_DIR;

/// `root` 감시를 시작한다. 반환한 watcher를 버리면 감시가 멈춘다.
pub fn watch(root: &Path) -> notify::Result<(RecommendedWatcher, UnboundedReceiver<()>)> {
    let (tx, rx) = unbounded();
    // 앱 내부 상태(.writer/)의 변경은 목록과 무관하므로 무시한다.
    // 이벤트 경로는 정규화된 경로(macOS: /private/var …)로 오므로 같은 형태로 비교한다.
    let internal = root
        .canonicalize()
        .unwrap_or_else(|_| root.to_path_buf())
        .join(MARKER_DIR);
    let mut watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
        let Ok(event) = result else {
            return;
        };
        if matches!(event.kind, EventKind::Access(_)) {
            return;
        }
        if !event.paths.is_empty() && event.paths.iter().all(|path| path.starts_with(&internal)) {
            return;
        }
        let _ = tx.unbounded_send(());
    })?;
    watcher.watch(root, RecursiveMode::Recursive)?;
    Ok((watcher, rx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// 신호가 올 때까지 최대 `timeout` 동안 기다린다.
    fn wait_signal(rx: &mut UnboundedReceiver<()>, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if rx.try_recv().is_ok() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    #[test]
    fn signals_when_sheet_is_deleted() {
        let root = std::env::temp_dir().join(format!("writer-watch-{}", std::process::id()));
        let group = root.join("Notes").join("Inbox");
        std::fs::create_dir_all(&group).unwrap();
        let sheet = group.join("a.md");
        std::fs::write(&sheet, "a").unwrap();
        // macOS(/var → /private/var)에서 이벤트 경로와 맞추려고 정규화한다.
        let root = root.canonicalize().unwrap();

        let (_watcher, mut rx) = watch(&root).unwrap();
        // 감시 시작 직후의 잔여 이벤트를 비운다.
        std::thread::sleep(Duration::from_millis(300));
        while rx.try_recv().is_ok() {}

        std::fs::remove_file(root.join("Notes").join("Inbox").join("a.md")).unwrap();
        assert!(wait_signal(&mut rx, Duration::from_secs(5)));

        std::fs::remove_dir_all(&root).ok();
    }
}
