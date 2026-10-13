using System;
using System.Collections.Concurrent;
using System.Collections.Generic;
using System.Linq;
using System.Text;
using System.Threading;
using System.Threading.Tasks;

namespace Apex.Mediation.Core.Metering
{
    /// <summary>
    /// Types of billable usage events
    /// </summary>
    public enum UsageEventType
    {
        AdRequest,
        AdImpression,
        AdClick,
        AdVideoStart,
        AdVideoComplete,
        AdRevenue,
        ApiCall,
        CacheHit,
        CacheMiss,
        Error
    }

    /// <summary>
    /// A single usage event for metering
    /// </summary>
    public struct UsageEvent
    {
        public UsageEventType Type { get; set; }
        public DateTime Timestamp { get; set; }
        public string PlacementId { get; set; }
        public string AdapterId { get; set; }
        public string AdFormat { get; set; }
        public double? RevenueAmount { get; set; }
        public Dictionary<string, object> Metadata { get; set; }

        public static UsageEvent Create(UsageEventType type, string placementId = null, 
            string adapterId = null, string adFormat = null, double? revenueAmount = null)
        {
            return new UsageEvent
            {
                Type = type,
                Timestamp = DateTime.UtcNow,
                PlacementId = placementId,
                AdapterId = adapterId,
                AdFormat = adFormat,
                RevenueAmount = revenueAmount
            };
        }
    }

    /// <summary>
    /// Aggregated usage metrics for a period
    /// </summary>
    public struct UsageMetrics
    {
        public long AdRequests { get; set; }
        public long AdImpressions { get; set; }
        public long AdClicks { get; set; }
        public long VideoStarts { get; set; }
        public long VideoCompletes { get; set; }
        public double TotalRevenue { get; set; }
        public long ApiCalls { get; set; }
        public long CacheHits { get; set; }
        public long CacheMisses { get; set; }
        public long Errors { get; set; }
        public DateTime PeriodStart { get; set; }
        public DateTime PeriodEnd { get; set; }
    }

    /// <summary>
    /// Metrics broken down by dimension
    /// </summary>
    public struct UsageBreakdown
    {
        public Dictionary<string, UsageMetrics> ByPlacement { get; set; }
        public Dictionary<string, UsageMetrics> ByAdapter { get; set; }
        public Dictionary<string, UsageMetrics> ByAdFormat { get; set; }
    }

    /// <summary>
    /// Configuration for the usage meter
    /// </summary>
    public class MeteringConfig
    {
        public TimeSpan FlushInterval { get; set; } = TimeSpan.FromMinutes(1);
        public int MaxEventsBeforeFlush { get; set; } = 1000;
        public bool EnableLocalStorage { get; set; } = true;
        public bool EnableRemoteReporting { get; set; } = true;
        public double SamplingRate { get; set; } = 1.0;
    }

    /// <summary>
    /// Interface for reporting metrics to remote service
    /// </summary>
    public interface IMeteringReporter
    {
        Task<bool> ReportAsync(UsageMetrics metrics, UsageBreakdown breakdown, 
            CancellationToken cancellationToken = default);
    }

    /// <summary>
    /// Dimension-specific metrics
    /// </summary>
    internal class DimensionMetrics
    {
        private long _adRequests;
        private long _adImpressions;
        private long _adClicks;
        private long _videoStarts;
        private long _videoCompletes;
        private double _totalRevenue;
        private long _apiCalls;
        private long _cacheHits;
        private long _cacheMisses;
        private long _errors;
        private readonly object _revenueLock = new object();

        public void IncrementAdRequests() => Interlocked.Increment(ref _adRequests);
        public void IncrementAdImpressions() => Interlocked.Increment(ref _adImpressions);
        public void IncrementAdClicks() => Interlocked.Increment(ref _adClicks);
        public void IncrementVideoStarts() => Interlocked.Increment(ref _videoStarts);
        public void IncrementVideoCompletes() => Interlocked.Increment(ref _videoCompletes);
        public void IncrementApiCalls() => Interlocked.Increment(ref _apiCalls);
        public void IncrementCacheHits() => Interlocked.Increment(ref _cacheHits);
        public void IncrementCacheMisses() => Interlocked.Increment(ref _cacheMisses);
        public void IncrementErrors() => Interlocked.Increment(ref _errors);

        public void AddRevenue(double amount)
        {
            lock (_revenueLock)
            {
                _totalRevenue += amount;
            }
        }

