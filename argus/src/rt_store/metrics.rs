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
    fn new() -> Self {
        Self {
            values: [0.0; MAX_TS_VALUES],
            n_values: 0,
        }
    }

    fn push(&mut self, value: f32) -> Result<(), StoreError> {
        if self.n_values >= MAX_TS_VALUES {
            return Err(StoreError::FullTimeSeries);
        }
        self.values[self.n_values] = value;
        self.n_values += 1;
        Ok(())
    }
}

impl Message {
    pub fn set(&mut self, message: &'static str) {
        *self = Self(message);
    }
}

impl Counter {
    pub fn add(&mut self, value: f32) {
        self.0 += value;
    }
}

impl Gauge {
    pub fn set(&mut self, value: f32) {
        self.0 = value;
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
}

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
