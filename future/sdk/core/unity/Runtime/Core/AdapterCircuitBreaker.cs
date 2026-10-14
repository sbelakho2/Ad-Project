using System;
using System.Collections.Generic;

namespace Apex.Mediation.Core
{
    internal sealed class AdapterCircuitBreaker
    {
        private enum State
        {
            Closed,
            Open,
            HalfOpen
        }

        private readonly int _failureThreshold;
        private readonly long _timeWindowMs;
        private readonly long _recoveryTimeMs;
        private readonly IClock _clock;
        private readonly List<long> _failures = new();
        private State _state = State.Closed;
        private long _openedAtMs;
        private readonly object _gate = new();

        public AdapterCircuitBreaker(int failureThreshold = 3, long timeWindowMs = 30_000, long recoveryTimeMs = 15_000, IClock? clock = null)
        {
            _failureThreshold = failureThreshold;
            _timeWindowMs = timeWindowMs;
            _recoveryTimeMs = recoveryTimeMs;
            _clock = clock ?? MonotonicClock.Instance;
        }

        public bool IsOpen()
        {
            lock (_gate)
            {
                var now = NowMs();
                if (_state == State.Open && now - _openedAtMs >= _recoveryTimeMs)
                {
                    _state = State.HalfOpen;
                }

                return _state == State.Open;
            }
        }

        public void RecordSuccess()
        {
            lock (_gate)
            {
                if (_state == State.HalfOpen)
                {
                    _state = State.Closed;
                    _failures.Clear();
                    return;
                }

                if (_state == State.Closed)
                {
                    _failures.Clear();
                }
            }
        }

        public void RecordFailure()
        {
            lock (_gate)
            {
                var now = NowMs();
                _failures.Add(now);
                _failures.RemoveAll(ts => now - ts > _timeWindowMs);

                if (_state == State.HalfOpen)
                {
                    _state = State.Open;
                    _openedAtMs = now;
                    return;
                }

                if (_state == State.Closed && _failures.Count >= _failureThreshold)
                {
                    _state = State.Open;
                    _openedAtMs = now;
                }
            }
        }

        public long RecoveryTimeMs => _recoveryTimeMs;

        private long NowMs() => (long)_clock.Now.TotalMilliseconds;
    }
}
