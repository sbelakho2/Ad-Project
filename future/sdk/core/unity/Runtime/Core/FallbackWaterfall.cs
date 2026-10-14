using System;
using System.Collections.Generic;
using System.Threading;
using System.Threading.Tasks;

namespace RivalApex.SDK.Core
{
    /// <summary>
    /// FallbackWaterfall - Implements a waterfall mediation strategy for ad loading.
    ///
    /// When one ad source fails, it automatically falls back to the next source
    /// in the waterfall. This ensures maximum fill rate by trying multiple
    /// demand partners in priority order.
    /// </summary>
    public class FallbackWaterfall<T>
    {
        /// <summary>
        /// Default timeout for source loading (ms)
        /// </summary>
        public int DefaultTimeoutMs { get; }
        
        public FallbackWaterfall(int defaultTimeoutMs = 5000)
        {
            DefaultTimeoutMs = defaultTimeoutMs;
        }
        
        /// <summary>
        /// Represents an ad source in the waterfall
        /// </summary>
        public class WaterfallSource
        {
            public string Id { get; }
            public int Priority { get; }
            public int TimeoutMs { get; }
            public Func<CancellationToken, Task<WaterfallResult<T>>> Loader { get; }
            
            public WaterfallSource(
                string id,
                int priority,
                int timeoutMs,
                Func<CancellationToken, Task<WaterfallResult<T>>> loader)
            {
                Id = id;
                Priority = priority;
                TimeoutMs = timeoutMs;
                Loader = loader;
            }
        }
        
        /// <summary>
        /// Result of attempting to load from a source
        /// </summary>
        public abstract class WaterfallResult<TResult>
        {
            public class Success : WaterfallResult<TResult>
            {
                public TResult Data { get; }
                public Success(TResult data) => Data = data;
            }
            
            public class NoFill : WaterfallResult<TResult>
            {
                public string Reason { get; }
                public NoFill(string reason = "No ad available") => Reason = reason;
            }
            
            public class Error : WaterfallResult<TResult>
            {
                public Exception Exception { get; }
                public Error(Exception exception) => Exception = exception;
            }
            
            public class Timeout : WaterfallResult<TResult> { }
        }
        
        public enum AttemptResultType
        {
            Success, NoFill, Error, Timeout, Skipped
        }
        
        /// <summary>
        /// Details about each attempt in the waterfall
        /// </summary>
        public readonly struct AttemptDetail
        {
            public string SourceId { get; }
            public int Priority { get; }
            public long DurationMs { get; }
            public AttemptResultType Result { get; }
            
            public AttemptDetail(string sourceId, int priority, long durationMs, AttemptResultType result)
            {
                SourceId = sourceId;
                Priority = priority;
                DurationMs = durationMs;
                Result = result;
            }
        }
        
        /// <summary>
        /// Waterfall execution result with metadata
        /// </summary>
        public class ExecutionResult
        {
            public WaterfallResult<T> Result { get; }
            public string SourceId { get; }
            public int AttemptsCount { get; }
            public long TotalDurationMs { get; }
            public IReadOnlyList<AttemptDetail> AttemptDetails { get; }
            
            public ExecutionResult(
                WaterfallResult<T> result,
                string sourceId,
                int attemptsCount,
                long totalDurationMs,
                IReadOnlyList<AttemptDetail> attemptDetails)
            {
                Result = result;
                SourceId = sourceId;
                AttemptsCount = attemptsCount;
                TotalDurationMs = totalDurationMs;
                AttemptDetails = attemptDetails;
            }
        }
        
        /// <summary>
        /// Performance statistics for a source
        /// </summary>
        public class SourceStats
        {
            private int _successCount;
            private int _failureCount;
            private int _timeoutCount;
            private int _noFillCount;
            private long _totalLatencyMs;
            
            public int SuccessCount => _successCount;
            public int FailureCount => _failureCount;
            public int TimeoutCount => _timeoutCount;
            public int NoFillCount => _noFillCount;
            
            public int TotalAttempts => _successCount + _failureCount + _timeoutCount + _noFillCount;
            
            public float SuccessRate => TotalAttempts > 0 ? (float)_successCount / TotalAttempts : 0f;
            
            public long AverageLatencyMs => TotalAttempts > 0 ? _totalLatencyMs / TotalAttempts : 0;
            
            public void RecordSuccess(long latencyMs)
            {
                Interlocked.Increment(ref _successCount);
                Interlocked.Add(ref _totalLatencyMs, latencyMs);
            }
            
            public void RecordFailure(long latencyMs)
            {
                Interlocked.Increment(ref _failureCount);
                Interlocked.Add(ref _totalLatencyMs, latencyMs);
            }
            
            public void RecordTimeout(long latencyMs)
            {
                Interlocked.Increment(ref _timeoutCount);
                Interlocked.Add(ref _totalLatencyMs, latencyMs);
            }
            
