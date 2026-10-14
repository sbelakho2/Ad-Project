#if UNITY_2020_3_OR_NEWER
using System;
using System.Linq;
using UnityEngine;
using Apex.Mediation;

namespace Apex.Mediation.Core
{
    /// <summary>
    /// Heads-up display for adapter traces (toggle with backquote by default).
    /// </summary>
    public sealed class MediationDebuggerOverlay : MonoBehaviour
    {
        [SerializeField] private KeyCode toggleKey = KeyCode.BackQuote;
        private bool _visible;
        private Vector2 _scroll;

        private void Awake()
        {
            DontDestroyOnLoad(gameObject);
        }

        private void Update()
        {
            if (Input.GetKeyDown(toggleKey))
            {
                _visible = !_visible;
            }
        }

        private void OnGUI()
        {
            if (!_visible)
            {
                return;
            }

            var traces = ApexMediation.GetTelemetryTraces().Reverse().ToList();
            var proofs = ApexMediation.GetTransparencyProofs();
            GUILayout.BeginArea(new Rect(20, 20, 420, Screen.height - 40), GUI.skin.box);
            GUILayout.Label($"Mediation Debugger ({traces.Count} events)", GUI.skin.label);
            if (proofs.Count > 0)
            {
                GUILayout.Label($"Latest proof: {proofs[^1].Hash.Substring(0, Math.Min(12, proofs[^1].Hash.Length))}…");
            }
            _scroll = GUILayout.BeginScrollView(_scroll);
            foreach (var trace in traces)
            {
                GUILayout.Label($"[{trace.Timestamp:HH:mm:ss}] {trace.PlacementId} · {trace.Adapter} · {trace.Outcome} · {trace.Latency.TotalMilliseconds:F0} ms");
            }

            GUILayout.EndScrollView();
            GUILayout.EndArea();
        }
    }
}
#endif
