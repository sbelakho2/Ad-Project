using System;
using System.Collections.Generic;
using System.Collections.Concurrent;
using System.Linq;
using System.Threading;
using System.Threading.Tasks;

namespace RivalApex.Mediation
{
    /// <summary>
    /// Ad source with priority and weight configuration
    /// </summary>
    public class WeightedAdSource
    {
        public string Id { get; }
        public int Priority { get; }
        public double Weight { get; }
        public TimeSpan Timeout { get; }
        public bool Enabled { get; }
        public double MinBid { get; }
        public IReadOnlyDictionary<string, string> Metadata { get; }

        public WeightedAdSource(
            string id,
            int priority,
            double weight = 1.0,
            TimeSpan? timeout = null,
            bool enabled = true,
            double minBid = 0.0,
            Dictionary<string, string> metadata = null)
        {
            if (string.IsNullOrEmpty(id))
                throw new ArgumentException("ID is required", nameof(id));
            if (priority < 0)
                throw new ArgumentException("Priority must be non-negative", nameof(priority));
            if (weight <= 0)
                throw new ArgumentException("Weight must be positive", nameof(weight));

            Id = id;
            Priority = priority;
            Weight = weight;
            Timeout = timeout ?? TimeSpan.FromSeconds(5);
            Enabled = enabled;
            MinBid = minBid;
            Metadata = metadata ?? new Dictionary<string, string>();
        }
    }

    /// <summary>
    /// Result of loading an ad from a source
    /// </summary>
    public abstract class AdLoadResult
    {
        public abstract string SourceId { get; }
        public abstract bool IsSuccess { get; }

        public sealed class Success : AdLoadResult
        {
            public object Ad { get; }
            public override string SourceId { get; }
            public long LatencyMs { get; }
            public double Bid { get; }
            public override bool IsSuccess => true;

            public Success(object ad, string sourceId, long latencyMs, double bid = 0.0)
            {
                Ad = ad;
                SourceId = sourceId;
                LatencyMs = latencyMs;
                Bid = bid;
            }
        }

        public sealed class NoFill : AdLoadResult
        {
            public override string SourceId { get; }
            public string Reason { get; }
            public override bool IsSuccess => false;

            public NoFill(string sourceId, string reason)
            {
                SourceId = sourceId;
                Reason = reason;
            }
        }

        public sealed class Error : AdLoadResult
        {
            public override string SourceId { get; }
            public Exception Exception { get; }
            public override bool IsSuccess => false;

            public Error(string sourceId, Exception exception)
            {
                SourceId = sourceId;
                Exception = exception;
            }
        }

        public sealed class Timeout : AdLoadResult
        {
            public override string SourceId { get; }
            public override bool IsSuccess => false;

            public Timeout(string sourceId)
            {
                SourceId = sourceId;
            }
        }
    }

    /// <summary>
    /// Statistics for a source
    /// </summary>
    public class SourcePerformance
    {
        public string SourceId { get; set; }
        public int TotalAttempts { get; set; }
        public int SuccessCount { get; set; }
        public int NoFillCount { get; set; }
        public int ErrorCount { get; set; }
        public int TimeoutCount { get; set; }
        public double AverageLatencyMs { get; set; }
        public double FillRate { get; set; }
        public double AverageBid { get; set; }
    }

    /// <summary>
    /// Configuration for priority-weighted mediation
    /// </summary>
    public class PriorityWeightedConfig
    {
        public bool UsePerformanceWeighting { get; set; } = true;
        public TimeSpan PerformanceWindow { get; set; } = TimeSpan.FromHours(1);
        public int MinSampleSize { get; set; } = 10;
        public int MaxConcurrentRequests { get; set; } = 3;
        public bool AdaptiveTimeoutsEnabled { get; set; } = true;

        public static PriorityWeightedConfig Default => new PriorityWeightedConfig();
    }

    /// <summary>
    /// Ad loader delegate
    /// </summary>
    public delegate Task<AdLoadResult> AdLoader(WeightedAdSource source, CancellationToken token);

    /// <summary>
    /// Priority-weighted mediation manager
    /// </summary>
    public sealed class PriorityWeightedMediation
    {
        private class SourceStats
        {
            public int Attempts;
            public int Successes;
            public int NoFills;
            public int Errors;
            public int Timeouts;
            public long TotalLatencyMs;
            public long TotalBidMicros;
        }

