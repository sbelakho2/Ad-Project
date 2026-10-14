using System;
using System.Diagnostics;

namespace Apex.Mediation.Core
{
    internal interface IClock
    {
        TimeSpan Now { get; }
    }

    // Monotonic clock based on Stopwatch to avoid wall-clock drift
    internal sealed class MonotonicClock : IClock
    {
        private static readonly double TickDurationSeconds = 1.0 / Stopwatch.Frequency;
        private readonly long _startTicks;

        private MonotonicClock()
        {
            _startTicks = Stopwatch.GetTimestamp();
        }

        public static MonotonicClock Instance { get; } = new MonotonicClock();

        public TimeSpan Now
        {
            get
            {
                var deltaTicks = Stopwatch.GetTimestamp() - _startTicks;
                var seconds = deltaTicks * TickDurationSeconds;
                return TimeSpan.FromSeconds(seconds);
            }
        }
    }
}
