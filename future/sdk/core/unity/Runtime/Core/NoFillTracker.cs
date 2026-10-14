using System;
using System.Collections.Generic;
using System.Collections.Concurrent;
using System.Linq;
using System.Threading;

namespace RivalApex.Mediation
{
    /// <summary>
    /// Reasons for ad no-fill
    /// </summary>
    public enum NoFillReason
    {
        Timeout,
        NoInventory,
        NetworkError,
        PolicyViolation,
        FrequencyCap,
        GeographicRestriction,
        BudgetExhausted,
        MalformedResponse,
        ServerError,
        Unknown
    }

    /// <summary>
    /// A single no-fill event
    /// </summary>
    public readonly struct NoFillEvent
    {
        public string SourceId { get; }
        public string PlacementId { get; }
        public NoFillReason Reason { get; }
        public DateTime Timestamp { get; }
        public long LatencyMs { get; }
        public IReadOnlyDictionary<string, string> Metadata { get; }

        public NoFillEvent(
            string sourceId,
            string placementId,
            NoFillReason reason,
            DateTime? timestamp = null,
            long latencyMs = 0,
            Dictionary<string, string> metadata = null)
        {
            SourceId = sourceId ?? throw new ArgumentNullException(nameof(sourceId));
            PlacementId = placementId ?? throw new ArgumentNullException(nameof(placementId));
            Reason = reason;
            Timestamp = timestamp ?? DateTime.UtcNow;
            LatencyMs = latencyMs;
            Metadata = metadata ?? new Dictionary<string, string>();
        }
    }

    /// <summary>
    /// Statistics for no-fill events
    /// </summary>
    public class NoFillStats
    {
        public int TotalNoFills { get; set; }
        public double NoFillRate { get; set; }
        public double AverageLatencyMs { get; set; }
        public Dictionary<NoFillReason, int> TopReasons { get; set; }
        public Dictionary<string, int> TopSources { get; set; }
        public Dictionary<string, int> TopPlacements { get; set; }
        public Dictionary<int, int> HourlyBreakdown { get; set; }
        public Dictionary<int, int> DailyBreakdown { get; set; }
    }

    /// <summary>
    /// Detected no-fill pattern
    /// </summary>
    public class NoFillPattern
    {
        public PatternType Type { get; set; }
        public Severity PatternSeverity { get; set; }
        public string Description { get; set; }
        public DateTime DetectedAt { get; set; }
        public string AffectedSourceId { get; set; }
        public string AffectedPlacementId { get; set; }

        public enum PatternType
        {
            ElevatedRate,
            SourceSpecific,
            PlacementSpecific,
            ConsecutiveFailures
        }

        public enum Severity
        {
            Low,
            Medium,
            High,
            Critical
        }
    }

    /// <summary>
    /// Configuration for NoFillTracker
    /// </summary>
    public class NoFillTrackerConfig
    {
        public int MaxEventsRetained { get; set; } = 10000;
        public int MaxRetentionHours { get; set; } = 24;
        public double ElevatedRateThreshold { get; set; } = 0.5;
        public bool PatternDetectionEnabled { get; set; } = true;
        public int ConsecutiveFailureThreshold { get; set; } = 5;

        public static NoFillTrackerConfig Default => new NoFillTrackerConfig();
    }

    /// <summary>
    /// Pattern listener delegate
    /// </summary>
    public delegate void NoFillPatternHandler(NoFillPattern pattern);

    /// <summary>
    /// Tracks no-fill events for analytics and pattern detection
    /// </summary>
    public sealed class NoFillTracker
    {
        private static readonly Lazy<NoFillTracker> _instance =
            new Lazy<NoFillTracker>(() => new NoFillTracker());

        public static NoFillTracker Shared => _instance.Value;

        private NoFillTrackerConfig _config;
        private readonly List<NoFillEvent> _events;
        private readonly object _lock = new object();

        private int _totalNoFills;
        private long _totalLatencyMs;

        private readonly ConcurrentDictionary<int, int> _hourlyNoFills;
        private readonly ConcurrentDictionary<int, int> _dailyNoFills;
        private readonly ConcurrentDictionary<string, int> _noFillsBySource;
        private readonly ConcurrentDictionary<string, int> _noFillsByPlacement;
        private readonly ConcurrentDictionary<NoFillReason, int> _noFillsByReason;
        private readonly ConcurrentDictionary<string, int> _consecutiveNoFillsBySource;
        private readonly List<NoFillPattern> _detectedPatterns;
        private readonly List<NoFillPatternHandler> _patternListeners;

