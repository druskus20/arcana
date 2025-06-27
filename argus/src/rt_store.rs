/// This module implements a store meant to act as an intermetiate layer between
/// the realtime thread and a monitoring thread.
///
/// The realtime thread can push metrics avoiding allocations and system calls, to a RtStore.
/// The monitoring thread can get a StoreSnapshot from the RtStoreReader, which is a snapshot
/// of the store at the time of reading.
///
/// The RTStore is implemented using a triple buffer to allow for concurrent access. We don't care
/// about losing metrics if the monitoring thread is not fast enough to read the store. This is not
/// a database.
///
/// The RTStore gets allocated only, on creation, at the beginning of the program, and it's metrics
/// are reset every time the monitoring thread reads it.
///
///  ## On the Realtime Safety of std::time::Instant
///
///  This crate is only realtime-safe in unix systems with vDSO support, as it uses
///  `std::time::Instant` with CLOCK_MONOTONIC which takes advantage of the vDSO clock_gettime syscall.
///
//    VDSO will still generate an actual system call for timers other than CLOCK_REALTIME, CLOCK_MONOTONIC,
//    CLOCK_REALTIME_COARSE and CLOCK_MONOTONIC_COARSE (as of Linux 3.13 up to 4.11-rc3).
//
//    `Instant` in Rust uses libc::CLOCK_MONOTONIC
//    See: https://doc.rust-lang.org/nightly/src/std/sys/pal/unix/time.rs.html
use std::{any::type_name, collections::HashMap, time::Instant};
use triple_buffer::Output;
use vdso::Vdso;

#[derive(Debug)]
pub struct RtStoreWriter<const COUNT: usize, Collection: MetricCollection<COUNT>> {
    store: triple_buffer::Input<RtStore<COUNT, Collection>>,
}

#[derive(Debug)]
pub struct RtStoreReader<const COUNT: usize, Collection: MetricCollection<COUNT>> {
    store: Output<RtStore<COUNT, Collection>>,
}

/// A stack-allocated string with a fixed size
//#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
//pub struct Str1024([u8; 1024]);
//
//impl Deref for Str1024 {
//    type Target = [u8; 1024];
//
//    fn deref(&self) -> &Self::Target {
//        &self.0
//    }
//}
//
//impl Str1024 {
//    pub fn empty() -> Self {
//        let arr = [0; 1024];
//        Self(arr)
//    }
//
//    pub fn new(s: &str) -> Self {
//        let mut arr = [0; 1024];
//        let bytes = s.as_bytes();
//        if bytes.len() > 1024 {
//            panic!("String too long for Str1024");
//        }
//        arr[..bytes.len()].copy_from_slice(bytes);
//        Self(arr)
//    }
//
//    pub fn as_str(&self) -> &str {
//        std::str::from_utf8(&self.0).unwrap_or("")
//    }
//}

//const MAX_TAGS: usize = 10;
#[derive(Debug, Clone, Copy)]
pub struct RtMetric {
    pub name: &'static str,
    pub value: RtMetricValue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MetricKind {
    Message,
    Counter,
    Gauge,
    TimeSeries,
    Instant,
}

// TODO: Validate at compile time
// TODO: add some way to skip that validation for dynamic tags and names
impl RtMetric {
    pub fn default_with_name(name: &'static str, kind: MetricKind) -> Self {
        let value = RtMetricValue::default_with_kind(kind);
        Self { name, value }
    }

