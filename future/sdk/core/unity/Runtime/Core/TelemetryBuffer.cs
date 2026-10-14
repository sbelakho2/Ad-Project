using System.Collections.Generic;
using System.Linq;

namespace Apex.Mediation.Core
{
    internal sealed class TelemetryBuffer
    {
        private readonly Queue<TelemetryTrace> _traces = new();
        private readonly object _gate = new();
        private readonly int _capacity;

        public TelemetryBuffer(int capacity = 50)
        {
            _capacity = capacity;
        }

        public void Record(TelemetryTrace trace)
        {
            lock (_gate)
            {
                _traces.Enqueue(trace);
                while (_traces.Count > _capacity)
                {
                    _traces.Dequeue();
                }
            }
        }

        public IReadOnlyList<TelemetryTrace> Snapshot()
        {
            lock (_gate)
            {
                return _traces.ToList();
            }
        }
    }
}
