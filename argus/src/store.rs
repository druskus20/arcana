use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

#[derive(Debug, Clone)]
pub struct ArcStore {
    store: Arc<Mutex<StoreInner>>,
}

#[derive(Debug)]
struct StoreInner {
    metrics: HashMap<String, Metric>,
}

#[derive(Debug, Clone)]
pub struct Metric {
    pub name: String,
    pub tags: HashSet<String>,
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
        values: Vec<f64>,
        start_ts: chrono::DateTime<chrono::Utc>,
    },
    Timestamp(chrono::DateTime<chrono::Utc>),
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("Failed to acquire lock on store")]
    LockError,
    #[error("Metric with the same name already exists")]
    MetricAlreadyExists,
    #[error("Metric not found")]
    MetricNotFound,
    #[error("Metric type mismatch, expected: {expected}, found: {found}")]
    MetricTypeMismatch { expected: String, found: String },
}

impl ArcStore {
    pub fn new() -> Self {
        ArcStore {
            store: Arc::new(Mutex::new(StoreInner {
                metrics: HashMap::new(),
            })),
        }
    }

    fn push_metric(&self, metric: Metric) -> Result<(), StoreError> {
        let mut inner = self.store.lock().map_err(|_| StoreError::LockError)?;
        if inner.metrics.contains_key(&metric.name) {
            return Err(StoreError::MetricAlreadyExists);
        }
        inner.metrics.insert(metric.name.clone(), metric);
        Ok(())
    }
    pub fn push_message(
        &self,
        name: &str,
        tags: HashSet<String>,
        message: String,
    ) -> Result<(), StoreError> {
        let metric = Metric {
            name: name.to_string(),
            tags,
            value: MetricValue::Message(message),
        };
        self.push_metric(metric)
    }
    pub fn push_counter(
        &self,
        name: &str,
        tags: HashSet<String>,
        value: f64,
    ) -> Result<(), StoreError> {
        let metric = Metric {
            name: name.to_string(),
            tags,
            value: MetricValue::Counter(value),
        };
        self.push_metric(metric)
    }
    pub fn push_gauge(
        &self,
        name: &str,
        tags: HashSet<String>,
        value: f64,
    ) -> Result<(), StoreError> {
        let metric = Metric {
            name: name.to_string(),
            tags,
            value: MetricValue::Gauge(value),
        };
        self.push_metric(metric)
    }

    pub fn push_to_timeseries(
        &self,
        name: &str,
        tags: HashSet<String>,
        value: f64,
    ) -> Result<(), StoreError> {
        // check that the metric exists, if not create it
        let mut inner = self.store.lock().map_err(|_| StoreError::LockError)?;
        let metric = inner
            .metrics
            .entry(name.to_string())
            .or_insert_with(|| Metric {
                name: name.to_string(),
                tags,
                value: MetricValue::TimeSeries {
                    values: Vec::new(),
                    start_ts: chrono::Utc::now(),
                },
            });

        if let MetricValue::TimeSeries { values, .. } = &mut metric.value {
            values.push(value);
            Ok(())
        } else {
            Err(StoreError::MetricTypeMismatch {
                expected: "TimeSeries".to_string(),
                found: format!("{:?}", metric.value),
            })
        }
    }

    pub fn push_timestamp(
        &self,
        name: &str,
        tags: HashSet<String>,
        timestamp: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), StoreError> {
        let metric = Metric {
            name: name.to_string(),
            tags,
            value: MetricValue::Timestamp(timestamp),
        };
        self.push_metric(metric)
    }

    /// resets the timeseries metrics, emptying their values and setting the start timestamp to now
    pub fn tick(&self) -> Result<(), StoreError> {
        let mut inner = self.store.lock().map_err(|_| StoreError::LockError)?;
        for metric in inner.metrics.values_mut() {
            if let MetricValue::TimeSeries { values, start_ts } = &mut metric.value {
                *values = Vec::new();
                *start_ts = chrono::Utc::now();
            }
        }
        Ok(())
    }

    pub fn query_by_name(&self, name: &str) -> Result<Metric, StoreError> {
        let inner = self.store.lock().map_err(|_| StoreError::LockError)?;
        inner
            .metrics
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
            .store
            .lock()
            .map_err(|_| StoreError::LockError)?
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

impl Default for ArcStore {
    fn default() -> Self {
        Self::new()
    }
}

pub trait MetricFilter {
    fn matches(&self, metric: &Metric) -> bool;
}

pub struct NameFilter<'a> {
    name: &'a str,
}

impl<'a> MetricFilter for NameFilter<'a> {
    fn matches(&self, metric: &Metric) -> bool {
        metric.name == self.name
    }
}

pub struct TagFilter {
    tags: HashSet<String>,
    mode: TagFilterMode,
}

impl TagFilter {
    pub fn any_of(tags: HashSet<String>) -> Self {
        TagFilter {
            tags,
            mode: TagFilterMode::Any,
        }
    }

    pub fn all_of(tags: HashSet<String>) -> Self {
        TagFilter {
            tags,
            mode: TagFilterMode::All,
        }
    }
}

pub enum TagFilterMode {
    All,
    Any,
}

impl MetricFilter for TagFilter {
    fn matches(&self, metric: &Metric) -> bool {
        match self.mode {
            TagFilterMode::All => self.tags.is_subset(&metric.tags),
            TagFilterMode::Any => self.tags.is_disjoint(&metric.tags),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn test_push_and_query_metric() {
        let store = ArcStore::new();
        let tags: HashSet<String> = ["tag1".to_string(), "tag2".to_string()]
            .iter()
            .cloned()
            .collect();

        store
            .push_counter("test_metric", tags.clone(), 42.0)
            .unwrap();

        let metric = store.query_by_name("test_metric").unwrap();
        assert_eq!(metric.name, "test_metric");
        assert_eq!(metric.tags, tags);
        if let MetricValue::Counter(value) = metric.value {
            assert_eq!(value, 42.0);
        } else {
            panic!("Expected Counter value");
        }
    }

    #[test]
    fn test_query_with_filter() {
        let store = ArcStore::new();
        let tags1: HashSet<String> = ["tag1".to_string(), "tag2".to_string()]
            .iter()
            .cloned()
            .collect();
        let tags2: HashSet<String> = ["tag3".to_string(), "tag4".to_string()]
            .iter()
            .cloned()
            .collect();

        store.push_counter("metric1", tags1.clone(), 10.0).unwrap();
        store.push_counter("metric2", tags2.clone(), 20.0).unwrap();

        let filter = NameFilter { name: "metric1" };
        let result = store.query_with_filter(filter).unwrap();
        assert_eq!(result.len(), 1);
        assert!(result.contains_key("metric1"));
    }

    #[test]
    fn test_push_and_query_timeseries() {
        let store = ArcStore::new();
        let tags: HashSet<String> = ["tag1".to_string(), "tag2".to_string()]
            .iter()
            .cloned()
            .collect();

        store
            .push_to_timeseries("test_timeseries", tags.clone(), 1.0)
            .unwrap();
        store
            .push_to_timeseries("test_timeseries", tags.clone(), 2.0)
            .unwrap();

        let metric = store.query_by_name("test_timeseries").unwrap();
        assert_eq!(metric.name, "test_timeseries");
        assert_eq!(metric.tags, tags);
        if let MetricValue::TimeSeries { values, .. } = metric.value {
            assert_eq!(values, vec![1.0, 2.0]);
        } else {
            panic!("Expected TimeSeries value");
        }
    }
}