        private readonly PriorityWeightedConfig _config;
        private readonly ConcurrentDictionary<string, SourceStats> _sourceStats;
        private readonly Random _random;

        public PriorityWeightedMediation(PriorityWeightedConfig config = null)
        {
            _config = config ?? PriorityWeightedConfig.Default;
            _sourceStats = new ConcurrentDictionary<string, SourceStats>();
            _random = new Random();
        }

        /// <summary>
        /// Execute mediation with priority-weighted selection
        /// </summary>
        public async Task<AdLoadResult> ExecuteAsync(
            IEnumerable<WeightedAdSource> sources,
            AdLoader loader,
            CancellationToken cancellationToken = default)
        {
            var sourceList = sources?.ToList() ?? new List<WeightedAdSource>();

            if (!sourceList.Any())
            {
                return new AdLoadResult.NoFill("none", "No sources configured");
            }

            var enabledSources = sourceList.Where(s => s.Enabled).ToList();
            if (!enabledSources.Any())
            {
                return new AdLoadResult.NoFill("none", "All sources disabled");
            }

            var priorityGroups = enabledSources
                .GroupBy(s => s.Priority)
                .OrderBy(g => g.Key);

            foreach (var group in priorityGroups)
            {
                var result = await ExecutePriorityGroupAsync(
                    group.ToList(), 
                    loader, 
                    cancellationToken);

                if (result.IsSuccess)
                {
                    return result;
                }
            }

            return new AdLoadResult.NoFill("all", "All sources exhausted");
        }

        /// <summary>
        /// Get performance statistics for all sources
        /// </summary>
        public List<SourcePerformance> GetPerformanceStats()
        {
            return _sourceStats.Select(kvp =>
            {
                var stats = kvp.Value;
                var attempts = stats.Attempts;
                var successes = stats.Successes;

                return new SourcePerformance
                {
                    SourceId = kvp.Key,
                    TotalAttempts = attempts,
                    SuccessCount = successes,
                    NoFillCount = stats.NoFills,
                    ErrorCount = stats.Errors,
                    TimeoutCount = stats.Timeouts,
                    AverageLatencyMs = successes > 0 ? (double)stats.TotalLatencyMs / successes : 0,
                    FillRate = attempts > 0 ? (double)successes / attempts : 0,
                    AverageBid = successes > 0 ? (double)stats.TotalBidMicros / (successes * 1_000_000) : 0
                };
            }).ToList();
        }

        /// <summary>
        /// Reset all statistics
        /// </summary>
        public void ResetAllStats()
        {
            _sourceStats.Clear();
        }

        private async Task<AdLoadResult> ExecutePriorityGroupAsync(
            List<WeightedAdSource> sources,
            AdLoader loader,
            CancellationToken cancellationToken)
        {
            var remainingSources = new List<WeightedAdSource>(sources);

            while (remainingSources.Any())
            {
                cancellationToken.ThrowIfCancellationRequested();

                var selected = SelectByWeight(remainingSources);
                remainingSources.Remove(selected);

                var result = await ExecuteWithTimeoutAsync(selected, loader, cancellationToken);
                RecordResult(selected.Id, result);

                if (result.IsSuccess)
                {
                    return result;
                }
            }

            return new AdLoadResult.NoFill("priority_group", "No fill from priority group");
        }

        private WeightedAdSource SelectByWeight(List<WeightedAdSource> sources)
        {
            if (sources.Count == 1)
            {
                return sources[0];
            }

            var effectiveWeights = sources
                .Select(s => (Source: s, Weight: CalculateEffectiveWeight(s)))
                .ToList();

            var totalWeight = effectiveWeights.Sum(x => x.Weight);

            if (totalWeight <= 0)
            {
                return sources[_random.Next(sources.Count)];
            }

            var random = _random.NextDouble() * totalWeight;

            foreach (var (source, weight) in effectiveWeights)
            {
                random -= weight;
                if (random <= 0)
                {
                    return source;
                }
            }

            return sources.Last();
        }