    pub fn reset_value(&mut self) {
        self.value.reset();
    }
}

#[derive(Debug, Clone)]
pub struct Metric {
    pub name: String,
    //pub tags: HashSet<String>,
    pub value: MetricValue,
}

/// This type has a fixed-size of 1024 bytes
const MAX_TS_VALUES: usize = 256;
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Copy)]
pub enum RtMetricValue {
    /// A message is a string
    Message(&'static str),
    /// A counter is a cumulative metric that represents a single numerical value that only ever goes up
    Counter(f32),
    /// A gauge is a single value that can go up and down
    Gauge(f32),
    /// A single span of values with a start timestamp (- until present)
    TimeSeries {
        values: [f32; MAX_TS_VALUES],
        n_values: usize,
        start_instant: Option<Instant>,
        end_instant: Option<Instant>,
    },
    Instant(Option<Instant>),
}

impl RtMetricValue {
    pub fn default_with_kind(kind: MetricKind) -> Self {
        match kind {
            MetricKind::Message => RtMetricValue::Message(""),
            MetricKind::Counter => RtMetricValue::Counter(0.0),
            MetricKind::Gauge => RtMetricValue::Gauge(0.0),
            MetricKind::TimeSeries => RtMetricValue::TimeSeries {
                values: [0.0; MAX_TS_VALUES],
                n_values: 0,
                start_instant: None,
                end_instant: None,
            },
            MetricKind::Instant => RtMetricValue::Instant(None),
        }
    }
    pub fn kind(&self) -> MetricKind {
        match self {
            RtMetricValue::Message(_) => MetricKind::Message,
            RtMetricValue::Counter(_) => MetricKind::Counter,
            RtMetricValue::Gauge(_) => MetricKind::Gauge,
            RtMetricValue::TimeSeries { .. } => MetricKind::TimeSeries,
            RtMetricValue::Instant(_) => MetricKind::Instant,
        }
    }
    pub fn reset(&mut self) {
        match self {
            RtMetricValue::Message(_) => *self = RtMetricValue::Message(""),
            RtMetricValue::Counter(_) => *self = RtMetricValue::Counter(0.0),
            RtMetricValue::Gauge(_) => *self = RtMetricValue::Gauge(0.0),
            RtMetricValue::TimeSeries {
                values,
                n_values,
                start_instant,
                end_instant,
            } => {
                *values = [0.0; MAX_TS_VALUES];
                *n_values = 0;
                *start_instant = None;
                *end_instant = None;
            }
            RtMetricValue::Instant(_) => *self = RtMetricValue::Instant(None),
        }
    }
}

#[derive(Debug, Clone)]
pub enum MetricValue {
    /// A message is a string
    Message(String),
    /// A counter is a cumulative metric that represents a single numerical value that only ever goes up
    Counter(f64),
    /// A gauge is a single value that can go up and down
    Gauge(f64),
    /// A single span of values with a start timestamp (- until present)
    TimeSeries {
        values: Vec<f32>,
        start_instant: Instant,
        end_instant: Instant,
    },
    Instant(Instant),
}

impl From<&RtMetricValue> for MetricValue {
    fn from(value: &RtMetricValue) -> Self {
        match value {
            RtMetricValue::Message(s) => MetricValue::Message(s.to_string()),
            RtMetricValue::Counter(c) => MetricValue::Counter(*c as f64),
            RtMetricValue::Gauge(g) => MetricValue::Gauge(*g as f64),
            RtMetricValue::TimeSeries {
                values,
                n_values,
                start_instant,
                end_instant,
            } => MetricValue::TimeSeries {
                values: values[..*n_values].to_vec(),
                start_instant: start_instant
                    .expect("Timeseries start instant should not be None at this point"),
                end_instant: end_instant
                    .expect("Timeseries end instant should not be None at this point"),
            },
            RtMetricValue::Instant(ts) => {
                MetricValue::Instant(ts.expect("Instant should not be None at this point"))
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("Failed to acquire lock on store")]
    LockError,
    #[error("Metric with the same name already exists, and it's not a timeseries")]
    MetricAlreadyExists,
    #[error("Metric not found")]
    MetricNotFound,
    #[error("Metric type mismatch, expected: {expected}, found: {found}")]
    MetricTypeMismatch { expected: String, found: String },
    #[error("Full timeseries, cannot push more values")]
    FullTimeSeries,
}

pub trait MetricCollection<const COUNT: usize>: Send {
    fn get_index(name: &str) -> Option<usize>;
    fn get_name(index: usize) -> &'static str;
}

#[derive(Debug, Clone)]
struct RtStore<const COUNT: usize, Collection: MetricCollection<COUNT>> {
    phantom: std::marker::PhantomData<Collection>,
    metrics: [RtMetric; COUNT],
}

fn is_vdso_clock_gettime_available() -> bool {
    #[cfg(not(target_os = "linux"))]
    return false;

    //    VDSO will still generate an actual system call for timers other than CLOCK_REALTIME, CLOCK_MONOTONIC,
    //    CLOCK_REALTIME_COARSE and CLOCK_MONOTONIC_COARSE (as of Linux 3.13 up to 4.11-rc3).
    //
    //    `Instant` in Rust uses libc::CLOCK_MONOTONIC
    //    See: https://doc.rust-lang.org/nightly/src/std/sys/pal/unix/time.rs.html

    #[cfg(target_os = "linux")]
    return Vdso::locate()
        .and_then(|x| x.lookup("__vdso_clock_gettime"))
        .is_some();
}

impl<const COUNT: usize, C: MetricCollection<COUNT> + Clone> RtStore<COUNT, C> {
    pub fn init(metrics_shape: [(&'static str, MetricKind); COUNT]) -> Self {
        if !is_vdso_clock_gettime_available() {
            panic!(
                "vDSO clock_gettime not found, this is required for the realtime store to work 
                as it uses Rust's Instant type which relies on it for realtime safety."
            );
        }

        let metrics = {
            let mut metrics = [RtMetric {
                name: "",
                value: RtMetricValue::Message(""),
            }; COUNT];

            for (i, (name, kind)) in metrics_shape.iter().enumerate() {
                metrics[i] = RtMetric::default_with_name(name, *kind);
            }

            metrics
        };

        Self {
            metrics,
            phantom: std::marker::PhantomData,
        }
    }

    pub fn split(
        metrics_shape: [(&'static str, MetricKind); COUNT],
    ) -> (RtStoreWriter<COUNT, C>, RtStoreReader<COUNT, C>) {
        let (producer, consumer) = triple_buffer::triple_buffer(&RtStore::init(metrics_shape));

        (
            RtStoreWriter { store: producer },
            RtStoreReader { store: consumer },
        )
    }

    // This method should be called at the beginning of the real time loop. It resets the values of
    // all metrics and sets the start timestamp for time series metrics.
    pub fn enter(&mut self) {
        for metric in self.metrics.iter_mut() {
            metric.reset_value();
            if let RtMetricValue::TimeSeries { start_instant, .. } = &mut metric.value {
                start_instant.replace(Instant::now());
            }
        }
    }

    /// This method should be called at the end of the real time loop. It updates the end timestamp
    /// of the current batch of metrics.
    pub fn exit(&mut self) {
        for metric in self.metrics.iter_mut() {
            if let RtMetricValue::TimeSeries { end_instant, .. } = &mut metric.value {
                end_instant.replace(Instant::now());
            }
        }
    }
}

impl<const COUNT: usize, C: MetricCollection<COUNT> + Clone> RtStoreWriter<COUNT, C> {
    pub fn push_value_to_ts(
        &mut self,
        metric_name: &'static str,
        value: f32,
    ) -> Result<(), StoreError> {
        let buffer = self.store.input_buffer_mut();
        let index = C::get_index(metric_name).ok_or(StoreError::MetricNotFound)?;

        let metric = &mut buffer.metrics[index];
        if let RtMetricValue::TimeSeries {
            values, n_values, ..
        } = &mut metric.value
        {
            if *n_values >= MAX_TS_VALUES {
                return Err(StoreError::FullTimeSeries);
            }
            values[*n_values] = value;
            *n_values += 1;
        } else {
            return Err(StoreError::MetricTypeMismatch {
                expected: "TimeSeries".to_string(),
                found: type_name::<RtMetricValue>().to_string(),
            });
        };

        Ok(())
    }

    pub fn set_message(
        &mut self,
        metric_name: &'static str,
        message: &'static str,
    ) -> Result<(), StoreError> {
        let buffer = self.store.input_buffer_mut();
        let index = C::get_index(metric_name).ok_or(StoreError::MetricNotFound)?;

        let metric = &mut buffer.metrics[index];
        if let RtMetricValue::Message(_) = &mut metric.value {
            metric.value = RtMetricValue::Message(message);
            Ok(())
        } else {
            Err(StoreError::MetricTypeMismatch {
                expected: "Message".to_string(),
                found: type_name::<RtMetricValue>().to_string(),
            })
        }
    }

    pub fn set_counter(&mut self, metric_name: &'static str, value: f32) -> Result<(), StoreError> {
        let buffer = self.store.input_buffer_mut();
        let index = C::get_index(metric_name).ok_or(StoreError::MetricNotFound)?;

        let metric = &mut buffer.metrics[index];
        if let RtMetricValue::Counter(_) = &mut metric.value {
            metric.value = RtMetricValue::Counter(value);
            Ok(())
        } else {
            Err(StoreError::MetricTypeMismatch {
                expected: "Counter".to_string(),
                found: type_name::<RtMetricValue>().to_string(),
            })
        }
    }

    pub fn set_gauge(&mut self, metric_name: &'static str, value: f32) -> Result<(), StoreError> {
        let buffer = self.store.input_buffer_mut();
        let index = C::get_index(metric_name).ok_or(StoreError::MetricNotFound)?;

        let metric = &mut buffer.metrics[index];
        if let RtMetricValue::Gauge(_) = &mut metric.value {
            metric.value = RtMetricValue::Gauge(value);
            Ok(())
        } else {
            Err(StoreError::MetricTypeMismatch {
                expected: "Gauge".to_string(),
                found: type_name::<RtMetricValue>().to_string(),
            })
        }
    }

    pub fn set_instant(
        &mut self,
        metric_name: &'static str,
        instant: Instant,
    ) -> Result<(), StoreError> {
        let buffer = self.store.input_buffer_mut();
        let index = C::get_index(metric_name).ok_or(StoreError::MetricNotFound)?;

        let metric = &mut buffer.metrics[index];
        if let RtMetricValue::Instant(_) = &mut metric.value {
            metric.value = RtMetricValue::Instant(Some(instant));
            Ok(())
        } else {
            Err(StoreError::MetricTypeMismatch {
                expected: "Instant".to_string(),
                found: type_name::<RtMetricValue>().to_string(),
            })
        }
    }

    /// This method should be called at the beginning of the real time loop. It resets the values of
    ///
    pub fn enter(&mut self) {
        let buffer = self.store.input_buffer_mut();
        buffer.enter();
    }

    /// This method should be called at the end of the real time loop. It updates the end timestamp
    /// of the current batch of metrics. And then publishes the buffer to the reader.
    pub fn exit(&mut self) {
        let buffer = self.store.input_buffer_mut();
        buffer.exit();
        let _has_buffer_been_overwritten = self.store.publish();
    }
}

impl<const COUNT: usize, C: MetricCollection<COUNT>> RtStoreReader<COUNT, C> {
    pub fn read_snapshot(&mut self) -> StoreSnapshot {
        let rt_store = self.store.read();
        StoreSnapshot::from(rt_store)
    }
}

/// Non realtime safe version of the store, which is implemented with heap allocated types for
/// better ergonomics
#[derive(Debug, Clone)]
pub struct StoreSnapshot {
    metrics: HashMap<String, Metric>,
}

impl<const COUNT: usize, C: MetricCollection<COUNT>> From<&RtStore<COUNT, C>> for StoreSnapshot {
    fn from(value: &RtStore<COUNT, C>) -> Self {
        let metrics = value
            .metrics
            .into_iter()
            .map(|rt_metric| {
                // allocate name
                let name = rt_metric.name.to_string();
                // allocate tags
                //let tags = rt_metric.tags.iter().map(|tag| tag.to_string()).collect();
                // allocate value
                let value = MetricValue::from(&rt_metric.value);
                (name.clone(), Metric { name, value })
            })
            .collect();

        StoreSnapshot { metrics }
    }
}

impl StoreSnapshot {
    pub fn query_by_name(&self, name: &str) -> Result<Metric, StoreError> {
        self.metrics
            .get(name)
            .ok_or(StoreError::MetricNotFound)
            .cloned()
    }

    /// Retrieves a snapshot of the store
    pub fn query_with_filter(
        &self,
        filter: impl MetricFilter,
    ) -> Result<HashMap<String, Metric>, StoreError> {
        let r = self
            .metrics
            .iter()
            .filter_map(|(name, metric)| {
                if filter.matches(metric) {
                    Some((name.clone(), metric.clone()))
                } else {
                    None
                }
            })
            .collect();

        Ok(r)
    }
}
pub trait MetricFilter {
    fn matches(&self, metric: &Metric) -> bool;
}

pub struct NameFilter {
    name: String,
}

impl MetricFilter for NameFilter {
    fn matches(&self, metric: &Metric) -> bool {
        metric.name == self.name
    }
}

//pub struct TagFilter {
//    tags: HashSet<String>,
//    mode: TagFilterMode,
//}
//
//impl TagFilter {
//    pub fn any_of(tags: HashSet<String>) -> Self {
//        TagFilter {
//            tags,
//            mode: TagFilterMode::Any,
//        }
//    }
//
//    pub fn all_of(tags: HashSet<String>) -> Self {
//        TagFilter {
//            tags,
//            mode: TagFilterMode::All,
//        }
//    }
//}
//
//pub enum TagFilterMode {
//    All,
//    Any,
//}
//
//impl MetricFilter for TagFilter {
//    fn matches(&self, metric: &Metric) -> bool {
//        match self.mode {
//            TagFilterMode::All => self.tags.is_subset(&metric.tags),
//            TagFilterMode::Any => self.tags.is_disjoint(&metric.tags),
//        }
//    }
//}