            public void RecordNoFill(long latencyMs)
            {
                Interlocked.Increment(ref _noFillCount);
                Interlocked.Add(ref _totalLatencyMs, latencyMs);
            }
        }
        
        private readonly Dictionary<string, SourceStats> _sourceStats = new();
        private readonly object _statsLock = new();
        
        /// <summary>
        /// Executes the waterfall, trying each source in priority order
        /// </summary>
        public async Task<ExecutionResult> ExecuteAsync(
            IEnumerable<WaterfallSource> sources,
            CancellationToken cancellationToken = default)
        {
            var startTime = DateTime.UtcNow;
            var attemptDetails = new List<AttemptDetail>();
            var sortedSources = new List<WaterfallSource>(sources);
            sortedSources.Sort((a, b) => a.Priority.CompareTo(b.Priority));
            
            foreach (var source in sortedSources)
            {
                cancellationToken.ThrowIfCancellationRequested();
                
                var attemptStart = DateTime.UtcNow;
                WaterfallResult<T> result;
                
                try
                {
                    using var timeoutCts = new CancellationTokenSource(source.TimeoutMs);
                    using var linkedCts = CancellationTokenSource.CreateLinkedTokenSource(
                        cancellationToken, timeoutCts.Token);
                    
                    result = await source.Loader(linkedCts.Token);
                }
                catch (OperationCanceledException) when (!cancellationToken.IsCancellationRequested)
                {
                    result = new WaterfallResult<T>.Timeout();
                }
                catch (Exception ex)
                {
                    result = new WaterfallResult<T>.Error(ex);
                }
                
                var durationMs = (long)(DateTime.UtcNow - attemptStart).TotalMilliseconds;
                RecordStats(source.Id, result, durationMs);
                
                var attemptResultType = result switch
                {
                    WaterfallResult<T>.Success => AttemptResultType.Success,
                    WaterfallResult<T>.NoFill => AttemptResultType.NoFill,
                    WaterfallResult<T>.Error => AttemptResultType.Error,
                    WaterfallResult<T>.Timeout => AttemptResultType.Timeout,
                    _ => AttemptResultType.Error
                };
                
                attemptDetails.Add(new AttemptDetail(
                    source.Id, source.Priority, durationMs, attemptResultType));
                
                if (result is WaterfallResult<T>.Success)
                {
                    return new ExecutionResult(
                        result,
                        source.Id,
                        attemptDetails.Count,
                        (long)(DateTime.UtcNow - startTime).TotalMilliseconds,
                        attemptDetails);
                }
            }
            
            // All sources exhausted
            return new ExecutionResult(
                new WaterfallResult<T>.NoFill($"All {sortedSources.Count} sources exhausted"),
                sortedSources.Count > 0 ? sortedSources[^1].Id : "none",
                attemptDetails.Count,
                (long)(DateTime.UtcNow - startTime).TotalMilliseconds,
                attemptDetails);
        }
        
        public SourceStats GetStats(string sourceId)
        {
            lock (_statsLock)
            {
                return _sourceStats.TryGetValue(sourceId, out var stats) ? stats : null;
            }
        }
        
        public Dictionary<string, SourceStats> GetAllStats()
        {
            lock (_statsLock)
            {
                return new Dictionary<string, SourceStats>(_sourceStats);
            }
        }
        
        public void ClearStats()
        {
            lock (_statsLock)
            {
                _sourceStats.Clear();
            }
        }
        
        // Overload for the common case (no explicit timeout): keeps the natural 3-arg shape.
        public WaterfallSource CreateSource(
            string id,
            int priority,
            Func<CancellationToken, Task<WaterfallResult<T>>> loader)
        {
            return CreateSource(id, priority, null, loader);
        }

        // Original 4-arg shape preserved: optional parameter now precedes the required loader,
        // which is only legal as a complete method (the previous single signature was CS1737).
        public WaterfallSource CreateSource(
            string id,
            int priority,
            int? timeoutMs,
            Func<CancellationToken, Task<WaterfallResult<T>>> loader)
        {
            return new WaterfallSource(id, priority, timeoutMs ?? DefaultTimeoutMs, loader);
        }
        
        private void RecordStats(string sourceId, WaterfallResult<T> result, long durationMs)
        {
            SourceStats stats;
            lock (_statsLock)
            {
                if (!_sourceStats.TryGetValue(sourceId, out stats))
                {
                    stats = new SourceStats();
                    _sourceStats[sourceId] = stats;
                }
            }
            
            switch (result)
            {
                case WaterfallResult<T>.Success:
                    stats.RecordSuccess(durationMs);
                    break;
                case WaterfallResult<T>.NoFill:
                    stats.RecordNoFill(durationMs);
                    break;
                case WaterfallResult<T>.Error:
                    stats.RecordFailure(durationMs);
                    break;
                case WaterfallResult<T>.Timeout:
                    stats.RecordTimeout(durationMs);
                    break;
            }
        }
    }
}