        public UsageMetrics ToMetrics(DateTime periodStart, DateTime periodEnd)
        {
            return new UsageMetrics
            {
                AdRequests = Interlocked.Read(ref _adRequests),
                AdImpressions = Interlocked.Read(ref _adImpressions),
                AdClicks = Interlocked.Read(ref _adClicks),
                VideoStarts = Interlocked.Read(ref _videoStarts),
                VideoCompletes = Interlocked.Read(ref _videoCompletes),
                TotalRevenue = _totalRevenue,
                ApiCalls = Interlocked.Read(ref _apiCalls),
                CacheHits = Interlocked.Read(ref _cacheHits),
                CacheMisses = Interlocked.Read(ref _cacheMisses),
                Errors = Interlocked.Read(ref _errors),
                PeriodStart = periodStart,
                PeriodEnd = periodEnd
            };
        }

        public void Reset()
        {
            Interlocked.Exchange(ref _adRequests, 0);
            Interlocked.Exchange(ref _adImpressions, 0);
            Interlocked.Exchange(ref _adClicks, 0);
            Interlocked.Exchange(ref _videoStarts, 0);
            Interlocked.Exchange(ref _videoCompletes, 0);
            lock (_revenueLock) { _totalRevenue = 0; }
            Interlocked.Exchange(ref _apiCalls, 0);
            Interlocked.Exchange(ref _cacheHits, 0);
            Interlocked.Exchange(ref _cacheMisses, 0);
            Interlocked.Exchange(ref _errors, 0);
        }
    }

    /// <summary>
    /// Tracks and reports billable usage events
    /// </summary>
    public class UsageMeter : IDisposable
    {
        private readonly MeteringConfig _config;
        private readonly IMeteringReporter _reporter;
        
        // Global counters
        private readonly DimensionMetrics _globalMetrics = new DimensionMetrics();
        
        // Dimensional breakdowns
        private readonly ConcurrentDictionary<string, DimensionMetrics> _placementMetrics = new();
        private readonly ConcurrentDictionary<string, DimensionMetrics> _adapterMetrics = new();
        private readonly ConcurrentDictionary<string, DimensionMetrics> _formatMetrics = new();
        
        // Pending events
        private readonly List<UsageEvent> _pendingEvents = new();
        private readonly object _eventsLock = new object();
        
        // Period tracking
        private DateTime _periodStart = DateTime.UtcNow;
        
        // Flush timer
        private readonly CancellationTokenSource _cts = new();
        private Task _flushTask;
        private bool _isRunning;
        private readonly Random _random = new();

        public UsageMeter(MeteringConfig config = null, IMeteringReporter reporter = null)
        {
            _config = config ?? new MeteringConfig();
            _reporter = reporter;
        }

        /// <summary>
        /// Start the metering service
        /// </summary>
        public void Start()
        {
            if (_isRunning) return;
            _isRunning = true;
            _periodStart = DateTime.UtcNow;

            _flushTask = Task.Run(async () =>
            {
                while (!_cts.Token.IsCancellationRequested && _isRunning)
                {
                    await Task.Delay(_config.FlushInterval, _cts.Token).ConfigureAwait(false);
                    await FlushAsync(_cts.Token).ConfigureAwait(false);
                }
            });
        }

        /// <summary>
        /// Stop the metering service
        /// </summary>
        public void Stop()
        {
            _isRunning = false;
            _cts.Cancel();
        }

        /// <summary>
        /// Record a usage event
        /// </summary>
        public void Record(UsageEvent evt)
        {
            // Apply sampling
            if (_config.SamplingRate < 1.0 && _random.NextDouble() > _config.SamplingRate)
            {
                return;
            }

            // Update global counters
            UpdateMetrics(_globalMetrics, evt);

            // Update dimensional breakdowns
            if (!string.IsNullOrEmpty(evt.PlacementId))
            {
                var metrics = _placementMetrics.GetOrAdd(evt.PlacementId, _ => new DimensionMetrics());
                UpdateMetrics(metrics, evt);
            }

            if (!string.IsNullOrEmpty(evt.AdapterId))
            {
                var metrics = _adapterMetrics.GetOrAdd(evt.AdapterId, _ => new DimensionMetrics());
                UpdateMetrics(metrics, evt);
            }

            if (!string.IsNullOrEmpty(evt.AdFormat))
            {
                var metrics = _formatMetrics.GetOrAdd(evt.AdFormat, _ => new DimensionMetrics());
                UpdateMetrics(metrics, evt);
            }

            // Queue event for storage
            if (_config.EnableLocalStorage)
            {
                lock (_eventsLock)
                {
                    _pendingEvents.Add(evt);
                    if (_pendingEvents.Count >= _config.MaxEventsBeforeFlush)
                    {
                        _ = FlushAsync(CancellationToken.None);
                    }
                }
            }
        }

