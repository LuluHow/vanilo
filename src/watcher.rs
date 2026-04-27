use std::path::Path;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};

const DEBOUNCE_MS: u64 = 1000;

const WATCH_DIRS: &[&str] = &[
    "pages",
    "components",
    "static",
    "functions",
    "content",
];

const WATCH_FILES: &[&str] = &[
    "layout.html",
    "vanilo.toml",
];

/// Watches source directories for changes and triggers rebuilds.
/// Runs in a loop — call from a dedicated thread.
pub fn watch(build_lock: Arc<Mutex<()>>) -> Result<(), String> {
    let (tx, rx) = mpsc::channel();

    let mut watcher = RecommendedWatcher::new(
        move |res: Result<notify::Event, notify::Error>| {
            if let Ok(event) = res {
                if event.kind.is_modify() || event.kind.is_create() || event.kind.is_remove() {
                    let _ = tx.send(());
                }
            }
        },
        Config::default(),
    )
    .map_err(|e| format!("watcher init: {e}"))?;

    for dir in WATCH_DIRS {
        let path = Path::new(dir);
        if path.is_dir() {
            watcher
                .watch(path, RecursiveMode::Recursive)
                .map_err(|e| format!("watch {dir}: {e}"))?;
        }
    }
    for file in WATCH_FILES {
        let path = Path::new(file);
        if path.exists() {
            watcher
                .watch(path, RecursiveMode::NonRecursive)
                .map_err(|e| format!("watch {file}: {e}"))?;
        }
    }

    println!("watching for changes...");

    loop {
        // Block until first event
        match rx.recv() {
            Ok(()) => {}
            Err(_) => return Ok(()), // channel closed
        }

        // Debounce: drain events for DEBOUNCE_MS
        loop {
            match rx.recv_timeout(Duration::from_millis(DEBOUNCE_MS)) {
                Ok(()) => continue,
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return Ok(()),
            }
        }

        // Try to acquire lock; skip if a build is already running
        let _guard = match build_lock.try_lock() {
            Ok(g) => g,
            Err(_) => {
                eprintln!("rebuild already in progress, skipping");
                continue;
            }
        };

        let start = Instant::now();
        match crate::builder::build() {
            Ok(()) => {
                let ms = start.elapsed().as_millis();
                println!("rebuilt in {ms}ms");
            }
            Err(e) => {
                eprintln!("rebuild error: {e}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc as std_mpsc;
    use std::thread;

    /// Helper: simulates the debounce loop logic without real file watching.
    /// Returns the number of rebuild triggers for a given sequence of events.
    fn count_rebuilds(events: Vec<Duration>) -> usize {
        let (tx, rx) = std_mpsc::channel::<()>();
        let (result_tx, result_rx) = std_mpsc::channel::<usize>();

        // Consumer thread: debounce loop
        thread::spawn(move || {
            let mut rebuilds = 0;
            loop {
                match rx.recv_timeout(Duration::from_millis(500)) {
                    Ok(()) => {}
                    Err(RecvTimeoutError::Timeout) => {
                        let _ = result_tx.send(rebuilds);
                        return;
                    }
                    Err(RecvTimeoutError::Disconnected) => {
                        let _ = result_tx.send(rebuilds);
                        return;
                    }
                }
                // Debounce drain
                loop {
                    match rx.recv_timeout(Duration::from_millis(DEBOUNCE_MS)) {
                        Ok(()) => continue,
                        Err(_) => break,
                    }
                }
                rebuilds += 1;
            }
        });

        // Producer thread: send events at specified delays
        thread::spawn(move || {
            for delay in events {
                thread::sleep(delay);
                let _ = tx.send(());
            }
        });

        result_rx.recv_timeout(Duration::from_secs(5)).unwrap_or(0)
    }

    #[test]
    fn debounce_coalesces_events() {
        // 10 rapid events (no delay between them) should produce 1 rebuild
        let events = vec![Duration::ZERO; 10];
        assert_eq!(count_rebuilds(events), 1);
    }

    #[test]
    fn debounce_separate_batches() {
        // Two events separated by more than the debounce window
        let events = vec![
            Duration::ZERO,
            Duration::from_millis(DEBOUNCE_MS + 200),
        ];
        assert_eq!(count_rebuilds(events), 2);
    }

    #[test]
    fn skip_if_locked() {
        let lock = Arc::new(Mutex::new(()));
        // Hold the lock
        let _guard = lock.lock().unwrap();
        // try_lock should fail
        assert!(lock.try_lock().is_err());
    }

    #[test]
    fn watches_correct_dirs() {
        // Verify our constants contain the expected directories
        assert!(WATCH_DIRS.contains(&"pages"));
        assert!(WATCH_DIRS.contains(&"components"));
        assert!(WATCH_DIRS.contains(&"static"));
        assert!(WATCH_DIRS.contains(&"functions"));
        assert!(WATCH_DIRS.contains(&"content"));
        assert!(WATCH_FILES.contains(&"layout.html"));
        assert!(WATCH_FILES.contains(&"vanilo.toml"));
    }
}
