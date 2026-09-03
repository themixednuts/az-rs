use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

/// A blocking, event-driven change source for one local file.
///
/// The parent directory is the durable watch target: log files may be
/// truncated, atomically replaced, or removed and recreated while a follower
/// is attached. A one-slot wake channel coalesces append bursts without losing
/// replacement state.
pub struct FileChangeEvents {
    target: PathBuf,
    wake: crossbeam_channel::Receiver<()>,
    reset: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
    _watcher: RecommendedWatcher,
}

impl FileChangeEvents {
    pub(crate) fn new(path: &Path) -> io::Result<Self> {
        let target = absolute_lexical(path)?;
        let parent = target.parent().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("log path `{}` has no parent directory", target.display()),
            )
        })?;
        let (wake_tx, wake) = crossbeam_channel::bounded(1);
        let reset = Arc::new(AtomicBool::new(false));
        let error = Arc::new(Mutex::new(None));

        let callback_target = target.clone();
        let callback_reset = Arc::clone(&reset);
        let callback_error = Arc::clone(&error);
        let mut watcher =
            notify::recommended_watcher(move |result: notify::Result<Event>| match result {
                Ok(event)
                    if event.need_rescan() || event_mentions_target(&event, &callback_target) =>
                {
                    if event_replaces_file(&event) {
                        callback_reset.store(true, Ordering::Release);
                    }
                    let _ = wake_tx.try_send(());
                }
                Ok(_) => {}
                Err(source) => {
                    callback_reset.store(true, Ordering::Release);
                    let mut slot = callback_error
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    *slot = Some(source.to_string());
                    drop(slot);
                    let _ = wake_tx.try_send(());
                }
            })
            .map_err(notify_io_error)?;
        watcher
            .watch(parent, RecursiveMode::NonRecursive)
            .map_err(notify_io_error)?;

        Ok(Self {
            target,
            wake,
            reset,
            error,
            _watcher: watcher,
        })
    }

    /// Blocks until the target changes. Returns `true` when the caller must
    /// discard its prior byte offset because the path was replaced.
    pub(crate) fn wait(&self) -> io::Result<bool> {
        self.wake.recv().map_err(|_| {
            io::Error::new(
                io::ErrorKind::BrokenPipe,
                format!("log watcher for `{}` closed", self.target.display()),
            )
        })?;
        let reported = self
            .error
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(message) = reported {
            return Err(io::Error::other(format!(
                "log watcher for `{}` failed: {message}",
                self.target.display()
            )));
        }
        Ok(self.reset.swap(false, Ordering::AcqRel))
    }
}

fn absolute_lexical(path: &Path) -> io::Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    Ok(az_filesystem::normalize_lexical(&absolute))
}

fn event_mentions_target(event: &Event, target: &Path) -> bool {
    event
        .paths
        .iter()
        .any(|path| absolute_lexical(path).is_ok_and(|path| paths_equal(&path, target)))
}

fn event_replaces_file(event: &Event) -> bool {
    matches!(
        event.kind,
        EventKind::Create(_)
            | EventKind::Remove(_)
            | EventKind::Modify(notify::event::ModifyKind::Name(_))
    ) || event.need_rescan()
}

#[cfg(windows)]
fn paths_equal(lhs: &Path, rhs: &Path) -> bool {
    lhs.to_string_lossy()
        .eq_ignore_ascii_case(&rhs.to_string_lossy())
}

#[cfg(not(windows))]
fn paths_equal(lhs: &Path, rhs: &Path) -> bool {
    lhs == rhs
}

fn notify_io_error(source: notify::Error) -> io::Error {
    io::Error::other(source)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Write as _;
    use std::time::Duration;

    use super::*;

    #[test]
    fn replacement_events_reset_the_follower_offset() {
        let event = Event {
            kind: EventKind::Create(notify::event::CreateKind::File),
            paths: vec![PathBuf::from("service.log")],
            attrs: notify::event::EventAttributes::new(),
        };
        assert!(event_replaces_file(&event));
    }

    #[test]
    fn data_events_preserve_the_follower_offset() {
        let event = Event {
            kind: EventKind::Modify(notify::event::ModifyKind::Data(
                notify::event::DataChange::Content,
            )),
            paths: vec![PathBuf::from("service.log")],
            attrs: notify::event::EventAttributes::new(),
        };
        assert!(!event_replaces_file(&event));
    }

    #[test]
    fn parent_watch_reports_an_append_without_polling() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("service.log");
        fs::write(&path, b"before\n").unwrap();
        let events = FileChangeEvents::new(&path).unwrap();

        fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"after\n")
            .unwrap();

        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || tx.send(events.wait()).unwrap());
        rx.recv_timeout(Duration::from_secs(5)).unwrap().unwrap();
    }
}