        private void UpdateMetrics(DimensionMetrics metrics, UsageEvent evt)
        {
            switch (evt.Type)
            {
                case UsageEventType.AdRequest: metrics.IncrementAdRequests(); break;
                case UsageEventType.AdImpression: metrics.IncrementAdImpressions(); break;
                case UsageEventType.AdClick: metrics.IncrementAdClicks(); break;
                case UsageEventType.AdVideoStart: metrics.IncrementVideoStarts(); break;
                case UsageEventType.AdVideoComplete: metrics.IncrementVideoCompletes(); break;
                case UsageEventType.AdRevenue:
                    if (evt.RevenueAmount.HasValue) metrics.AddRevenue(evt.RevenueAmount.Value);
                    break;
                case UsageEventType.ApiCall: metrics.IncrementApiCalls(); break;
                case UsageEventType.CacheHit: metrics.IncrementCacheHits(); break;
                case UsageEventType.CacheMiss: metrics.IncrementCacheMisses(); break;
                case UsageEventType.Error: metrics.IncrementErrors(); break;
            }
        }

        // Convenience recording methods
        public void RecordRequest(string placementId, string adapterId = null, string adFormat = null) =>
            Record(UsageEvent.Create(UsageEventType.AdRequest, placementId, adapterId, adFormat));

        public void RecordImpression(string placementId, string adapterId, string adFormat = null) =>
            Record(UsageEvent.Create(UsageEventType.AdImpression, placementId, adapterId, adFormat));

        public void RecordClick(string placementId, string adapterId, string adFormat = null) =>
            Record(UsageEvent.Create(UsageEventType.AdClick, placementId, adapterId, adFormat));

        public void RecordRevenue(string placementId, string adapterId, double amount) =>
            Record(UsageEvent.Create(UsageEventType.AdRevenue, placementId, adapterId, null, amount));

        public void RecordVideoStart(string placementId, string adapterId) =>
            Record(UsageEvent.Create(UsageEventType.AdVideoStart, placementId, adapterId, "video"));

        public void RecordVideoComplete(string placementId, string adapterId) =>
            Record(UsageEvent.Create(UsageEventType.AdVideoComplete, placementId, adapterId, "video"));

        public void RecordCacheHit(string placementId = null) =>
            Record(UsageEvent.Create(UsageEventType.CacheHit, placementId));

        public void RecordCacheMiss(string placementId = null) =>
            Record(UsageEvent.Create(UsageEventType.CacheMiss, placementId));

        public void RecordError(string placementId = null, string adapterId = null) =>
            Record(UsageEvent.Create(UsageEventType.Error, placementId, adapterId));

        /// <summary>
        /// Get current metrics snapshot
        /// </summary>
        public UsageMetrics GetMetrics()
        {
            return _globalMetrics.ToMetrics(_periodStart, DateTime.UtcNow);
        }

        /// <summary>
        /// Get breakdown by dimensions
        /// </summary>
        public UsageBreakdown GetBreakdown()
        {
            var now = DateTime.UtcNow;
            return new UsageBreakdown
            {
                ByPlacement = new Dictionary<string, UsageMetrics>(
                    _placementMetrics.Select(kv => new KeyValuePair<string, UsageMetrics>(
                        kv.Key, kv.Value.ToMetrics(_periodStart, now)))),
                ByAdapter = new Dictionary<string, UsageMetrics>(
                    _adapterMetrics.Select(kv => new KeyValuePair<string, UsageMetrics>(
                        kv.Key, kv.Value.ToMetrics(_periodStart, now)))),
                ByAdFormat = new Dictionary<string, UsageMetrics>(
                    _formatMetrics.Select(kv => new KeyValuePair<string, UsageMetrics>(
                        kv.Key, kv.Value.ToMetrics(_periodStart, now))))
            };
        }

        /// <summary>
        /// Get click-through rate
        /// </summary>
        public double GetCTR()
        {
            var metrics = GetMetrics();
            if (metrics.AdImpressions == 0) return 0;
            return (double)metrics.AdClicks / metrics.AdImpressions;
        }

        /// <summary>
        /// Get fill rate
        /// </summary>
        public double GetFillRate()
        {
            var metrics = GetMetrics();
            if (metrics.AdRequests == 0) return 0;
            return (double)metrics.AdImpressions / metrics.AdRequests;
        }

