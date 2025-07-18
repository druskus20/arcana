use std::path::PathBuf;

use oculus::DashboardTcpLayerParams;
use tracing::Level;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{
    EnvFilter, Layer as _, Registry,
    fmt::{self, format::FmtSpan},
    layer::SubscriberExt,
    util::SubscriberInitExt,
};
use tracing_tracy::TracyLayer;

#[cfg(feature = "oculus")]
pub mod oculus;

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

    // Occulus (custom dashboard)
    pub oculus_layer: bool,

    // Error layer
    pub error_layer: bool,

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
            oculus_layer: false,
            error_layer: true,
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
    setup_tracing_with_filter(args, None)
}

pub fn setup_tracing_with_filter(
    args: &TracingOptions,
    env_filter_str: Option<String>,
) -> WorkerGuard {
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

        let env_filter = EnvFilter::try_from_default_env().unwrap_or(
            env_filter_str
                .map(EnvFilter::new)
                .unwrap_or(EnvFilter::new(format!(
                    "{},smol=warn,async_io=warn,polling=warn,\
                    tokio_tungstenite=warn,tungstenite=warn,\
                    reqwest=info,hyper_util=info",
                    args.log_level
                ))),
        );

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
        #[cfg(not(feature = "tracy-profiler"))]
        panic!("tracy-profiler feature is not enabled, but tracy_layer is set to true");
        Some(TracyLayer::default())
    } else {
        None
    };

    let error = if args.error_layer {
        Some(tracing_error::ErrorLayer::default())
    } else {
        None
    };

    let oculus = {
        #[cfg(feature = "oculus")]
        if args.oculus_layer {
            Some(oculus::DashboardTcpLayer::new(
                "127.0.0.1:8080".to_string(),
                DashboardTcpLayerParams::default(),
            ))
        } else {
            None
        }
        #[cfg(not(feature = "oculus"))]
        if args.oculus_layer {
            panic!("oculus feature is not enabled, but oculus_layer is set to true");
        } else {
            Option::<()>::None
        }
    };

    #[cfg(feature = "oculus")]
    Registry::default()
        // tracy needs to go first - otherwise it somehow inherits with_ansi(true) and that shows weird
        // in the Tracy profiler
        .with(tracy)
        .with(base_layer)
        .with(error)
        .with(oculus)
        .init();

    #[cfg(not(feature = "oculus"))]
    Registry::default()
        // tracy needs to go first - otherwise it somehow inherits with_ansi(true) and that shows weird
        // in the Tracy profiler
        .with(tracy)
        .with(base_layer)
        .with(error)
        .init();

    guard
}
