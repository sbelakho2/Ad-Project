using System;
using System.Collections.Generic;
using UnityEngine;
using UnityEngine.Networking;

namespace RivalApex.SDK.Core
{
    /// <summary>
    /// NetworkMonitor - Monitors network connectivity and provides fast-fail behavior.
    ///
    /// This class provides:
    /// - Real-time network state monitoring using Unity's Network APIs
    /// - Fast-fail for no-network conditions (no UI jank)
    /// - Connection quality assessment
    /// - Callback-based network state change notifications
    /// </summary>
    public sealed class NetworkMonitor : MonoBehaviour
    {
        private static NetworkMonitor _instance;
        private static readonly object _lock = new object();
        
        /// <summary>
        /// Singleton instance of NetworkMonitor
        /// </summary>
        public static NetworkMonitor Instance
        {
            get
            {
                if (_instance == null)
                {
                    lock (_lock)
                    {
                        if (_instance == null)
                        {
                            var go = new GameObject("RivalApexNetworkMonitor");
                            _instance = go.AddComponent<NetworkMonitor>();
                            DontDestroyOnLoad(go);
                        }
                    }
                }
                return _instance;
            }
        }
        
        /// <summary>
        /// Fast-fail timeout when offline (seconds)
        /// </summary>
        public const float OfflineFastFailTimeout = 0.1f;
        
        /// <summary>
        /// Normal network timeout (seconds)
        /// </summary>
        public const float NormalTimeout = 10f;
        
        /// <summary>
        /// Interval between connectivity checks (seconds)
        /// </summary>
        public float CheckInterval = 3f;
        
        /// <summary>
        /// Connection type enumeration
        /// </summary>
        public enum ConnectionType
        {
            None,
            Wifi,
            Cellular,
            Ethernet,
            Other
        }
        
        /// <summary>
        /// Network state information
        /// </summary>
        public readonly struct NetworkState : IEquatable<NetworkState>
        {
            public readonly bool IsConnected;
            public readonly ConnectionType ConnectionType;
            public readonly bool IsExpensive;
            public readonly DateTime Timestamp;
            
            public NetworkState(bool isConnected, ConnectionType connectionType, bool isExpensive)
            {
                IsConnected = isConnected;
                ConnectionType = connectionType;
                IsExpensive = isExpensive;
                Timestamp = DateTime.UtcNow;
            }
            
            public static NetworkState Offline => new NetworkState(false, ConnectionType.None, false);
            
            public bool Equals(NetworkState other)
            {
                return IsConnected == other.IsConnected && 
                       ConnectionType == other.ConnectionType && 
                       IsExpensive == other.IsExpensive;
            }
            
            public override bool Equals(object obj) => obj is NetworkState other && Equals(other);
            
            public override int GetHashCode() => HashCode.Combine(IsConnected, ConnectionType, IsExpensive);
            
            public static bool operator ==(NetworkState left, NetworkState right) => left.Equals(right);
            public static bool operator !=(NetworkState left, NetworkState right) => !left.Equals(right);
        }
        
        /// <summary>
        /// Result of a pre-flight check before network operations
        /// </summary>
        public readonly struct PreflightResult
        {
            public readonly bool ShouldProceed;
            public readonly string FailReason;
            public readonly NetworkState State;
            
            private PreflightResult(bool shouldProceed, string failReason, NetworkState state)
            {
                ShouldProceed = shouldProceed;
                FailReason = failReason;
                State = state;
            }
            
            public static PreflightResult Proceed(NetworkState state) => 
                new PreflightResult(true, null, state);
            
            public static PreflightResult FastFail(string reason, NetworkState state) => 
                new PreflightResult(false, reason, state);
        }
        
        /// <summary>
        /// Delegate for network state change events
        /// </summary>
        public delegate void NetworkStateChangedHandler(NetworkState state);
        
        /// <summary>
        /// Event fired when network state changes
        /// </summary>
        public event NetworkStateChangedHandler OnNetworkStateChanged;
        
        private NetworkState _currentState = NetworkState.Offline;
        private readonly Dictionary<string, NetworkStateChangedHandler> _listeners = new();
        private float _lastCheckTime;
        private bool _isMonitoring;
        
        /// <summary>
        /// Gets the current network state
        /// </summary>
        public NetworkState State => _currentState;
        
        /// <summary>
        /// Checks if network is currently connected
        /// </summary>
        public bool IsConnected => _currentState.IsConnected;
        
