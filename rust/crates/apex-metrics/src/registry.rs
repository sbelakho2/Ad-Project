//! The bounded telemetry registry.
//!
//! [`Registry`] stores counters, gauges and histograms keyed by their declared
//! [`MetricSpec`] and validated label set. It is the only writer path: a value
//! that is not part of a declared vocabulary is refused (with a typed error),
//! so no producer can mint a per-customer series. Rendering, snapshots and the
//! atomic textfile export are in [`crate::render`] and [`crate::textfile`].

use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock, TryLockError};

use crate::spec::{LabelValue, MetricKind, MetricSpec};

/// The series map is sharded so observations on different label sets update
/// different locks. Only the series *creation* path takes a shard write lock;
/// an existing series is found under a read lock and then updated through its
/// own atomics, so the steady state has no write-lock contention.
const SERIES_SHARDS: usize = 64;

/// How many series handles one registry keeps cached.
///
/// The cache is a pure fast path in front of the authoritative sharded series
/// map. The in-flight gauge and the per-route request and duration series are
/// constant for a deployment, so once a series has been observed a write is
/// one shared read lock and one hash-map probe instead of a family-map lookup
/// plus a sharded lookup. The bound keeps a producer that mints many label
/// sets from growing the cache without limit: the oldest entry is evicted and
/// its series stays in the sharded map, so a later write re-adopts it and
/// nothing is lost from the exposition.
pub const HANDLE_CACHE_CAP: usize = 4096;

type SeriesMap = HashMap<u64, Vec<(SeriesKey, Arc<Series>)>>;
type SeriesShard = RwLock<SeriesMap>;

/// The identity of one cached handle: the metric name and the hash of the
/// canonical label set. The name is a `&'static str` and the hash a `u64`, so
/// building one allocates nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct HandleKey {
    metric: &'static str,
    hash: u64,
}

/// One cached handle. The owned key is kept only so a hit can prove the label
/// set matches without allocating: a hash collision falls through to the
/// authoritative map, which resolves it exactly, and a conflicting
/// declaration of the same metric name is never served from here.
#[derive(Debug)]
struct CacheEntry {
    spec: &'static MetricSpec,
    key: SeriesKey,
    series: Arc<Series>,
}

/// The bounded handle cache plus the insertion order used to evict the oldest
/// entry. Both live under one lock so an insert and an eviction are atomic.
#[derive(Debug, Default)]
struct HandleCache {
    map: HashMap<HandleKey, CacheEntry>,
    order: std::collections::VecDeque<HandleKey>,
}

/// The owned form of a series key, kept only for a series that exists. The
/// steady-state lookup never builds one: it hashes the borrowed label values
/// and compares them against the stored key in place.
#[derive(Debug, Clone)]
struct SeriesKey(Box<[String]>);

impl SeriesKey {
    /// Copy the validated values in declaration order. This is the only
    /// allocation on the series-creation path.
    fn new(spec: &MetricSpec, labels: &[(&'static str, &str)]) -> Self {
        Self(
            spec.labels
                .iter()
                .map(|declared| {
                    value_of(labels, declared.name)
                        .unwrap_or_default()
                        .to_string()
                })
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        )
    }

    /// Compare with the incoming (borrowed) label values without allocating.
    fn matches(&self, spec: &MetricSpec, labels: &[(&'static str, &str)]) -> bool {
        self.0.len() == spec.labels.len()
            && self.0.iter().zip(spec.labels).all(|(stored, declared)| {
                stored.as_str() == value_of(labels, declared.name).unwrap_or_default()
            })
    }
}

/// The value of a declared label in the incoming set, borrowed (no copy).
fn value_of<'a>(labels: &'a [(&'static str, &str)], name: &str) -> Option<&'a str> {
    labels
        .iter()
        .find(|(label, _)| *label == name)
        .map(|(_, value)| *value)
}

/// Why a write was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteError {
    /// A label or label value failed the syntactic gate.
    Label(crate::spec::LabelError),
    /// A label name the metric does not declare was supplied.
    UnknownLabel {
        /// The metric.
        metric: &'static str,
        /// The undeclared label name.
        label: String,
    },
    /// A declared label was not supplied.
    MissingLabel {
        /// The metric.
        metric: &'static str,
        /// The missing label name.
        label: &'static str,
    },
    /// A label name was supplied twice.
    DuplicateLabel {
        /// The metric.
        metric: &'static str,
        /// The duplicated label name.
        label: String,
    },
    /// A label value is not in the metric's declared vocabulary.
    UndeclaredValue {
        /// The metric.
        metric: &'static str,
        /// The label name.
        label: &'static str,
        /// The refused value.
        value: String,
        /// What the label accepts.
        accepts: &'static str,
    },
    /// The write names a metric of a different kind.
    KindMismatch {
        /// The metric.
        metric: &'static str,
        /// What the operation needs.
        expected: &'static str,
    },
    /// Two different declarations with the same name reached one registry.
    ConflictingDeclaration {
        /// The metric name.
        metric: &'static str,
    },
}

impl std::fmt::Display for WriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WriteError::Label(error) => write!(f, "{error}"),
            WriteError::UnknownLabel { metric, label } => {
                write!(f, "{metric} has no declared label '{label}'")
            }
            WriteError::MissingLabel { metric, label } => {
                write!(f, "{metric} requires label '{label}'")
            }
            WriteError::DuplicateLabel { metric, label } => {
                write!(f, "{metric} label '{label}' was supplied twice")
            }
            WriteError::UndeclaredValue {
                metric,
                label,
                value,
                accepts,
            } => write!(
                f,
                "{metric} label '{label}' value '{value}' is outside the {accepts}"
            ),
            WriteError::KindMismatch { metric, expected } => {
                write!(f, "{metric} is not a {expected}")
            }
            WriteError::ConflictingDeclaration { metric } => {
                write!(f, "two declarations named '{metric}' disagree")
            }
        }
    }
}

impl std::error::Error for WriteError {}

impl From<crate::spec::LabelError> for WriteError {
    fn from(error: crate::spec::LabelError) -> Self {
        WriteError::Label(error)
    }
}

/// One series' stored value.
///
/// Every field is an atomic so a write to an existing series never holds a
/// map lock while it mutates. The bucket vector is fixed at declaration time
/// (the declared bounds plus the implicit `+Inf` bucket) and never resized.
#[derive(Debug)]
struct Series {
    counter: AtomicU64,
    /// The gauge as raw `f64` bits (`AtomicU64` has no `f64` form).
    gauge: AtomicU64,
    observations: AtomicU64,
    /// The observation sum as raw `f64` bits.
    sum: AtomicU64,
    /// Per-bucket counts (NOT cumulative); length = buckets + 1 (the implicit
    /// `+Inf` bucket). An observation increments exactly one bucket, so a
    /// write costs one atomic add instead of one per matching bound.
    /// Snapshots prefix-sum the vector to keep the public cumulative shape.
    buckets: Box<[AtomicU64]>,
}

impl Series {
    fn new(kind: MetricKind) -> Self {
        let bucket_count = match kind {
            MetricKind::Histogram { buckets } => buckets.len() + 1,
            MetricKind::Counter | MetricKind::Gauge => 0,
        };
        Self {
            counter: AtomicU64::new(0),
            gauge: AtomicU64::new(0.0f64.to_bits()),
            observations: AtomicU64::new(0),
            sum: AtomicU64::new(0.0f64.to_bits()),
            buckets: (0..bucket_count)
                .map(|_| AtomicU64::new(0))
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        }
    }

    fn counter_add(&self, delta: u64) -> u64 {
        let previous = self
            .counter
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                Some(value.saturating_add(delta))
            })
            .unwrap_or(0);
        previous.saturating_add(delta)
    }

