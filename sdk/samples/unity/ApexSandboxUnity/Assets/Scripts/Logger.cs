// Simple ring-buffer logger for on-screen console
using System.Collections.Generic;
using System.Text;

namespace ApexSandboxUnity
{
    public class Logger
    {
        private readonly int _capacity;
        private readonly Queue<string> _lines;

        public Logger(int capacity = 200)
        {
            _capacity = capacity;
            _lines = new Queue<string>(capacity);
        }

        public void Log(string line)
        {
            if (_lines.Count >= _capacity)
            {
                _lines.Dequeue();
            }
            _lines.Enqueue(line);
        }

        public string Dump()
        {
            var sb = new StringBuilder();
            foreach (var l in _lines)
            {
                sb.AppendLine(l);
            }
            return sb.ToString();
        }
    }
}
