//! Local, rolling log files with a switchable verbosity.
//!
//! Logs never leave the machine. At the default level they record summaries (scan/clean
//! totals, warnings); in verbose mode every file touched is written, which is the audit trail.

use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Registry, fmt, reload};

const KEEP_LOGS: Duration = Duration::from_secs(14 * 86_400);

pub struct Logging {
    reload: reload::Handle<EnvFilter, Registry>,
    _guard: WorkerGuard,
}

fn directive(verbose: bool) -> &'static str {
    if verbose {
        "info,vexus_core=debug,vexus_app=debug,ui=debug"
    } else {
        "info,wry=warn,tao=warn"
    }
}

impl Logging {
    pub fn init(logs: &Path, verbose: bool) -> Logging {
        prune_old(logs);
        let (filter, reload) = reload::Layer::new(EnvFilter::new(directive(verbose)));
        let (writer, guard) =
            tracing_appender::non_blocking(tracing_appender::rolling::daily(logs, "vexus.log"));
        let console = cfg!(debug_assertions).then(|| fmt::layer().with_writer(std::io::stderr));
        let _ = tracing_subscriber::registry()
            .with(filter)
            .with(fmt::layer().with_writer(writer).with_ansi(false).with_target(true))
            .with(console)
            .try_init();

        let default_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            tracing::error!("panic: {info}");
            default_hook(info);
        }));
        Logging { reload, _guard: guard }
    }

    pub fn set_verbose(&self, verbose: bool) {
        let _ = self.reload.modify(|f| *f = EnvFilter::new(directive(verbose)));
        tracing::info!(verbose, "log verbosity changed");
    }
}

/// Deletes log files older than two weeks (only files this app creates).
fn prune_old(logs: &Path) {
    let Ok(rd) = fs::read_dir(logs) else { return };
    let now = SystemTime::now();
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !name.starts_with("vexus.log") {
            continue;
        }
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .is_ok_and(|m| now.duration_since(m).is_ok_and(|age| age > KEEP_LOGS));
        if old {
            let _ = fs::remove_file(e.path());
        }
    }
}
