use super::{MetricKind, StoreError};

const MAX_TS_VALUES: usize = 256;
#[derive(Debug, Clone, Copy)]
pub struct TimeSeries {
    values: [f32; MAX_TS_VALUES],
    n_values: usize,
}
#[derive(Default, Debug, Clone, Copy)]
pub struct Message(&'static str);
#[derive(Default, Debug, Clone, Copy)]
pub struct Counter(f32);
#[derive(Default, Debug, Clone, Copy)]
pub struct Gauge(f32);
#[derive(Default, Debug, Clone, Copy)]
pub struct Instant(Option<std::time::Instant>);

impl Default for TimeSeries {
    fn default() -> Self {
        Self::new()
    }
}

impl TimeSeries {
    pub fn new() -> Self {
        Self {
            values: [0.0; MAX_TS_VALUES],
            n_values: 0,
        }
    }

    pub fn push(&mut self, value: f32) -> Result<(), StoreError> {
        if self.n_values >= MAX_TS_VALUES {
            return Err(StoreError::FullTimeSeries);
        }
        self.values[self.n_values] = value;
        self.n_values += 1;
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.n_values
    }

    pub fn is_empty(&self) -> bool {
        self.n_values == 0
    }

    pub fn values(&self) -> &[f32] {
        &self.values
    }
}

impl Message {
    pub fn set(&mut self, message: &'static str) {
        *self = Self(message);
    }
    pub fn value(&self) -> &str {
        self.0
    }
}

impl Counter {
    pub fn add(&mut self, value: f32) {
        self.0 += value;
    }

    pub fn value(&self) -> f32 {
        self.0
    }
}

impl Gauge {
    pub fn set(&mut self, value: f32) {
        self.0 = value;
    }

    pub fn value(&self) -> f32 {
        self.0
    }
}

impl Instant {
    pub fn set(&mut self, instant: std::time::Instant) {
        *self = Self(Some(instant));
    }

    pub fn reset(&mut self) {
        *self = Self(None);
    }

    pub fn is_set(&self) -> bool {
        self.0.is_some()
    }

    pub fn value(&self) -> Option<std::time::Instant> {
        self.0
    }
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Copy)]
pub enum RtMetric {
    Message(Message),
    TimeSeries(TimeSeries),
    Counter(Counter),
    Gauge(Gauge),
    Instant(Instant),
}

impl From<Message> for RtMetric {
    fn from(value: Message) -> Self {
        Self::Message(value)
    }
}
impl From<TimeSeries> for RtMetric {
    fn from(value: TimeSeries) -> Self {
        Self::TimeSeries(value)
    }
}
impl From<Counter> for RtMetric {
    fn from(value: Counter) -> Self {
        Self::Counter(value)
    }
}
impl From<Gauge> for RtMetric {
    fn from(value: Gauge) -> Self {
        Self::Gauge(value)
    }
}
impl From<Instant> for RtMetric {
    fn from(value: Instant) -> Self {
        Self::Instant(value)
    }
}

impl RtMetric {
    pub fn kind(&self) -> MetricKind {
        match self {
            Self::Message(_) => MetricKind::Message,
            Self::TimeSeries(_) => MetricKind::TimeSeries,
            Self::Counter(_) => MetricKind::Counter,
            Self::Gauge(_) => MetricKind::Gauge,
            Self::Instant(_) => MetricKind::Instant,
        }
    }

    pub fn reset(&mut self) {
        match self {
            Self::Message(m) => *m = Message::default(),
            Self::TimeSeries(ts) => *ts = TimeSeries::default(),
            Self::Counter(c) => *c = Counter::default(),
            Self::Gauge(g) => *g = Gauge::default(),
            Self::Instant(i) => *i = Instant::default(),
        }
    }
}