    fn counter_value(&self) -> u64 {
        self.counter.load(Ordering::Relaxed)
    }

    fn gauge_value(&self) -> f64 {
        f64::from_bits(self.gauge.load(Ordering::Relaxed))
    }

    fn gauge_set(&self, value: f64) {
        self.gauge.store(value.to_bits(), Ordering::Relaxed);
    }

    fn gauge_add(&self, delta: f64) {
        let _ = self
            .gauge
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |bits| {
                Some((f64::from_bits(bits) + delta).to_bits())
            });
    }

    fn observe(&self, value: f64, bounds: &[f64]) {
        // Find the first bucket whose upper bound admits the value. Buckets
        // are stored per-observation (not cumulative), so exactly one atomic
        // add is needed: the old representation touched every matching bound
        // and the implicit `+Inf` bucket on each observation.
        let index = bounds
            .iter()
            .position(|bound| value <= *bound)
            .unwrap_or_else(|| self.buckets.len().saturating_sub(1));
        if let Some(bucket) = self.buckets.get(index) {
            bucket.fetch_add(1, Ordering::Relaxed);
        }
        self.observations.fetch_add(1, Ordering::Relaxed);
        let _ = self
            .sum
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |bits| {
                Some((f64::from_bits(bits) + value).to_bits())
            });
    }

    fn observations(&self) -> u64 {
        self.observations.load(Ordering::Relaxed)
    }

    fn sum(&self) -> f64 {
        f64::from_bits(self.sum.load(Ordering::Relaxed))
    }

    fn bucket(&self, index: usize) -> u64 {
        self.buckets
            .get(index)
            .map(|bucket| bucket.load(Ordering::Relaxed))
            .unwrap_or(0)
    }
}

/// One metric family: the immutable declaration plus the sharded series map.
#[derive(Debug)]
struct Family {
    spec: &'static MetricSpec,
    shards: Box<[SeriesShard]>,
    stats: Arc<LockStats>,
}

impl Family {
    fn new(spec: &'static MetricSpec, stats: Arc<LockStats>) -> Self {
        Self {
            spec,
            shards: (0..SERIES_SHARDS)
                .map(|_| RwLock::new(HashMap::new()))
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            stats,
        }
    }

    fn shard(&self, hash: u64) -> usize {
        (hash as usize) % self.shards.len()
    }

    /// The stored series for `labels`, if any, without creating it. The lookup
    /// borrows the incoming values: no key, `String` or `Vec` is allocated.
    fn lookup(&self, labels: &[(&'static str, &str)], hash: u64) -> Option<Arc<Series>> {
        let shard = self.shard(hash);
        let guard = read_or_wait(self.shards.get(shard)?, &self.stats);
        guard
            .get(&hash)?
            .iter()
            .find_map(|(key, series)| key.matches(self.spec, labels).then(|| Arc::clone(series)))
    }

    /// The stored series for `labels`, creating it when absent. The returned
    /// handle is safe to use after every lock is released. Only the creation
    /// path allocates the owned key.
    fn series(&self, labels: &[(&'static str, &str)], hash: u64) -> Arc<Series> {
        let shard = self.shard(hash);
        let Some(slot) = self.shards.get(shard) else {
            // `shard` is always in range; this keeps the method total.
            return Arc::new(Series::new(self.spec.kind));
        };
        {
            let guard = read_or_wait(slot, &self.stats);
            if let Some(bucket) = guard.get(&hash) {
                if let Some((_, series)) = bucket
                    .iter()
                    .find(|(key, _)| key.matches(self.spec, labels))
                {
                    return Arc::clone(series);
                }
            }
        }
        let mut guard = write_or_wait(slot, &self.stats);
        let bucket = guard.entry(hash).or_default();
        if let Some((_, series)) = bucket
            .iter()
            .find(|(key, _)| key.matches(self.spec, labels))
        {
            return Arc::clone(series);
        }
        let series = Arc::new(Series::new(self.spec.kind));
        bucket.push((SeriesKey::new(self.spec, labels), Arc::clone(&series)));
        series
    }

    /// Every series, in canonical key order.
    fn series_snapshot(&self) -> Vec<(Vec<String>, Arc<Series>)> {
        let mut all = Vec::new();
        for shard in self.shards.iter() {
            let guard = read_or_wait(shard, &self.stats);
            all.extend(guard.values().flat_map(|bucket| {
                bucket
                    .iter()
                    .map(|(key, series)| (key.0.to_vec(), Arc::clone(series)))
            }));
        }
        all.sort_by(|left, right| left.0.cmp(&right.0));
        all
    }
}

/// A snapshot of one series.
#[derive(Debug, Clone, PartialEq)]
pub struct SeriesSnapshot {
    /// Labels in declaration order.
    pub labels: Vec<(&'static str, String)>,
    /// The counter total, for counters.
    pub counter: Option<u64>,
    /// The gauge value, for gauges.
    pub gauge: Option<f64>,
    /// The histogram state, for histograms.
    pub histogram: Option<HistogramSnapshot>,
}

/// A snapshot of one histogram series.
#[derive(Debug, Clone, PartialEq)]
pub struct HistogramSnapshot {
    /// Declared upper bounds.
    pub buckets: Vec<f64>,
    /// Cumulative counts per bucket (including the implicit `+Inf` bucket).
    pub cumulative_counts: Vec<u64>,
    /// Total observations (the `+Inf` bucket).
    pub count: u64,
    /// Sum of all observations.
    pub sum: f64,
}

/// A snapshot of one metric family.
#[derive(Debug, Clone, PartialEq)]
pub struct FamilySnapshot {
    /// The metric name.
    pub name: &'static str,
    /// The `# HELP` text.
    pub help: &'static str,
    /// Counter, gauge or histogram.
    pub kind: MetricKind,
    /// Series in canonical (label-sorted) order.
    pub series: Vec<SeriesSnapshot>,
}

/// Everything the registry holds, in canonical order.
pub type Snapshot = Vec<FamilySnapshot>;

/// Deterministic lock instrumentation for the contention harness: a read or
/// write that had to wait for an in-progress writer. In steady state both
/// stay flat because observations update per-series atomics.
#[derive(Debug, Default)]
struct LockStats {
    read_waits: AtomicU64,
    write_waits: AtomicU64,
}

/// The lock waits observed by one registry.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LockWaits {
    /// Reads that could not take the lock immediately and blocked.
    pub read_waits: u64,
    /// Writes that could not take the lock immediately and blocked.
    pub write_waits: u64,
}

fn read_or_wait<'a, T>(
    lock: &'a RwLock<T>,
    stats: &LockStats,
) -> std::sync::RwLockReadGuard<'a, T> {
    match lock.try_read() {
        Ok(guard) => guard,
        Err(TryLockError::WouldBlock) => {
            stats.read_waits.fetch_add(1, Ordering::Relaxed);
            lock.read().unwrap_or_else(|poisoned| poisoned.into_inner())
        }
        Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
    }
}

fn write_or_wait<'a, T>(
    lock: &'a RwLock<T>,
    stats: &LockStats,
) -> std::sync::RwLockWriteGuard<'a, T> {
    match lock.try_write() {
        Ok(guard) => guard,
        Err(TryLockError::WouldBlock) => {
            stats.write_waits.fetch_add(1, Ordering::Relaxed);
            lock.write()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
        }
        Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
    }
}