        /// <summary>
        /// Checks if on an expensive connection
        /// </summary>
        public bool IsExpensive => _currentState.IsExpensive;
        
        private void Awake()
        {
            if (_instance != null && _instance != this)
            {
                Destroy(gameObject);
                return;
            }
            
            _instance = this;
            DontDestroyOnLoad(gameObject);
            UpdateNetworkState();
        }
        
        /// <summary>
        /// Starts monitoring network state changes
        /// </summary>
        public void StartMonitoring()
        {
            _isMonitoring = true;
            UpdateNetworkState();
        }
        
        /// <summary>
        /// Stops monitoring network state changes
        /// </summary>
        public void StopMonitoring()
        {
            _isMonitoring = false;
        }
        
        private void Update()
        {
            if (!_isMonitoring) return;
            
            if (Time.unscaledTime - _lastCheckTime >= CheckInterval)
            {
                _lastCheckTime = Time.unscaledTime;
                UpdateNetworkState();
            }
        }
        
        /// <summary>
        /// Performs a preflight check before network operations.
        /// Returns immediately if offline, allowing fast-fail behavior.
        /// </summary>
        public PreflightResult Preflight()
        {
            // Force update before preflight
            UpdateNetworkState();
            
            if (_currentState.IsConnected)
            {
                return PreflightResult.Proceed(_currentState);
            }
            
            return PreflightResult.FastFail("No network connection", _currentState);
        }
        
        /// <summary>
        /// Gets the appropriate timeout based on network state.
        /// Returns fast-fail timeout when offline.
        /// </summary>
        public float EffectiveTimeout()
        {
            return IsConnected ? NormalTimeout : OfflineFastFailTimeout;
        }
        
        /// <summary>
        /// Adds a listener for network state changes.
        /// </summary>
        /// <returns>A token to use when removing the listener</returns>
        public string AddListener(NetworkStateChangedHandler listener)
        {
            var token = Guid.NewGuid().ToString();
            _listeners[token] = listener;
            
            // Immediately notify with current state
            listener?.Invoke(_currentState);
            return token;
        }
        
        /// <summary>
        /// Removes a listener using its token.
        /// </summary>
        public void RemoveListener(string token)
        {
            _listeners.Remove(token);
        }
        
        /// <summary>
        /// Gets network quality hints for adaptive behavior.
        /// </summary>
        public Dictionary<string, object> QualityHints()
        {
            return new Dictionary<string, object>
            {
                ["isConnected"] = _currentState.IsConnected,
                ["connectionType"] = _currentState.ConnectionType.ToString(),
                ["isExpensive"] = _currentState.IsExpensive,
                ["suggestedTimeout"] = EffectiveTimeout(),
                ["shouldReduceQuality"] = _currentState.IsExpensive
            };
        }
        
        /// <summary>
        /// Forces an immediate network state update
        /// </summary>
        public void ForceUpdate()
        {
            UpdateNetworkState();
        }
        
        private void UpdateNetworkState()
        {
            var reachability = Application.internetReachability;
            
            ConnectionType connectionType;
            bool isConnected;
            bool isExpensive;
            
            switch (reachability)
            {
                case NetworkReachability.ReachableViaLocalAreaNetwork:
                    connectionType = ConnectionType.Wifi;
                    isConnected = true;
                    isExpensive = false;
                    break;
                    
                case NetworkReachability.ReachableViaCarrierDataNetwork:
                    connectionType = ConnectionType.Cellular;
                    isConnected = true;
                    isExpensive = true;
                    break;
                    
                case NetworkReachability.NotReachable:
                default:
                    connectionType = ConnectionType.None;
                    isConnected = false;
                    isExpensive = false;
                    break;
            }
            
            var newState = new NetworkState(isConnected, connectionType, isExpensive);
            
            if (newState != _currentState)
            {
                _currentState = newState;
                NotifyListeners();
            }
        }
        
        private void NotifyListeners()
        {
            OnNetworkStateChanged?.Invoke(_currentState);
            
            foreach (var listener in _listeners.Values)
            {
                listener?.Invoke(_currentState);
            }
        }
        
        private void OnDestroy()
        {
            if (_instance == this)
            {
                _instance = null;
            }
        }
    }
    
    /// <summary>
    /// Exception for fast-fail on no-network conditions
    /// </summary>
    public class NoNetworkException : Exception
    {
        public NetworkMonitor.NetworkState NetworkState { get; }
        
        public NoNetworkException(string message, NetworkMonitor.NetworkState networkState)
            : base(message)
        {
            NetworkState = networkState;
        }
    }
}
