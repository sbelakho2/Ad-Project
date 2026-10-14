using System;
using System.Collections.Concurrent;
using Apex.Mediation.Internal;

namespace Apex.Mediation.Core
{
    internal sealed class EventPump
    {
        private readonly ConcurrentQueue<Action> _queue = new();

        public void Enqueue(Action action)
        {
            if (action == null)
            {
                return;
            }

            _queue.Enqueue(action);
            UnityMainThread.Dispatch(Drain);
        }

        private void Drain()
        {
            while (_queue.TryDequeue(out var action))
            {
                try
                {
                    action();
                }
                catch (Exception ex)
                {
                    Logger.LogError("EventPump exception", ex);
                }
            }
        }
    }
}
