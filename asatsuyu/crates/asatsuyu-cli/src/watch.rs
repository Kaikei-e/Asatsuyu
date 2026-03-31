//! Watch mode for `asatsuyu check --watch`.
//!
//! Watches source files for changes, debounces filesystem events, and re-runs
//! the check pipeline. Clears the terminal between cycles (human mode only) and
//! shows a status message after each check.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::exit_config_error;
use std::sync::mpsc;
use std::time::Duration;

use notify_debouncer_mini::{DebouncedEvent, DebouncedEventKind, new_debouncer};

use asatsuyu_hir::ffi::FfiResolverConfig;

use crate::ErrorFormat;

/// Debounce interval for filesystem events.
const DEBOUNCE_MS: u64 = 250;

/// Run check in watch mode: initial check, then re-check on file changes.
///
/// This function blocks until the watcher channel closes (typically on Ctrl-C).
pub(crate) fn run_watch(
    paths: &[PathBuf],
    ffi_config: &FfiResolverConfig,
    error_format: ErrorFormat,
) -> ExitCode {
    // 1. Initial check.
    clear_terminal(error_format);
    print_watch_header(error_format);
    let status = crate::cmd_check(paths, ffi_config, error_format);
    print_watch_footer(status, error_format);

    // 2. Determine directories to watch.
    let watch_dirs = compute_watch_dirs(paths);

    // 3. Set up debounced file watcher.
    let (tx, rx) = mpsc::channel();
    let debounce_duration = Duration::from_millis(DEBOUNCE_MS);
    let mut debouncer = match new_debouncer(debounce_duration, tx) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: cannot start file watcher: {e}");
            return exit_config_error();
        }
    };

    for dir in &watch_dirs {
        if let Err(e) = debouncer.watcher().watch(dir, notify::RecursiveMode::Recursive) {
            eprintln!("error: cannot watch {}: {e}", dir.display());
            return exit_config_error();
        }
    }

    // 4. Event loop.
    loop {
        match rx.recv() {
            Ok(Ok(events)) => {
                // Re-check on both normal and continuous debounced writes to `.asty` files.
                let asty_changed = events.iter().any(should_recheck);
                if !asty_changed {
                    continue;
                }

                clear_terminal(error_format);
                print_watch_header(error_format);
                let status = crate::cmd_check(paths, ffi_config, error_format);
                print_watch_footer(status, error_format);
            }
            Ok(Err(errors)) => {
                eprintln!("watch error: {errors:?}");
            }
            Err(_) => {
                // Channel closed — watcher dropped. Exit cleanly.
                break;
            }
        }
    }

    ExitCode::SUCCESS
}

fn should_recheck(event: &DebouncedEvent) -> bool {
    matches!(event.kind, DebouncedEventKind::Any | DebouncedEventKind::AnyContinuous)
        && event.path.extension().is_some_and(|ext| ext == "asty")
}

/// Compute the set of directories to watch.
///
/// Deduplicates by taking parent directories of explicit file paths.
fn compute_watch_dirs(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for path in paths {
        let dir = if path.is_file() || path.extension().is_some_and(|ext| ext == "asty") {
            path.parent().map_or_else(|| Path::new(".").to_path_buf(), Path::to_path_buf)
        } else {
            path.clone()
        };
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    if dirs.is_empty() {
        dirs.push(PathBuf::from("."));
    }
    dirs
}

/// Clear the terminal screen (human mode only).
fn clear_terminal(error_format: ErrorFormat) {
    if matches!(error_format, ErrorFormat::Human) {
        // ANSI escape: clear screen + move cursor to top-left.
        eprint!("\x1b[2J\x1b[H");
    }
}

/// Print the header at the start of each watch cycle.
fn print_watch_header(error_format: ErrorFormat) {
    if matches!(error_format, ErrorFormat::Json) {
        return;
    }
    eprintln!("Checking...\n");
}

/// Print the footer after each watch cycle.
fn print_watch_footer(status: ExitCode, error_format: ErrorFormat) {
    if matches!(error_format, ErrorFormat::Json) {
        return;
    }
    if status == ExitCode::SUCCESS {
        eprintln!("\nNo errors found.");
    }
    eprintln!("\nWatching for file changes... (press Ctrl+C to exit)");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_recheck_for_asty_any_event() {
        let event =
            DebouncedEvent { path: PathBuf::from("src/main.asty"), kind: DebouncedEventKind::Any };
        assert!(should_recheck(&event));
    }

    #[test]
    fn should_recheck_for_asty_any_continuous_event() {
        let event = DebouncedEvent {
            path: PathBuf::from("src/main.asty"),
            kind: DebouncedEventKind::AnyContinuous,
        };
        assert!(should_recheck(&event));
    }

    #[test]
    fn should_not_recheck_for_non_asty_file() {
        let event =
            DebouncedEvent { path: PathBuf::from("src/main.py"), kind: DebouncedEventKind::Any };
        assert!(!should_recheck(&event));
    }

    #[test]
    fn compute_watch_dirs_deduplicates_parent_dirs() {
        let dirs = compute_watch_dirs(&[
            PathBuf::from("src/main.asty"),
            PathBuf::from("src/lib.asty"),
            PathBuf::from("examples/demo.asty"),
        ]);

        assert_eq!(dirs, vec![PathBuf::from("src"), PathBuf::from("examples")]);
    }
}