        /// <summary>
        /// Get video completion rate
        /// </summary>
        public double GetVideoCompletionRate()
        {
            var metrics = GetMetrics();
            if (metrics.VideoStarts == 0) return 0;
            return (double)metrics.VideoCompletes / metrics.VideoStarts;
        }

        /// <summary>
        /// Get cache hit rate
        /// </summary>
        public double GetCacheHitRate()
        {
            var metrics = GetMetrics();
            var total = metrics.CacheHits + metrics.CacheMisses;
            if (total == 0) return 0;
            return (double)metrics.CacheHits / total;
        }

        /// <summary>
        /// Get effective CPM
        /// </summary>
        public double GetEffectiveCPM()
        {
            var metrics = GetMetrics();
            if (metrics.AdImpressions == 0) return 0;
            return (metrics.TotalRevenue / metrics.AdImpressions) * 1000.0;
        }

        /// <summary>
        /// Flush metrics to reporter
        /// </summary>
        public async Task<bool> FlushAsync(CancellationToken cancellationToken = default)
        {
            if (!_config.EnableRemoteReporting || _reporter == null)
            {
                return true;
            }

            var metrics = GetMetrics();
            var breakdown = GetBreakdown();

            try
            {
                var success = await _reporter.ReportAsync(metrics, breakdown, cancellationToken)
                    .ConfigureAwait(false);
                if (success)
                {
                    Reset();
                }
                return success;
            }
            catch
            {
                return false;
            }
        }

        /// <summary>
        /// Reset all counters
        /// </summary>
        public void Reset()
        {
            _globalMetrics.Reset();
            foreach (var metrics in _placementMetrics.Values) metrics.Reset();
            foreach (var metrics in _adapterMetrics.Values) metrics.Reset();
            foreach (var metrics in _formatMetrics.Values) metrics.Reset();

            lock (_eventsLock)
            {
                _pendingEvents.Clear();
            }

            _periodStart = DateTime.UtcNow;
        }

        /// <summary>
        /// Export metrics as JSON
        /// </summary>
        public string ExportAsJSON()
        {
            var metrics = GetMetrics();
            var sb = new StringBuilder();
            sb.AppendLine("{");
            sb.AppendLine("  \"metrics\": {");
            sb.AppendLine($"    \"adRequests\": {metrics.AdRequests},");
            sb.AppendLine($"    \"adImpressions\": {metrics.AdImpressions},");
            sb.AppendLine($"    \"adClicks\": {metrics.AdClicks},");
            sb.AppendLine($"    \"videoStarts\": {metrics.VideoStarts},");
            sb.AppendLine($"    \"videoCompletes\": {metrics.VideoCompletes},");
            sb.AppendLine($"    \"totalRevenue\": {metrics.TotalRevenue},");
            sb.AppendLine($"    \"apiCalls\": {metrics.ApiCalls},");
            sb.AppendLine($"    \"cacheHits\": {metrics.CacheHits},");
            sb.AppendLine($"    \"cacheMisses\": {metrics.CacheMisses},");
            sb.AppendLine($"    \"errors\": {metrics.Errors}");
            sb.AppendLine("  },");
            sb.AppendLine("  \"computed\": {");
            sb.AppendLine($"    \"ctr\": {GetCTR()},");
            sb.AppendLine($"    \"fillRate\": {GetFillRate()},");
            sb.AppendLine($"    \"videoCompletionRate\": {GetVideoCompletionRate()},");
            sb.AppendLine($"    \"cacheHitRate\": {GetCacheHitRate()},");
            sb.AppendLine($"    \"effectiveCPM\": {GetEffectiveCPM()}");
            sb.AppendLine("  }");
            sb.AppendLine("}");
            return sb.ToString();
        }

        public void Dispose()
        {
            Stop();
            _cts.Dispose();
        }
    }

    /// <summary>
    /// Builder for UsageMeter
    /// </summary>
    public class UsageMeterBuilder
    {
        private MeteringConfig _config = new MeteringConfig();
        private IMeteringReporter _reporter;

        public UsageMeterBuilder WithConfig(MeteringConfig config)
        {
            _config = config;
            return this;
        }

        public UsageMeterBuilder WithReporter(IMeteringReporter reporter)
        {
            _reporter = reporter;
            return this;
        }

        public UsageMeterBuilder WithFlushInterval(TimeSpan interval)
        {
            _config.FlushInterval = interval;
            return this;
        }

        public UsageMeterBuilder WithSamplingRate(double rate)
        {
            _config.SamplingRate = Math.Max(0, Math.Min(1, rate));
            return this;
        }

        public UsageMeter Build()
        {
            return new UsageMeter(_config, _reporter);
        }
    }
}
