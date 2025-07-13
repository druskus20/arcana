use std::{
    collections::HashMap,
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::json;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::sync::mpsc::UnboundedSender;
use tracing::{Event, Subscriber, span};
use tracing_subscriber::layer::Context;
use tracing_tracy::TracyLayer;

// Dashboard TCP Layer - Tracy-style real-time logging
pub struct DashboardTcpLayer {
    sender: UnboundedSender<DashboardEvent>,
    params: DashboardTcpLayerParams,
}

pub struct DashboardTcpLayerParams {
    pub span_events: bool,
}

impl Default for DashboardTcpLayerParams {
    fn default() -> Self {
        Self { span_events: false }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DashboardEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    pub timestamp: u64,
    pub level: Level,
    pub target: String,
    pub message: String,
    pub fields: HashMap<String, String>,
    pub span_id: Option<u64>,
    pub parent_span_id: Option<u64>,
    pub file: Option<String>,
    pub line: Option<u32>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Eq, PartialEq, PartialOrd, Hash)]
pub enum Level {
    TRACE = 0,
    DEBUG,
    INFO,
    WARN,
    ERROR,
}

impl From<&tracing::Level> for Level {
    fn from(level: &tracing::Level) -> Self {
        match level {
            &tracing::Level::TRACE => Level::TRACE,
            &tracing::Level::DEBUG => Level::DEBUG,
            &tracing::Level::INFO => Level::INFO,
            &tracing::Level::WARN => Level::WARN,
            &tracing::Level::ERROR => Level::ERROR,
        }
    }
}

impl ToString for Level {
    fn to_string(&self) -> String {
        match self {
            Level::TRACE => "TRACE".to_string(),
            Level::DEBUG => "DEBUG".to_string(),
            Level::INFO => "INFO".to_string(),
            Level::WARN => "WARN".to_string(),
            Level::ERROR => "ERROR".to_string(),
        }
    }
}
impl DashboardTcpLayer {
    pub fn new(remote_addr: String, params: DashboardTcpLayerParams) -> Self {
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel::<DashboardEvent>();

        tokio::spawn(async move {
            let mut connection_attempts = 0;

            loop {
                match TcpStream::connect(&remote_addr).await {
                    Ok(mut stream) => {
                        println!("Connected to dashboard at {}", remote_addr);
                        connection_attempts = 0;

                        while let Some(event) = receiver.recv().await {
                            let event_json = match serde_json::to_string(&event) {
                                Ok(json) => json,
                                Err(e) => {
                                    eprintln!("Failed to serialize event: {}", e);
                                    continue;
                                }
                            };

                            if let Err(e) = stream
                                .write_all(format!("{}\n", event_json).as_bytes())
                                .await
                            {
                                eprintln!("Failed to write to dashboard TCP stream: {}", e);
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        connection_attempts += 1;
                        eprintln!(
                            "Failed to connect to dashboard (attempt {}): {}",
                            connection_attempts, e
                        );
                        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                    }
                }
            }
        });

        Self { sender, params }
    }
}

impl<S> tracing_subscriber::Layer<S> for DashboardTcpLayer
where
    S: Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let mut visitor = DashboardFieldVisitor::new();
        event.record(&mut visitor);

        // Get current span context
        let span_id = ctx.current_span().id().map(|id| id.into_u64());
        let parent_span_id = event.parent().map(|id| id.into_u64());

        let dashboard_event = DashboardEvent {
            event_type: "log".to_string(),
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
            level: event.metadata().level().into(),
            target: event.metadata().target().to_string(),
            message: visitor.message,
            fields: visitor.fields,
            span_id,
            parent_span_id,
            file: event.metadata().file().map(|s| s.to_string()),
            line: event.metadata().line(),
        };

        // Send non-blocking
        let _ = self.sender.send(dashboard_event);
    }

    fn on_enter(&self, id: &span::Id, ctx: Context<'_, S>) {
        if !self.params.span_events {
            return;
        }
        if let Some(span_ref) = ctx.span(id) {
            let dashboard_event = DashboardEvent {
                event_type: "span_enter".to_string(),
                timestamp: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u64,
                level: Level::TRACE,
                target: span_ref.metadata().target().to_string(),
                message: format!("Entering span: {}", span_ref.metadata().name()),
                fields: HashMap::new(),
                span_id: Some(id.into_u64()),
                parent_span_id: None,
                file: span_ref.metadata().file().map(|s| s.to_string()),
                line: span_ref.metadata().line(),
            };

            let _ = self.sender.send(dashboard_event);
        }
    }

    fn on_exit(&self, id: &span::Id, ctx: Context<'_, S>) {
        if !self.params.span_events {
            return; // Skip exit events if not enabled
        }
        if let Some(span_ref) = ctx.span(id) {
            let dashboard_event = DashboardEvent {
                event_type: "span_exit".to_string(),
                timestamp: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u64,
                level: Level::TRACE,
                target: span_ref.metadata().target().to_string(),
                message: format!("Exiting span: {}", span_ref.metadata().name()),
                fields: HashMap::new(),
                span_id: Some(id.into_u64()),
                parent_span_id: None,
                file: span_ref.metadata().file().map(|s| s.to_string()),
                line: span_ref.metadata().line(),
            };

            let _ = self.sender.send(dashboard_event);
        }
    }
}

struct DashboardFieldVisitor {
    message: String,
    fields: HashMap<String, String>,
}

impl DashboardFieldVisitor {
    fn new() -> Self {
        Self {
            message: String::new(),
            fields: HashMap::new(),
        }
    }
}

impl tracing::field::Visit for DashboardFieldVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{:?}", value).trim_matches('"').to_string();
        } else {
            self.fields
                .insert(field.name().to_string(), format!("{:?}", value));
        }
    }
}