        private NoFillTracker()
        {
            _config = NoFillTrackerConfig.Default;
            _events = new List<NoFillEvent>();
            _hourlyNoFills = new ConcurrentDictionary<int, int>();
            _dailyNoFills = new ConcurrentDictionary<int, int>();
            _noFillsBySource = new ConcurrentDictionary<string, int>();
            _noFillsByPlacement = new ConcurrentDictionary<string, int>();
            _noFillsByReason = new ConcurrentDictionary<NoFillReason, int>();
            _consecutiveNoFillsBySource = new ConcurrentDictionary<string, int>();
            _detectedPatterns = new List<NoFillPattern>();
            _patternListeners = new List<NoFillPatternHandler>();
        }

        /// <summary>
        /// Record a no-fill event
        /// </summary>
        public void RecordNoFill(
            string sourceId,
            string placementId,
            NoFillReason reason,
            long latencyMs = 0,
            Dictionary<string, string> metadata = null)
        {
            var evt = new NoFillEvent(
                sourceId,
                placementId,
                reason,
                DateTime.UtcNow,
                latencyMs,
                metadata);

            lock (_lock)
            {
                _events.Add(evt);
                Interlocked.Increment(ref _totalNoFills);
                Interlocked.Add(ref _totalLatencyMs, latencyMs);

                _noFillsBySource.AddOrUpdate(sourceId, 1, (_, v) => v + 1);
                _noFillsByPlacement.AddOrUpdate(placementId, 1, (_, v) => v + 1);
                _noFillsByReason.AddOrUpdate(reason, 1, (_, v) => v + 1);

                int hour = evt.Timestamp.Hour;
                _hourlyNoFills.AddOrUpdate(hour, 1, (_, v) => v + 1);

                int dayOfWeek = (int)evt.Timestamp.DayOfWeek;
                _dailyNoFills.AddOrUpdate(dayOfWeek, 1, (_, v) => v + 1);

                _consecutiveNoFillsBySource.AddOrUpdate(sourceId, 1, (_, v) => v + 1);

                CleanupIfNeeded();

                if (_config.PatternDetectionEnabled)
                {
                    DetectPatternsForEvent(evt);
                }
            }
        }

        /// <summary>
        /// Record a successful fill
        /// </summary>
        public void RecordFill(string sourceId)
        {
            _consecutiveNoFillsBySource.TryUpdate(sourceId, 0, 
                _consecutiveNoFillsBySource.GetValueOrDefault(sourceId, 0));
        }

        /// <summary>
        /// Get current statistics
        /// </summary>
        public NoFillStats GetStats()
        {
            lock (_lock)
            {
                int total = _totalNoFills;
                double avgLatency = total > 0 
                    ? (double)_totalLatencyMs / total 
                    : 0.0;

                DateTime oneHourAgo = DateTime.UtcNow.AddHours(-1);
                int recentCount = _events.Count(e => e.Timestamp >= oneHourAgo);
                double ratePerMinute = recentCount / 60.0;

                return new NoFillStats
                {
                    TotalNoFills = total,
                    NoFillRate = ratePerMinute,
                    AverageLatencyMs = avgLatency,
                    TopReasons = _noFillsByReason.OrderByDescending(x => x.Value)
                        .Take(5).ToDictionary(x => x.Key, x => x.Value),
                    TopSources = _noFillsBySource.OrderByDescending(x => x.Value)
                        .Take(5).ToDictionary(x => x.Key, x => x.Value),
                    TopPlacements = _noFillsByPlacement.OrderByDescending(x => x.Value)
                        .Take(5).ToDictionary(x => x.Key, x => x.Value),
                    HourlyBreakdown = _hourlyNoFills.ToDictionary(x => x.Key, x => x.Value),
                    DailyBreakdown = _dailyNoFills.ToDictionary(x => x.Key, x => x.Value)
                };
            }
        }

        /// <summary>
        /// Get hourly breakdown
        /// </summary>
        public Dictionary<int, int> GetHourlyBreakdown()
        {
            return _hourlyNoFills.ToDictionary(x => x.Key, x => x.Value);
        }

        /// <summary>
        /// Get daily breakdown
        /// </summary>
        public Dictionary<int, int> GetDailyBreakdown()
        {
            return _dailyNoFills.ToDictionary(x => x.Key, x => x.Value);
        }

        /// <summary>
        /// Get no-fills by source
        /// </summary>
        public Dictionary<string, int> GetNoFillsBySource()
        {
            return _noFillsBySource.ToDictionary(x => x.Key, x => x.Value);
        }

        /// <summary>
        /// Get no-fills by placement
        /// </summary>
        public Dictionary<string, int> GetNoFillsByPlacement()
        {
            return _noFillsByPlacement.ToDictionary(x => x.Key, x => x.Value);
        }

