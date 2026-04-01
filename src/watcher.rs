use std::path::{Path, PathBuf};
use std::time::Duration;

use notify::RecursiveMode;
use notify_debouncer_mini::{new_debouncer, DebouncedEventKind};
use tokio::sync::mpsc;

use crate::events::DevxEvent;

const IGNORED_DIRS: &[&str] = &[
    "node_modules",
    "target",
    ".git",
    "__pycache__",
    ".next",
    ".nuxt",
    "dist",
    ".cache",
];

fn should_ignore(path: &Path) -> bool {
    path.components().any(|c| {
        let s = c.as_os_str().to_str().unwrap_or("");
        IGNORED_DIRS.contains(&s)
    })
}

/// Starts a file watcher for a service directory. Sends `DevxEvent::FileChanged`
/// through the event channel when relevant files are modified.
///
/// The watcher runs in a background thread (notify uses OS-level watchers) and
/// forwards debounced events into the async event channel.
pub fn start_watcher(
    service_name: String,
    watch_dir: PathBuf,
    event_tx: mpsc::Sender<DevxEvent>,
) -> notify::Result<()> {
    let service = service_name.clone();
    let tx = event_tx.clone();

    let mut debouncer = new_debouncer(
        Duration::from_millis(300),
        move |result: Result<Vec<notify_debouncer_mini::DebouncedEvent>, notify::Error>| {
            let events = match result {
                Ok(events) => events,
                Err(_) => return,
            };

            let dominated = events.iter().any(|e| {
                e.kind == DebouncedEventKind::Any && !should_ignore(&e.path)
            });

            if dominated {
                let _ = tx.try_send(DevxEvent::FileChanged {
                    service: service.clone(),
                });
            }
        },
    )?;

    debouncer.watcher().watch(&watch_dir, RecursiveMode::Recursive)?;

    // Leak the debouncer so it lives for the process lifetime.
    // devx services run until the process exits, so there is no
    // meaningful point to stop the watcher earlier.
    std::mem::forget(debouncer);

    Ok(())
}