/// The process-wide telemetry registry.
#[derive(Debug)]
pub struct Registry {
    families: RwLock<HashMap<&'static str, Arc<Family>>>,
    handles: RwLock<HandleCache>,
    stats: Arc<LockStats>,
}

impl Default for Registry {
    fn default() -> Self {
        Self {
            families: RwLock::new(HashMap::new()),
            handles: RwLock::new(HandleCache::default()),
            stats: Arc::new(LockStats::default()),
        }
    }
}

impl Registry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Lock waits this registry observed so far. Test and operations seam for
    /// the contention harness; it never changes a value.
    pub fn lock_waits(&self) -> LockWaits {
        LockWaits {
            read_waits: self.stats.read_waits.load(Ordering::Relaxed),
            write_waits: self.stats.write_waits.load(Ordering::Relaxed),
        }
    }

    /// Increment a counter by one and return the new total.
    pub fn increment(
        &self,
        metric: &'static MetricSpec,
        labels: &[(&'static str, &str)],
    ) -> Result<u64, WriteError> {
        self.increment_by(metric, labels, 1)
    }

    /// Increment a counter and return the new total.
    pub fn increment_by(
        &self,
        metric: &'static MetricSpec,
        labels: &[(&'static str, &str)],
        delta: u64,
    ) -> Result<u64, WriteError> {
        let hash = validate(metric, labels, "counter")?;
        Ok(self.series_handle(metric, labels, hash)?.counter_add(delta))
    }

    /// Set a gauge to `value`.
    pub fn gauge_set(
        &self,
        metric: &'static MetricSpec,
        labels: &[(&'static str, &str)],
        value: f64,
    ) -> Result<(), WriteError> {
        let hash = validate(metric, labels, "gauge")?;
        self.series_handle(metric, labels, hash)?.gauge_set(value);
        Ok(())
    }

    /// Add `delta` to a gauge.
    pub fn gauge_add(
        &self,
        metric: &'static MetricSpec,
        labels: &[(&'static str, &str)],
        delta: f64,
    ) -> Result<(), WriteError> {
        let hash = validate(metric, labels, "gauge")?;
        self.series_handle(metric, labels, hash)?.gauge_add(delta);
        Ok(())
    }

    /// The current value of a gauge series, when it exists.
    pub fn gauge_value(
        &self,
        metric: &'static MetricSpec,
        labels: &[(&'static str, &str)],
    ) -> Result<f64, WriteError> {
        let hash = validate(metric, labels, "gauge")?;
        Ok(self
            .lookup_series(metric, labels, hash)
            .map(|series| series.gauge_value())
            .unwrap_or(0.0))
    }

    /// Observe one histogram value.
    pub fn observe(
        &self,
        metric: &'static MetricSpec,
        labels: &[(&'static str, &str)],
        value: f64,
    ) -> Result<(), WriteError> {
        let buckets = metric.buckets().ok_or(WriteError::KindMismatch {
            metric: metric.name,
            expected: "histogram",
        })?;
        let hash = validate(metric, labels, "histogram")?;
        let series = self.series_handle(metric, labels, hash)?;
        series.observe(value, buckets);
        Ok(())
    }

    /// An in-flight guard: increments the gauge now and decrements it on drop,
    /// so cancellation and error unwinding cannot leak the gauge.
    pub fn in_flight(
        &self,
        metric: &'static MetricSpec,
        labels: &[(&'static str, &str)],
    ) -> Result<InFlightGuard<'_>, WriteError> {
        let hash = validate(metric, labels, "gauge")?;
        let series = self.series_handle(metric, labels, hash)?;
        series.gauge_add(1.0);
        Ok(InFlightGuard {
            series,
            _registry: std::marker::PhantomData,
        })
    }

    /// A snapshot of every family and series, in canonical order.
    pub fn snapshot(&self) -> Snapshot {
        let families = read_or_wait(&self.families, &self.stats);
        let mut snapshot: Snapshot = families
            .values()
            .map(|family| FamilySnapshot {
                name: family.spec.name,
                help: family.spec.help,
                kind: family.spec.kind,
                series: family
                    .series_snapshot()
                    .into_iter()
                    .map(|(key, series)| series_snapshot(family.spec, &key, &series))
                    .collect(),
            })
            .collect();
        snapshot.sort_by_key(|family| family.name);
        snapshot
    }

    /// The snapshot of one family by name.
    pub fn family_snapshot(&self, name: &str) -> Option<FamilySnapshot> {
        self.snapshot()
            .into_iter()
            .find(|family| family.name == name)
    }

    /// The current counter total of one series, or zero when never written.
    pub fn counter_value(
        &self,
        metric: &'static MetricSpec,
        labels: &[(&'static str, &str)],
    ) -> Result<u64, WriteError> {
        let hash = validate(metric, labels, "counter")?;
        Ok(self
            .lookup_series(metric, labels, hash)
            .map(|series| series.counter_value())
            .unwrap_or(0))
    }

    fn lookup_series(
        &self,
        metric: &MetricSpec,
        labels: &[(&'static str, &str)],
        hash: u64,
    ) -> Option<Arc<Series>> {
        let families = read_or_wait(&self.families, &self.stats);
        families
            .get(metric.name)
            .and_then(|family| family.lookup(labels, hash))
    }

    /// The handle-cache fast path: the series for `labels`, served from the
    /// bounded cache when present and otherwise resolved through the
    /// authoritative family and sharded series maps (which create it) and then
    /// adopted into the cache.
    ///
    /// A cache hit takes one shared read lock and one hash-map probe, then
    /// clones the `Arc` (no allocation). A hash collision or a conflicting
    /// declaration of the same metric name is not served from the cache: the
    /// `spec` and owned key checks send it to `family`, which resolves it
    /// exactly and refuses a real conflict.
    fn series_handle(
        &self,
        metric: &'static MetricSpec,
        labels: &[(&'static str, &str)],
        hash: u64,
    ) -> Result<Arc<Series>, WriteError> {
        let key = HandleKey {
            metric: metric.name,
            hash,
        };
        {
            let cache = read_or_wait(&self.handles, &self.stats);
            if let Some(entry) = cache.map.get(&key) {
                if entry.spec == metric && entry.key.matches(metric, labels) {
                    return Ok(Arc::clone(&entry.series));
                }
            }
        }
        let family = self.family(metric)?;
        let series = family.series(labels, hash);
        self.cache_handle(key, metric, labels, Arc::clone(&series));
        Ok(series)
    }

    /// Adopt a resolved series into the bounded cache, evicting the oldest
    /// entry when the cap is reached. Called only off the creation path, so a
    /// steady-state write never takes this write lock.
    fn cache_handle(
        &self,
        key: HandleKey,
        metric: &'static MetricSpec,
        labels: &[(&'static str, &str)],
        series: Arc<Series>,
    ) {
        let mut cache = write_or_wait(&self.handles, &self.stats);
        if let Some(entry) = cache.map.get_mut(&key) {
            // A collision or a refreshed entry replaces in place; the series
            // identity is unchanged because `family.series` is authoritative.
            entry.spec = metric;
            entry.key = SeriesKey::new(metric, labels);
            entry.series = series;
            return;
        }
        while cache.map.len() >= HANDLE_CACHE_CAP {
            let Some(oldest) = cache.order.pop_front() else {
                break;
            };
            cache.map.remove(&oldest);
        }
        cache.map.insert(
            key,
            CacheEntry {
                spec: metric,
                key: SeriesKey::new(metric, labels),
                series,
            },
        );
        cache.order.push_back(key);
    }

    /// The number of series handles currently cached. A test and operations
    /// seam; it never changes a value. The cache is bounded by
    /// [`HANDLE_CACHE_CAP`].
    pub fn handle_cache_len(&self) -> usize {
        read_or_wait(&self.handles, &self.stats).map.len()
    }

    /// Get or declare the family for `metric`; a conflicting re-declaration
    /// under the same name is refused so two crates cannot silently merge
    /// series.
    fn family(&self, metric: &'static MetricSpec) -> Result<Arc<Family>, WriteError> {
        {
            let families = read_or_wait(&self.families, &self.stats);
            if let Some(family) = families.get(metric.name) {
                if family.spec != metric {
                    return Err(WriteError::ConflictingDeclaration {
                        metric: metric.name,
                    });
                }
                return Ok(Arc::clone(family));
            }
        }
        let mut families = write_or_wait(&self.families, &self.stats);
        resolve_family(families.entry(metric.name), metric, &self.stats)
    }
}

/// Resolve a family-map entry under the write lock: insert a vacant entry,
/// return an occupied entry whose declaration matches, and refuse one that
/// does not.
fn resolve_family(
    entry: Entry<'_, &'static str, Arc<Family>>,
    metric: &'static MetricSpec,
    stats: &Arc<LockStats>,
) -> Result<Arc<Family>, WriteError> {
    match entry {
        Entry::Occupied(existing) => {
            if existing.get().spec != metric {
                return Err(WriteError::ConflictingDeclaration {
                    metric: metric.name,
                });
            }
            Ok(Arc::clone(existing.get()))
        }
        Entry::Vacant(slot) => {
            let family = Arc::new(Family::new(metric, Arc::clone(stats)));
            slot.insert(Arc::clone(&family));
            Ok(family)
        }
    }
}

/// Validate a label set against a metric declaration and return the hash of
/// the canonical series key (values in declaration order).
///
/// The steady-state path allocates nothing: the duplicate and missing checks
/// scan the borrowed slices, and the hash walks the declared vocabulary in
/// order. Only error variants (which own their message text) allocate, and
/// only a new series copies its values.
fn validate(
    metric: &'static MetricSpec,
    labels: &[(&'static str, &str)],
    expected: &'static str,
) -> Result<u64, WriteError> {
    match (metric.kind, expected) {
        (MetricKind::Counter, "counter") => {}
        (MetricKind::Gauge, "gauge") => {}
        (MetricKind::Histogram { .. }, "histogram") => {}
        _ => {
            return Err(WriteError::KindMismatch {
                metric: metric.name,
                expected,
            })
        }
    }
    // The checks are in the same order as the owning implementation, so every
    // typed error is unchanged; only the per-label vector is gone.
    for (index, (name, value)) in labels.iter().enumerate() {
        let declared = metric.labels.iter().find(|label| label.name == *name);
        let Some(declared) = declared else {
            return Err(WriteError::UnknownLabel {
                metric: metric.name,
                label: (*name).to_string(),
            });
        };
        let value = LabelValue::new(name, value)?.value();
        if !declared.vocab.accepts(value) {
            return Err(WriteError::UndeclaredValue {
                metric: metric.name,
                label: name,
                value: value.to_string(),
                accepts: declared.vocab.kind_name(),
            });
        }
        if labels[..index]
            .iter()
            .any(|(existing, _)| *existing == *name)
        {
            return Err(WriteError::DuplicateLabel {
                metric: metric.name,
                label: (*name).to_string(),
            });
        }
    }
    for declared in metric.labels {
        if value_of(labels, declared.name).is_none() {
            return Err(WriteError::MissingLabel {
                metric: metric.name,
                label: declared.name,
            });
        }
    }
    Ok(label_hash(metric, labels))
}

/// The series-key hash over the values in declaration order. Borrowed values
/// are hashed in place, so an existing-series probe allocates nothing.
fn label_hash(metric: &MetricSpec, labels: &[(&'static str, &str)]) -> u64 {
    let mut hasher = DefaultHasher::new();
    for declared in metric.labels {
        value_of(labels, declared.name)
            .unwrap_or_default()
            .hash(&mut hasher);
    }
    hasher.finish()
}

fn series_snapshot(spec: &MetricSpec, key: &[String], series: &Series) -> SeriesSnapshot {
    let labels = spec
        .labels
        .iter()
        .zip(key.iter())
        .map(|(label, value)| (label.name, value.clone()))
        .collect();
    match spec.kind {
        MetricKind::Counter => SeriesSnapshot {
            labels,
            counter: Some(series.counter_value()),
            gauge: None,
            histogram: None,
        },
        MetricKind::Gauge => SeriesSnapshot {
            labels,
            counter: None,
            gauge: Some(series.gauge_value()),
            histogram: None,
        },
        MetricKind::Histogram { buckets } => {
            // Stored buckets are per-observation; the public snapshot shape is
            // cumulative (Prometheus exposition semantics). Prefix-sum here,
            // once per scrape, instead of paying for it on every observation.
            let mut running = 0u64;
            let cumulative_counts: Vec<u64> = (0..series.buckets.len())
                .map(|index| {
                    running = running.saturating_add(series.bucket(index));
                    running
                })
                .collect();
            SeriesSnapshot {
                labels,
                counter: None,
                gauge: None,
                histogram: Some(HistogramSnapshot {
                    buckets: buckets.to_vec(),
                    cumulative_counts,
                    count: series.observations(),
                    sum: series.sum(),
                }),
            }
        }
    }
}

/// One in-flight operation; decrements its gauge on drop. The guard owns the
/// series handle, so the decrement needs no map lookup or global lock.
#[derive(Debug)]
pub struct InFlightGuard<'a> {
    series: Arc<Series>,
    _registry: std::marker::PhantomData<&'a Registry>,
}

impl Drop for InFlightGuard<'_> {
    fn drop(&mut self) {
        self.series.gauge_add(-1.0);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::catalog;
    use crate::spec::{LabelError, LabelSpec, Vocab};
    use crate::vocab::{JobType, Outcome, Platform};
    use std::sync::mpsc;
    use std::time::Duration;

    fn job_labels<'a>(
        job: &'a str,
        platform: &'a str,
        outcome: &'a str,
    ) -> Vec<(&'static str, &'a str)> {
        vec![
            ("job_type", job),
            ("platform", platform),
            ("outcome", outcome),
        ]
    }

    #[test]
    fn counters_accumulate_per_series() {
        let registry = Registry::new();
        let metric = &catalog::JOB_RUNS_TOTAL;
        let labels = job_labels("daily", "applovin_max", "completed");
        assert_eq!(registry.increment(metric, &labels).unwrap(), 1);
        assert_eq!(registry.increment(metric, &labels).unwrap(), 2);
        assert_eq!(registry.increment_by(metric, &labels, 5).unwrap(), 7);
        let other = job_labels("monthly", "applovin_max", "completed");
        assert_eq!(registry.increment(metric, &other).unwrap(), 1);
        assert_eq!(registry.counter_value(metric, &labels).unwrap(), 7);
    }

    #[test]
    fn id_like_label_values_are_refused_everywhere() {
        let registry = Registry::new();
        let metric = &catalog::PAYMENT_EVENTS_TOTAL;
        // The metric has no publisher/experiment/invoice/PSP label at all.
        for name in ["publisher_id", "experiment_id", "invoice_id", "payment_id"] {
            let labels = vec![
                (name, "value"),
                ("event_class", "authorisation"),
                ("result", "accepted"),
            ];
            let error = registry.increment(metric, &labels).expect_err(name);
            assert!(matches!(error, WriteError::UnknownLabel { .. }), "{error}");
        }
        // An id-like value on a declared label is refused by the syntax gate.
        let labels = vec![
            ("provider", "mollie"),
            ("event_class", "authorisation"),
            ("result", "psp_123456"),
        ];
        let error = registry.increment(metric, &labels).expect_err("id value");
        assert!(
            matches!(
                error,
                WriteError::Label(crate::spec::LabelError::IdentifierLike { .. })
            ),
            "{error}"
        );
        // A value outside the declared vocabulary is refused.
        let labels = vec![
            ("provider", "northstar"),
            ("event_class", "authorisation"),
            ("result", "accepted"),
        ];
        let error = registry
            .increment(metric, &labels)
            .expect_err("publisher id");
        assert!(
            matches!(error, WriteError::UndeclaredValue { .. }),
            "{error}"
        );
        // Emails and URLs never pass the syntax gate.
        for bad in ["ops@example.com", "https://example.com/x"] {
            let error = LabelValue::new("x", bad).expect_err(bad);
            assert!(matches!(WriteError::from(error), WriteError::Label(_)));
        }
    }

    #[test]
    fn missing_duplicate_and_unknown_labels_are_refused() {
        let registry = Registry::new();
        let metric = &catalog::JOB_RUNS_TOTAL;
        let error = registry
            .increment(metric, &[("platform", "admob"), ("outcome", "completed")])
            .expect_err("missing job_type");
        assert!(matches!(
            error,
            WriteError::MissingLabel {
                label: "job_type",
                ..
            }
        ));
        let duplicate = vec![
            ("job_type", "daily"),
            ("job_type", "monthly"),
            ("platform", "admob"),
            ("outcome", "completed"),
        ];
        let error = registry
            .increment(metric, &duplicate)
            .expect_err("duplicate");
        assert!(
            matches!(error, WriteError::DuplicateLabel { .. }),
            "{error}"
        );
        let undeclared = job_labels("welcome", "admob", "completed");
        let error = registry
            .increment(metric, &undeclared)
            .expect_err("job type");
        assert!(
            matches!(error, WriteError::UndeclaredValue { .. }),
            "{error}"
        );
    }

    #[test]
    fn kind_mismatches_are_refused() {
        let registry = Registry::new();
        let labels = job_labels("daily", "applovin_max", "completed");
        assert!(matches!(
            registry.gauge_set(&catalog::JOB_RUNS_TOTAL, &labels, 1.0),
            Err(WriteError::KindMismatch { .. })
        ));
        assert!(matches!(
            registry.observe(&catalog::JOB_RUNS_TOTAL, &labels, 1.0),
            Err(WriteError::KindMismatch { .. })
        ));
        let gauge_labels = [("service", "apex-api")];
        assert!(matches!(
            registry.increment(&catalog::HTTP_REQUESTS_IN_FLIGHT, &gauge_labels),
            Err(WriteError::KindMismatch { .. })
        ));
    }

    #[test]
    fn gauges_set_add_and_read() {
        let registry = Registry::new();
        let metric = &catalog::REPORT_ACCOUNTS;
        let labels = [("platform", "applovin_max"), ("readiness_class", "final")];
        registry.gauge_set(metric, &labels, 3.0).unwrap();
        registry.gauge_add(metric, &labels, 2.5).unwrap();
        assert_eq!(registry.gauge_value(metric, &labels).unwrap(), 5.5);
        assert_eq!(
            registry
                .gauge_value(
                    metric,
                    &[("platform", "admob"), ("readiness_class", "stale")]
                )
                .unwrap(),
            0.0
        );
        registry.gauge_set(metric, &labels, -1.0).unwrap();
        assert_eq!(registry.gauge_value(metric, &labels).unwrap(), -1.0);
    }

    #[test]
    fn histograms_keep_cumulative_buckets_count_and_sum() {
        let registry = Registry::new();
        let metric = &catalog::CONNECTOR_DURATION_SECONDS;
        let labels = [("platform", "applovin_max"), ("operation", "maxReport")];
        for value in [0.01, 0.3, 2.0, 90.0] {
            registry.observe(metric, &labels, value).unwrap();
        }
        let family = registry.family_snapshot(metric.name).expect("family");
        let series = &family.series[0];
        let histogram = series.histogram.as_ref().expect("histogram");
        assert_eq!(histogram.count, 4);
        assert!((histogram.sum - 92.31).abs() < 1e-9);
        // 0.05 bucket: only the 0.01 observation.
        assert_eq!(histogram.cumulative_counts[0], 1);
        // 0.5 bucket: 0.01 and 0.3.
        assert_eq!(histogram.cumulative_counts[3], 2);
        // +Inf bucket: everything.
        assert_eq!(*histogram.cumulative_counts.last().unwrap(), 4);
        assert_eq!(
            histogram.cumulative_counts.len(),
            histogram.buckets.len() + 1
        );
    }

    #[test]
    fn conflicts_and_debug_surfaces_work() {
        let registry = Registry::new();
        let labels = job_labels("daily", "applovin_max", "completed");
        registry
            .increment(&catalog::JOB_RUNS_TOTAL, &labels)
            .unwrap();
        assert!(!format!("{registry:?}").is_empty());
        let error = WriteError::ConflictingDeclaration { metric: "x" };
        assert!(error.to_string().contains("disagree"));
        let error = WriteError::from(crate::spec::LabelError::Empty { label: "y".into() });
        assert!(error.to_string().contains("empty"));
        for job in JobType::VALUES {
            registry
                .increment(
                    &catalog::JOB_RUNS_TOTAL,
                    &job_labels(job, "admob", "completed"),
                )
                .unwrap();
        }
        for platform in Platform::VALUES {
            registry
                .increment(
                    &catalog::JOB_RUNS_TOTAL,
                    &job_labels("daily", platform, Outcome::Completed.as_str()),
                )
                .unwrap();
        }
    }

    #[test]
    fn in_flight_guard_decrements_on_drop() {
        let registry = Registry::new();
        let metric = &catalog::HTTP_REQUESTS_IN_FLIGHT;
        let labels = [("service", "apex-api")];
        let outer = registry.in_flight(metric, &labels).unwrap();
        {
            let _inner = registry.in_flight(metric, &labels).unwrap();
            assert_eq!(registry.gauge_value(metric, &labels).unwrap(), 2.0);
        }
        assert_eq!(registry.gauge_value(metric, &labels).unwrap(), 1.0);
        drop(outer);
        assert_eq!(registry.gauge_value(metric, &labels).unwrap(), 0.0);
        // Dropping again (by scope exit) never underflows past zero here
        // because each guard decrements exactly once.
        assert_eq!(registry.gauge_value(metric, &labels).unwrap(), 0.0);
    }

    #[tokio::test]
    async fn in_flight_guard_cleans_up_when_the_task_is_cancelled() {
        // Leak the registry so the spawned (possibly aborted) task can hold
        // the guard; this is a test process, so the memory is reclaimed at
        // exit.
        let registry: &'static Registry = Box::leak(Box::new(Registry::new()));
        let metric = &catalog::HTTP_REQUESTS_IN_FLIGHT;
        let labels = [("service", "apex-runner")];
        let guard = registry.in_flight(metric, &labels).unwrap();
        assert_eq!(registry.gauge_value(metric, &labels).unwrap(), 1.0);
        // Simulate a cancelled request: the future holding the guard is
        // dropped, which must run the guard's Drop.
        let pending = async move {
            let _guard = guard;
            std::future::pending::<()>().await;
        };
        let handle = tokio::spawn(pending);
        tokio::task::yield_now().await;
        handle.abort();
        let _ = handle.await;
        assert_eq!(registry.gauge_value(metric, &labels).unwrap(), 0.0);
    }

    #[test]
    fn snapshot_is_canonical_and_complete() {
        let registry = Registry::new();
        registry
            .increment(
                &catalog::JOB_RUNS_TOTAL,
                &job_labels("monthly", "applovin_max", "completed"),
            )
            .unwrap();
        registry
            .increment(
                &catalog::JOB_RUNS_TOTAL,
                &job_labels("daily", "applovin_max", "completed"),
            )
            .unwrap();
        registry
            .gauge_set(
                &catalog::HTTP_REQUESTS_IN_FLIGHT,
                &[("service", "apex-api")],
                1.0,
            )
            .unwrap();
        let snapshot = registry.snapshot();
        assert_eq!(snapshot.len(), 2);
        // Families sort by name: apex_http_* before apex_job_*.
        assert_eq!(snapshot[0].name, catalog::HTTP_REQUESTS_IN_FLIGHT.name);
        assert_eq!(snapshot[1].name, catalog::JOB_RUNS_TOTAL.name);
        // Series sort by their label tuple: daily before monthly.
        assert_eq!(snapshot[1].series[0].labels[0].1, "daily");
        assert_eq!(snapshot[1].series[0].counter, Some(1));
        assert!(registry.family_snapshot("apex_missing").is_none());
    }

    #[test]
    fn every_write_error_variant_displays_its_exact_message() {
        let cases = [
            (
                WriteError::Label(LabelError::Empty {
                    label: "service".into(),
                }),
                "label 'service' value is empty",
            ),
            (
                WriteError::UnknownLabel {
                    metric: "apex_probe_total",
                    label: "publisher_id".into(),
                },
                "apex_probe_total has no declared label 'publisher_id'",
            ),
            (
                WriteError::MissingLabel {
                    metric: "apex_probe_total",
                    label: "outcome",
                },
                "apex_probe_total requires label 'outcome'",
            ),
            (
                WriteError::DuplicateLabel {
                    metric: "apex_probe_total",
                    label: "Platform".into(),
                },
                "apex_probe_total label 'Platform' was supplied twice",
            ),
            (
                WriteError::UndeclaredValue {
                    metric: "apex_probe_total",
                    label: "platform",
                    value: "northstar".into(),
                    accepts: "declared value set",
                },
                "apex_probe_total label 'platform' value 'northstar' is outside the declared value set",
            ),
            (
                WriteError::KindMismatch {
                    metric: "apex_probe_total",
                    expected: "gauge",
                },
                "apex_probe_total is not a gauge",
            ),
            (
                WriteError::ConflictingDeclaration {
                    metric: "apex_probe_total",
                },
                "two declarations named 'apex_probe_total' disagree",
            ),
        ];
        for (error, expected) in cases {
            assert_eq!(error.to_string(), expected);
        }
    }

    #[test]
    fn poisoned_series_shard_recovers_without_losing_the_mutation() {
        let stats = Arc::new(LockStats::default());
        let family = Family::new(&catalog::JOB_RUNS_TOTAL, Arc::clone(&stats));
        let labels = [
            ("job_type", "daily"),
            ("platform", "applovin_max"),
            ("outcome", "completed"),
        ];
        let hash = label_hash(&catalog::JOB_RUNS_TOTAL, &labels);
        let shard = family.shard(hash);
        std::thread::scope(|scope| {
            let slot = &family.shards[shard];
            let handle = scope.spawn(move || {
                let _guard = slot.write().unwrap();
                panic!("poison the series shard");
            });
            assert!(handle.join().is_err());
        });
        assert!(family.shards[shard].is_poisoned());
        let series = family.series(&labels, hash);
        assert_eq!(series.counter_add(2), 2);
        assert_eq!(family.lookup(&labels, hash).unwrap().counter_value(), 2);
        assert_eq!(family.series_snapshot().len(), 1);
        assert_eq!(stats.read_waits.load(Ordering::Relaxed), 0);
        assert_eq!(stats.write_waits.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn poisoned_family_map_recovers_for_reads_and_writes() {
        let registry = Registry::new();
        std::thread::scope(|scope| {
            let families = &registry.families;
            let handle = scope.spawn(move || {
                let _guard = families.write().unwrap();
                panic!("poison the family map");
            });
            assert!(handle.join().is_err());
        });
        assert!(registry.families.is_poisoned());
        let labels = job_labels("daily", "applovin_max", "completed");
        assert_eq!(
            registry
                .increment(&catalog::JOB_RUNS_TOTAL, &labels)
                .unwrap(),
            1
        );
        assert_eq!(
            registry
                .counter_value(&catalog::JOB_RUNS_TOTAL, &labels)
                .unwrap(),
            1
        );
        assert_eq!(registry.snapshot().len(), 1);
        assert!(registry
            .family_snapshot(catalog::JOB_RUNS_TOTAL.name)
            .is_some());
        assert_eq!(registry.lock_waits(), LockWaits::default());
    }

    #[test]
    fn blocked_reads_and_writes_record_the_wait_and_complete() {
        let stats = LockStats::default();
        let lock = RwLock::new(0u64);
        let (held_tx, held_rx) = mpsc::channel();
        std::thread::scope(|scope| {
            let reader = scope.spawn(|| {
                let guard = lock.read().unwrap();
                held_tx.send(()).unwrap();
                std::thread::sleep(Duration::from_millis(250));
                drop(guard);
            });
            held_rx.recv().unwrap();
            {
                let mut guard = write_or_wait(&lock, &stats);
                *guard = 7;
            }
            reader.join().unwrap();
        });
        assert_eq!(*lock.read().unwrap(), 7);
        assert_eq!(stats.write_waits.load(Ordering::Relaxed), 1);
        assert_eq!(stats.read_waits.load(Ordering::Relaxed), 0);

        let stats = LockStats::default();
        let lock = RwLock::new(7u64);
        let (held_tx, held_rx) = mpsc::channel();
        std::thread::scope(|scope| {
            let writer = scope.spawn(|| {
                let mut guard = lock.write().unwrap();
                *guard = 9;
                held_tx.send(()).unwrap();
                std::thread::sleep(Duration::from_millis(250));
                drop(guard);
            });
            held_rx.recv().unwrap();
            assert_eq!(*read_or_wait(&lock, &stats), 9);
            writer.join().unwrap();
        });
        assert_eq!(stats.read_waits.load(Ordering::Relaxed), 1);
        assert_eq!(stats.write_waits.load(Ordering::Relaxed), 0);
    }

    static CONFLICT_LABELS: &[LabelSpec] = &[LabelSpec {
        name: "outcome",
        vocab: Vocab::Values(&["completed"]),
    }];
    static CONFLICT_EXTRA_LABELS: &[LabelSpec] = &[
        LabelSpec {
            name: "outcome",
            vocab: Vocab::Values(&["completed"]),
        },
        LabelSpec {
            name: "platform",
            vocab: Vocab::Values(&["admob"]),
        },
    ];
    static CONFLICT_COUNTER: MetricSpec = MetricSpec {
        name: "apex_conflict_probe_total",
        help: "Conflict probe.",
        kind: MetricKind::Counter,
        labels: CONFLICT_LABELS,
    };
    static CONFLICT_COUNTER_EXTRA: MetricSpec = MetricSpec {
        name: "apex_conflict_probe_total",
        help: "Conflict probe.",
        kind: MetricKind::Counter,
        labels: CONFLICT_EXTRA_LABELS,
    };

    #[test]
    fn conflicting_declaration_is_refused_on_the_read_hit_path() {
        let registry = Registry::new();
        registry
            .increment(&CONFLICT_COUNTER, &[("outcome", "completed")])
            .unwrap();
        let error = registry
            .increment(
                &CONFLICT_COUNTER_EXTRA,
                &[("outcome", "completed"), ("platform", "admob")],
            )
            .expect_err("conflicting declaration");
        assert_eq!(
            error,
            WriteError::ConflictingDeclaration {
                metric: "apex_conflict_probe_total"
            }
        );
        assert_eq!(
            registry
                .counter_value(&CONFLICT_COUNTER, &[("outcome", "completed")])
                .unwrap(),
            1
        );
    }

    #[test]
    fn conflicting_declaration_is_refused_on_the_entry_occupied_path() {
        let stats = Arc::new(LockStats::default());
        let mut families: HashMap<&'static str, Arc<Family>> = HashMap::new();
        families.insert(
            CONFLICT_COUNTER.name,
            Arc::new(Family::new(&CONFLICT_COUNTER, Arc::clone(&stats))),
        );
        let error = resolve_family(
            families.entry(CONFLICT_COUNTER_EXTRA.name),
            &CONFLICT_COUNTER_EXTRA,
            &stats,
        )
        .expect_err("occupied entry with a different declaration");
        assert_eq!(
            error,
            WriteError::ConflictingDeclaration {
                metric: "apex_conflict_probe_total"
            }
        );
        let resolved = resolve_family(
            families.entry(CONFLICT_COUNTER.name),
            &CONFLICT_COUNTER,
            &stats,
        )
        .expect("occupied entry with the same declaration");
        assert!(Arc::ptr_eq(
            &resolved,
            families.get(CONFLICT_COUNTER.name).unwrap()
        ));
        let inserted = resolve_family(
            families.entry(CONFLICT_COUNTER_EXTRA.name),
            &CONFLICT_COUNTER_EXTRA,
            &stats,
        )
        .expect_err("the conflicting entry remains the declared one");
        assert_eq!(
            inserted,
            WriteError::ConflictingDeclaration {
                metric: "apex_conflict_probe_total"
            }
        );
        assert_eq!(families.len(), 1);
    }

    static CACHE_PROBE_LABELS: &[LabelSpec] = &[LabelSpec {
        name: "route_template",
        vocab: Vocab::RouteTemplate,
    }];
    static CACHE_PROBE: MetricSpec = MetricSpec {
        name: "apex_cache_probe_total",
        help: "Handle cache probe.",
        kind: MetricKind::Counter,
        labels: CACHE_PROBE_LABELS,
    };

    /// A unique, valid route-template parameter for `index` (three lowercase
    /// letters, bijective over 26^3 values).
    fn cache_probe_route(mut index: usize) -> String {
        let mut slots = [b'a'; 3];
        for slot in slots.iter_mut().rev() {
            *slot = b'a' + (index % 26) as u8;
            index /= 26;
        }
        format!(
            "/x/{{{}}}",
            String::from_utf8(slots.to_vec()).expect("ascii")
        )
    }

    #[test]
    fn ten_thousand_increments_share_one_handle_and_render_identically() {
        let registry = Registry::new();
        let metric = &catalog::JOB_RUNS_TOTAL;
        let labels = job_labels("daily", "applovin_max", "completed");
        for _ in 0..10_000 {
            registry.increment(metric, &labels).expect("increment");
        }
        assert_eq!(
            registry.handle_cache_len(),
            1,
            "one key keeps exactly one cached handle"
        );
        assert_eq!(
            registry
                .family_snapshot(metric.name)
                .expect("family")
                .series
                .len(),
            1,
            "one key keeps exactly one stored series"
        );
        assert_eq!(
            registry.counter_value(metric, &labels).expect("read"),
            10_000
        );
        assert_eq!(
            registry.render_prometheus(),
            "# HELP apex_job_runs_total Scheduled runner job runs by outcome.\n\
             # TYPE apex_job_runs_total counter\n\
             apex_job_runs_total{job_type=\"daily\",platform=\"applovin_max\",outcome=\"completed\"} 10000\n"
        );
    }

    #[test]
    fn the_handle_cache_is_bounded_and_eviction_loses_no_series() {
        let registry = Registry::new();
        let total = HANDLE_CACHE_CAP + 500;
        for index in 0..total {
            let route = cache_probe_route(index);
            registry
                .increment(&CACHE_PROBE, &[("route_template", route.as_str())])
                .expect("probe increment");
        }
        assert!(
            registry.handle_cache_len() <= HANDLE_CACHE_CAP,
            "the cache is bounded at {HANDLE_CACHE_CAP}, saw {}",
            registry.handle_cache_len()
        );
        // Eviction never removes a series from the authoritative map, so the
        // exposition still carries every observed key.
        assert_eq!(
            registry
                .family_snapshot(CACHE_PROBE.name)
                .expect("probe family")
                .series
                .len(),
            total,
            "every observed series survives cache eviction"
        );
        // The most recently observed key is still served from the cache and
        // accumulates normally.
        let recent = cache_probe_route(total - 1);
        assert_eq!(
            registry
                .increment(&CACHE_PROBE, &[("route_template", recent.as_str())])
                .expect("second increment"),
            2
        );
    }
}