        /// <summary>
        /// Get no-fills by reason
        /// </summary>
        public Dictionary<NoFillReason, int> GetNoFillsByReason()
        {
            return _noFillsByReason.ToDictionary(x => x.Key, x => x.Value);
        }

        /// <summary>
        /// Get detected patterns
        /// </summary>
        public List<NoFillPattern> GetDetectedPatterns()
        {
            lock (_lock)
            {
                return new List<NoFillPattern>(_detectedPatterns);
            }
        }

        /// <summary>
        /// Add pattern listener
        /// </summary>
        public void AddPatternListener(NoFillPatternHandler handler)
        {
            lock (_lock)
            {
                _patternListeners.Add(handler);
            }
        }

        /// <summary>
        /// Remove pattern listener
        /// </summary>
        public void RemovePatternListener(NoFillPatternHandler handler)
        {
            lock (_lock)
            {
                _patternListeners.Remove(handler);
            }
        }

        /// <summary>
        /// Get recent events
        /// </summary>
        public List<NoFillEvent> GetRecentEvents(int count = 100)
        {
            lock (_lock)
            {
                return _events.TakeLast(count).ToList();
            }
        }

        /// <summary>
        /// Clear all data
        /// </summary>
        public void Clear()
        {
            lock (_lock)
            {
                _events.Clear();
                _totalNoFills = 0;
                _totalLatencyMs = 0;
                _hourlyNoFills.Clear();
                _dailyNoFills.Clear();
                _noFillsBySource.Clear();
                _noFillsByPlacement.Clear();
                _noFillsByReason.Clear();
                _consecutiveNoFillsBySource.Clear();
                _detectedPatterns.Clear();
            }
        }

        /// <summary>
        /// Update configuration
        /// </summary>
        public void UpdateConfiguration(NoFillTrackerConfig config)
        {
            lock (_lock)
            {
                _config = config ?? throw new ArgumentNullException(nameof(config));
            }
        }

        private void CleanupIfNeeded()
        {
            DateTime cutoff = DateTime.UtcNow.AddHours(-_config.MaxRetentionHours);
            _events.RemoveAll(e => e.Timestamp < cutoff);

            while (_events.Count > _config.MaxEventsRetained)
            {
                _events.RemoveAt(0);
            }
        }

        private void DetectPatternsForEvent(NoFillEvent evt)
        {
            // Check for consecutive failures
            if (_consecutiveNoFillsBySource.TryGetValue(evt.SourceId, out int consecutive))
            {
                if (consecutive >= _config.ConsecutiveFailureThreshold)
                {
                    var severity = consecutive >= 10 
                        ? NoFillPattern.Severity.High 
                        : NoFillPattern.Severity.Medium;

                    var pattern = new NoFillPattern
                    {
                        Type = NoFillPattern.PatternType.ConsecutiveFailures,
                        PatternSeverity = severity,
                        Description = $"Source {evt.SourceId} has {consecutive} consecutive no-fills",
                        DetectedAt = DateTime.UtcNow,
                        AffectedSourceId = evt.SourceId
                    };
                    AddPattern(pattern);
                }
            }

            // Check for elevated source rate
            if (_noFillsBySource.TryGetValue(evt.SourceId, out int sourceTotal))
            {
                int total = Math.Max(1, _totalNoFills);
                double sourceRate = (double)sourceTotal / total;

                if (sourceRate > _config.ElevatedRateThreshold && sourceTotal > 10)
                {
                    var severity = sourceRate > 0.8 
                        ? NoFillPattern.Severity.Critical 
                        : NoFillPattern.Severity.Medium;

                    var pattern = new NoFillPattern
                    {
                        Type = NoFillPattern.PatternType.SourceSpecific,
                        PatternSeverity = severity,
                        Description = $"Source {evt.SourceId} has {(int)(sourceRate * 100)}% no-fill rate",
                        DetectedAt = DateTime.UtcNow,
                        AffectedSourceId = evt.SourceId
                    };
                    AddPattern(pattern);
                }
            }
        }

        private void AddPattern(NoFillPattern pattern)
        {
            DateTime recentCutoff = DateTime.UtcNow.AddMinutes(-5);
            bool isDuplicate = _detectedPatterns.Any(existing =>
                existing.Type == pattern.Type &&
                existing.AffectedSourceId == pattern.AffectedSourceId &&
                existing.DetectedAt > recentCutoff);

            if (!isDuplicate)
            {
                _detectedPatterns.Add(pattern);

                DateTime oneDayAgo = DateTime.UtcNow.AddDays(-1);
                _detectedPatterns.RemoveAll(p => p.DetectedAt < oneDayAgo);

                foreach (var listener in _patternListeners)
                {
                    try
                    {
                        listener(pattern);
                    }
                    catch
                    {
                        // Ignore listener errors
                    }
                }
            }
        }
    }
}
