use tracing::Level;
use tracing_subscriber::{
    EnvFilter, Layer, Registry, fmt::format::FmtSpan, layer::SubscriberExt, util::SubscriberInitExt,
};

#[macro_export]
macro_rules! plot_with_name {
    ($plot_name:expr, $value:expr) => {{
        $crate::Client::running()
            .expect("plot_const! without a running Client")
            .plot($plot_name, $value)
    }};
}

#[cfg(feature = "tracy-profiler")]
tracing_tracy::client::register_demangler!(tracing_tracy::client::demangle::default);

#[cfg(feature = "tracy-profiler")]
#[global_allocator]
static GLOBAL: tracing_tracy::client::ProfiledAllocator<std::alloc::System> =
    tracing_tracy::client::ProfiledAllocator::new(std::alloc::System, 100);

#[derive(Debug, Clone)]
pub struct TracingOptions {
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
            log_level: Level::WARN,
            lines: false,
            file: false,
            pretty_print: false,
            color: false,
            enter_span_events: false,
        }
    }
}

pub fn setup_tracing(args: &TracingOptions) {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or(EnvFilter::new(format!(
        "{},smol=warn,async_io=warn,polling=warn,tokio_tungstenite=warn,tungstenite=warn,reqwest=info,hyper_util=info",
        args.log_level
    )));
    let span_events = if args.enter_span_events {
        FmtSpan::FULL
    } else {
        FmtSpan::NONE
    };

    let stdout_layer = {
        let layer = tracing_subscriber::fmt::layer()
            .without_time()
            .with_level(true)
            .with_ansi(args.color)
            .with_line_number(args.lines)
            .with_span_events(span_events)
            .with_file(args.file);
        let layer = if args.pretty_print {
            layer.pretty().boxed()
        } else {
            layer.compact().boxed()
        };
        layer.with_filter(env_filter)
    };

    // tracy needs to go first - otherwise it somehow inherits with_ansi(true) and that shows weird
    // in the Tracy profiler
    Registry::default()
        .with(tracing_tracy::TracyLayer::default())
        .with(stdout_layer)
        .with(tracing_error::ErrorLayer::default())
        .init();
}
