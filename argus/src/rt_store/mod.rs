use metrics::RtMetric;
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
use std::{any::type_name, collections::HashMap};
use triple_buffer::Output;
use vdso::Vdso;

pub mod metrics;

#[derive(Debug, Clone)]
pub struct RtStore<M> {
    pub initialized: bool,
    pub metrics: M,
    pub start_instant: Option<std::time::Instant>,
    pub end_instant: Option<std::time::Instant>,
}

#[derive(Debug)]
pub struct RtStoreWriter<M: Send> {
    store: triple_buffer::Input<RtStore<M>>,
}

#[derive(Debug)]
pub struct StoreReader<M: Send> {
    store: Output<RtStore<M>>,
    // Trick to detect if the store has been read already this iteration
    last_read_start_instant: Option<std::time::Instant>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MetricKind {
    Message,
    Counter,
    Gauge,
    TimeSeries,
    Instant,
}

#[derive(Debug, Clone)]
pub struct Metric {
    pub name: String,
    pub value: MetricValue,
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
        start_instant: std::time::Instant,
        end_instant: std::time::Instant,
    },
    Instant(std::time::Instant),
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("Failed to acquire lock on store")]
    LockError,
    #[error("Full timeseries, cannot push more values")]
    FullTimeSeries,
    #[error("Store has not been initialized yet")]
    StoreNotInitialized,
    #[error("Store has been already read")]
    StoreAlreadyRead,
}

pub trait MetricCollection
where
    Self: Sized,
{
    fn iter_mut(&mut self) -> impl Iterator<Item = RtMetric>;
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

impl<M: MetricCollection + Clone + Send + Default> RtStore<M> {
    pub fn init() -> Self {
        if !is_vdso_clock_gettime_available() {
            panic!(
                "vDSO clock_gettime not found, this is required for the realtime store to work 
                as it uses Rust's Instant type which relies on it for realtime safety."
            );
        }

        Self {
            initialized: false,
            metrics: M::default(),
            start_instant: None,
            end_instant: None,
        }
    }

    pub fn split(self) -> (RtStoreWriter<M>, StoreReader<M>) {
        let (producer, consumer) = triple_buffer::triple_buffer(&self);

        (
            RtStoreWriter { store: producer },
            StoreReader {
                store: consumer,
                last_read_start_instant: None,
            },
        )
    }
}

impl<M: MetricCollection> RtStore<M> {
    pub fn reset(&mut self) {
        self.initialized = false;
        self.start_instant = None;
        self.end_instant = None;
        for mut metric in self.metrics.iter_mut() {
            metric.reset();
        }
    }

    pub fn enter(&mut self) {
        self.start_instant = Some(std::time::Instant::now());
    }

    /// This method should be called at the end of the real time loop. It updates the end timestamp
    /// of the current batch of metrics.
    pub fn exit(&mut self) {
        self.end_instant = Some(std::time::Instant::now());
        self.initialized = true;
    }
}

impl<M: MetricCollection + Send> RtStoreWriter<M> {
    pub fn inner(&mut self) -> &mut M {
        &mut self.store.input_buffer_mut().metrics
    }

    /// This method should be called at the beginning of the real time loop. It resets the values of
    pub fn reset_and_enter(&mut self) {
        let buffer = self.store.input_buffer_mut();
        buffer.reset();
        buffer.enter();
    }

    /// This method should be called at the end of the real time loop. It updates the end timestamp
    /// of the current batch of metrics. And then publishes the buffer to the reader.
    pub fn exit_and_publish(&mut self) {
        let buffer = self.store.input_buffer_mut();
        buffer.exit();
        let _has_buffer_been_overwritten = self.store.publish();
    }
}

impl<M: MetricCollection + Send + Clone> StoreReader<M> {
    pub fn read(&mut self) -> Result<&RtStore<M>, StoreError> {
        let store = self.store.read();
        if !store.initialized {
            Err(StoreError::StoreNotInitialized)
        } else if self.last_read_start_instant.is_some()
            && self.last_read_start_instant == store.start_instant
        {
            Err(StoreError::StoreAlreadyRead)
        } else {
            self.last_read_start_instant = store.start_instant;
            Ok(store)
        }
    }
}