        private double CalculateEffectiveWeight(WeightedAdSource source)
        {
            if (!_config.UsePerformanceWeighting)
            {
                return source.Weight;
            }

            if (!_sourceStats.TryGetValue(source.Id, out var stats))
            {
                return source.Weight;
            }

            if (stats.Attempts < _config.MinSampleSize)
            {
                return source.Weight;
            }

            var fillRate = (double)stats.Successes / Math.Max(1, stats.Attempts);
            var performanceMultiplier = 0.5 + fillRate;

            return Math.Max(0.1, source.Weight * performanceMultiplier);
        }

        private async Task<AdLoadResult> ExecuteWithTimeoutAsync(
            WeightedAdSource source,
            AdLoader loader,
            CancellationToken cancellationToken)
        {
            var timeout = _config.AdaptiveTimeoutsEnabled
                ? CalculateAdaptiveTimeout(source)
                : source.Timeout;

            using var cts = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
            cts.CancelAfter(timeout);

            try
            {
                return await loader(source, cts.Token);
            }
            catch (OperationCanceledException) when (!cancellationToken.IsCancellationRequested)
            {
                return new AdLoadResult.Timeout(source.Id);
            }
            catch (Exception ex)
            {
                return new AdLoadResult.Error(source.Id, ex);
            }
        }

        private TimeSpan CalculateAdaptiveTimeout(WeightedAdSource source)
        {
            if (!_sourceStats.TryGetValue(source.Id, out var stats))
            {
                return source.Timeout;
            }

            if (stats.Successes < _config.MinSampleSize)
            {
                return source.Timeout;
            }

            var avgLatencyMs = (double)stats.TotalLatencyMs / stats.Successes;
            var adaptiveTimeout = TimeSpan.FromMilliseconds(avgLatencyMs * 2);

            return adaptiveTimeout < source.Timeout
                ? (adaptiveTimeout > TimeSpan.FromSeconds(1) ? adaptiveTimeout : TimeSpan.FromSeconds(1))
                : source.Timeout;
        }

        private void RecordResult(string sourceId, AdLoadResult result)
        {
            var stats = _sourceStats.GetOrAdd(sourceId, _ => new SourceStats());

            Interlocked.Increment(ref stats.Attempts);

            switch (result)
            {
                case AdLoadResult.Success success:
                    Interlocked.Increment(ref stats.Successes);
                    Interlocked.Add(ref stats.TotalLatencyMs, success.LatencyMs);
                    Interlocked.Add(ref stats.TotalBidMicros, (long)(success.Bid * 1_000_000));
                    break;
                case AdLoadResult.NoFill:
                    Interlocked.Increment(ref stats.NoFills);
                    break;
                case AdLoadResult.Error:
                    Interlocked.Increment(ref stats.Errors);
                    break;
                case AdLoadResult.Timeout:
                    Interlocked.Increment(ref stats.Timeouts);
                    break;
            }
        }
    }

    /// <summary>
    /// Builder for WeightedAdSource
    /// </summary>
    public class WeightedAdSourceBuilder
    {
        private string _id = "";
        private int _priority = 0;
        private double _weight = 1.0;
        private TimeSpan _timeout = TimeSpan.FromSeconds(5);
        private bool _enabled = true;
        private double _minBid = 0.0;
        private Dictionary<string, string> _metadata = new Dictionary<string, string>();

        public WeightedAdSourceBuilder Id(string id)
        {
            _id = id;
            return this;
        }

        public WeightedAdSourceBuilder Priority(int priority)
        {
            _priority = priority;
            return this;
        }

        public WeightedAdSourceBuilder Weight(double weight)
        {
            _weight = weight;
            return this;
        }

        public WeightedAdSourceBuilder Timeout(TimeSpan timeout)
        {
            _timeout = timeout;
            return this;
        }

        public WeightedAdSourceBuilder Enabled(bool enabled)
        {
            _enabled = enabled;
            return this;
        }

        public WeightedAdSourceBuilder MinBid(double minBid)
        {
            _minBid = minBid;
            return this;
        }

        public WeightedAdSourceBuilder Metadata(string key, string value)
        {
            _metadata[key] = value;
            return this;
        }

        public WeightedAdSource Build()
        {
            return new WeightedAdSource(
                _id,
                _priority,
                _weight,
                _timeout,
                _enabled,
                _minBid,
                _metadata);
        }
    }
}
