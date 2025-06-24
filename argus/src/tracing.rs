use std::path::PathBuf;

use tracing::Level;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{
    EnvFilter, Layer as _, Registry,
    fmt::{self, format::FmtSpan},
    layer::SubscriberExt,
    util::SubscriberInitExt,
};
use tracing_tracy::TracyLayer;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Output {
    Stdout,
    Stderr,
    File(PathBuf),
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Output::Stdout => write!(f, "stdout"),
            Output::Stderr => write!(f, "stderr"),
            Output::File(path) => write!(f, "{}", path.display()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TracingOptions {
    // Tracy
    pub tracy_layer: bool,

    // Base
    pub output: Output,
    pub log_level: Level,
    pub lines: bool,
    pub file: bool,
    pub pretty_print: bool,
    pub color: bool,
    pub enter_span_events: bool,
}

impl Default for TracingOptions {
    fn default() -> Self {
        Self {
            output: Output::Stdout,
            tracy_layer: false,
            log_level: Level::WARN,
            lines: false,
            file: false,
            pretty_print: false,
            color: false,
            enter_span_events: false,
        }
    }
}

pub fn setup_tracing(args: &TracingOptions) -> WorkerGuard {
    let (writer, guard) = match args.output.clone() {
        Output::Stdout => tracing_appender::non_blocking(std::io::stdout()),
        Output::Stderr => tracing_appender::non_blocking(std::io::stderr()),
        Output::File(path) => {
            let file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .expect("Failed to open log file");
            tracing_appender::non_blocking(file)
        }
    };

    let base_layer = {
        let span_events = if args.enter_span_events {
            FmtSpan::FULL
        } else {
            FmtSpan::NONE
        };

        let env_filter = EnvFilter::try_from_default_env().unwrap_or(EnvFilter::new(
            format!("{},smol=warn,async_io=warn,polling=warn,tokio_tungstenite=warn,tungstenite=warn,reqwest=info,hyper_util=info", args.log_level)
        ));

        let layer = fmt::Layer::default()
            .with_writer(writer)
            .without_time()
            .with_level(true)
            .with_ansi(args.color)
            .with_line_number(args.lines)
            .with_span_events(span_events)
            .with_file(args.file);

        // Apply pretty/compact formatting
        let layer = if args.pretty_print {
            layer.pretty().boxed()
        } else {
            layer.compact().boxed()
        };

        layer.with_filter(env_filter)
    };

    let tracy = if args.tracy_layer {
        Some(TracyLayer::default())
    } else {
        None
    };

    Registry::default()
        // tracy needs to go first - otherwise it somehow inherits with_ansi(true) and that shows weird
        // in the Tracy profiler
        .with(tracy)
        .with(base_layer)
        .with(tracing_error::ErrorLayer::default())
        .init();

    guard
}
