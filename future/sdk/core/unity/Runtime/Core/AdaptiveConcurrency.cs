using System;
using System.Collections.Concurrent;
using System.Collections.Generic;
using System.Threading;
using System.Threading.Tasks;
using UnityEngine;

namespace RivalApexMediation.Core.Threading
{
    /// <summary>
    /// AdaptiveConcurrency - Adaptive task scheduler sizing based on device capabilities.
    /// 
    /// This class provides adaptive concurrency management for Unity that:
    /// - Detects CPU core count and platform capabilities
    /// - Sizes task limits appropriately for the device
    /// - Provides separate scheduling for CPU-bound and I/O-bound operations
    /// - Integrates with Unity's main thread for UI updates
    /// </summary>
    public sealed class AdaptiveConcurrency : IDisposable
    {
        private static AdaptiveConcurrency _instance;
        private static readonly object _lock = new object();

        /// <summary>
        /// Gets the singleton instance of AdaptiveConcurrency.
        /// </summary>
        public static AdaptiveConcurrency Instance
        {
            get
            {
                if (_instance == null)
                {
                    lock (_lock)
                    {
                        if (_instance == null)
                        {
                            _instance = new AdaptiveConcurrency();
                        }
                    }
                }
                return _instance;
            }
        }

        // Configuration constants
        private const int MinConcurrentTasks = 2;
        private const int MaxBackgroundTasks = 8;
        private const int MaxNetworkTasks = 16;
        private const int MaxComputeTasks = 4;

        /// <summary>
        /// Detected CPU information for the current device.
        /// </summary>
        public struct CpuInfo
        {
            public int TotalCores;
            public string Platform;
            public int SystemMemoryMB;
            public bool IsLowEndDevice;
            public GraphicsDeviceType GraphicsType;
        }

        /// <summary>
        /// Pool configuration for different workload types.
        /// </summary>
        public struct PoolConfig
        {
            public int MaxConcurrency;
            public ThreadPriority Priority;
        }

        /// <summary>
        /// Graphics device type enumeration.
        /// </summary>
        public enum GraphicsDeviceType
        {
            Unknown,
            OpenGL,
            Metal,
            Vulkan,
            DirectX
        }

        // Properties
        public CpuInfo DeviceInfo { get; private set; }

        // Semaphores for limiting concurrent operations
        private readonly SemaphoreSlim _backgroundSemaphore;
        private readonly SemaphoreSlim _networkSemaphore;
        private readonly SemaphoreSlim _computeSemaphore;

        // Metrics
        private int _taskCount;
        private int _completedTaskCount;
        private int _rejectedTaskCount;
        
        // Main thread synchronization context
        private readonly SynchronizationContext _mainThreadContext;
        
        private bool _disposed;

        private AdaptiveConcurrency()
        {
            DeviceInfo = DetectCpuInfo();
            
            // Create semaphores based on device capabilities
            var backgroundConfig = GetBackgroundPoolConfig();
            var networkConfig = GetNetworkPoolConfig();
            var computeConfig = GetComputePoolConfig();
            
            _backgroundSemaphore = new SemaphoreSlim(backgroundConfig.MaxConcurrency, backgroundConfig.MaxConcurrency);
            _networkSemaphore = new SemaphoreSlim(networkConfig.MaxConcurrency, networkConfig.MaxConcurrency);
            _computeSemaphore = new SemaphoreSlim(computeConfig.MaxConcurrency, computeConfig.MaxConcurrency);
            
            // Capture main thread context for callbacks
            _mainThreadContext = SynchronizationContext.Current;
            
            Debug.Log($"[AdaptiveConcurrency] Initialized: {DeviceInfo.TotalCores} cores, Platform: {DeviceInfo.Platform}, LowEnd: {DeviceInfo.IsLowEndDevice}");
        }

        /// <summary>
        /// Gets the recommended pool configuration for background work.
        /// </summary>
        public PoolConfig GetBackgroundPoolConfig()
        {
            int maxConcurrency = DeviceInfo.IsLowEndDevice 
                ? Math.Min(2, DeviceInfo.TotalCores)
                : Math.Min(DeviceInfo.TotalCores - 1, MaxBackgroundTasks);
            
            return new PoolConfig
            {
                MaxConcurrency = Math.Max(MinConcurrentTasks, maxConcurrency),
                Priority = ThreadPriority.BelowNormal
            };
        }

        /// <summary>
        /// Gets the recommended pool configuration for network I/O.
        /// </summary>
        public PoolConfig GetNetworkPoolConfig()
        {
            int maxConcurrency = DeviceInfo.IsLowEndDevice
                ? Math.Min(4, DeviceInfo.TotalCores * 2)
                : Math.Min(DeviceInfo.TotalCores * 4, MaxNetworkTasks);
            
            return new PoolConfig
            {
                MaxConcurrency = Math.Max(MinConcurrentTasks, maxConcurrency),
                Priority = ThreadPriority.Normal
            };
        }

        /// <summary>
        /// Gets the recommended pool configuration for compute-intensive work.
        /// </summary>
        public PoolConfig GetComputePoolConfig()
        {
            int maxConcurrency = DeviceInfo.IsLowEndDevice
                ? 1
                : Math.Min(DeviceInfo.TotalCores / 2, MaxComputeTasks);
            
            return new PoolConfig
            {
                MaxConcurrency = Math.Max(1, maxConcurrency),
                Priority = ThreadPriority.AboveNormal
            };
        }

        /// <summary>
        /// Executes a background task with concurrency limiting.
        /// </summary>
        public async Task ExecuteBackgroundAsync(Func<Task> task, CancellationToken cancellationToken = default)
        {
            Interlocked.Increment(ref _taskCount);
            
            try
            {
                await _backgroundSemaphore.WaitAsync(cancellationToken);
                try
                {
                    await task();
                }
                finally
                {
                    _backgroundSemaphore.Release();
                    Interlocked.Increment(ref _completedTaskCount);
                }
            }
            catch (OperationCanceledException)
            {
                Interlocked.Increment(ref _rejectedTaskCount);
                throw;
            }
        }

        /// <summary>
        /// Executes a background task with concurrency limiting and returns a result.
        /// </summary>
        public async Task<T> ExecuteBackgroundAsync<T>(Func<Task<T>> task, CancellationToken cancellationToken = default)
        {
            Interlocked.Increment(ref _taskCount);
            
            try
            {
                await _backgroundSemaphore.WaitAsync(cancellationToken);
                try
                {
                    return await task();
                }
                finally
                {
                    _backgroundSemaphore.Release();
                    Interlocked.Increment(ref _completedTaskCount);
                }
            }
            catch (OperationCanceledException)
            {
                Interlocked.Increment(ref _rejectedTaskCount);
                throw;
            }
        }

        /// <summary>
        /// Executes a network I/O task with concurrency limiting.
        /// </summary>
        public async Task ExecuteNetworkAsync(Func<Task> task, CancellationToken cancellationToken = default)
        {
            Interlocked.Increment(ref _taskCount);
            
            try
            {
                await _networkSemaphore.WaitAsync(cancellationToken);
                try
                {
                    await task();
                }
                finally
                {
                    _networkSemaphore.Release();
                    Interlocked.Increment(ref _completedTaskCount);
                }
            }
            catch (OperationCanceledException)
            {
                Interlocked.Increment(ref _rejectedTaskCount);
                throw;
            }
        }

        /// <summary>
        /// Executes a network I/O task with concurrency limiting and returns a result.
        /// </summary>
        public async Task<T> ExecuteNetworkAsync<T>(Func<Task<T>> task, CancellationToken cancellationToken = default)
        {
            Interlocked.Increment(ref _taskCount);
            
            try
            {
                await _networkSemaphore.WaitAsync(cancellationToken);
                try
                {
                    return await task();
                }
                finally
                {
                    _networkSemaphore.Release();
                    Interlocked.Increment(ref _completedTaskCount);
                }
            }
            catch (OperationCanceledException)
            {
                Interlocked.Increment(ref _rejectedTaskCount);
                throw;
            }
        }

        /// <summary>
        /// Executes a compute-intensive task with concurrency limiting.
        /// </summary>
        public async Task ExecuteComputeAsync(Func<Task> task, CancellationToken cancellationToken = default)
        {
            Interlocked.Increment(ref _taskCount);
            
            try
            {
                await _computeSemaphore.WaitAsync(cancellationToken);
                try
                {
                    await task();
                }
                finally
                {
                    _computeSemaphore.Release();
                    Interlocked.Increment(ref _completedTaskCount);
                }
            }
            catch (OperationCanceledException)
            {
                Interlocked.Increment(ref _rejectedTaskCount);
                throw;
            }
        }

        /// <summary>
        /// Executes a compute-intensive task with concurrency limiting and returns a result.
        /// </summary>
        public async Task<T> ExecuteComputeAsync<T>(Func<Task<T>> task, CancellationToken cancellationToken = default)
        {
            Interlocked.Increment(ref _taskCount);
            
            try
            {
                await _computeSemaphore.WaitAsync(cancellationToken);
                try
                {
                    return await task();
                }
                finally
                {
                    _computeSemaphore.Release();
                    Interlocked.Increment(ref _completedTaskCount);
                }
            }
            catch (OperationCanceledException)
            {
                Interlocked.Increment(ref _rejectedTaskCount);
                throw;
            }
        }

        /// <summary>
        /// Executes a synchronous task on a background thread.
        /// </summary>
        public Task ExecuteOnThreadPool(Action action)
        {
            Interlocked.Increment(ref _taskCount);
            
            return Task.Run(() =>
            {
                try
                {
                    action();
                }
                finally
                {
                    Interlocked.Increment(ref _completedTaskCount);
                }
            });
        }

        /// <summary>
        /// Executes a callback on the main Unity thread.
        /// </summary>
        public void ExecuteOnMainThread(Action action)
        {
            if (_mainThreadContext != null)
            {
                _mainThreadContext.Post(_ => action(), null);
            }
            else
            {
                // Fallback if no synchronization context is available
                action();
            }
        }

        /// <summary>
        /// Gets current concurrency statistics.
        /// </summary>
        public Dictionary<string, object> GetStats()
        {
            return new Dictionary<string, object>
            {
                { "cpuCores", DeviceInfo.TotalCores },
                { "platform", DeviceInfo.Platform },
                { "isLowEndDevice", DeviceInfo.IsLowEndDevice },
                { "systemMemoryMB", DeviceInfo.SystemMemoryMB },
                { "backgroundSemaphore", new Dictionary<string, int>
                    {
                        { "available", _backgroundSemaphore.CurrentCount },
                        { "max", GetBackgroundPoolConfig().MaxConcurrency }
                    }
                },
                { "networkSemaphore", new Dictionary<string, int>
                    {
                        { "available", _networkSemaphore.CurrentCount },
                        { "max", GetNetworkPoolConfig().MaxConcurrency }
                    }
                },
                { "computeSemaphore", new Dictionary<string, int>
                    {
                        { "available", _computeSemaphore.CurrentCount },
                        { "max", GetComputePoolConfig().MaxConcurrency }
                    }
                },
                { "totalTasksSubmitted", _taskCount },
                { "totalTasksCompleted", _completedTaskCount },
                { "totalTasksRejected", _rejectedTaskCount }
            };
        }

        private static CpuInfo DetectCpuInfo()
        {
            int cores = SystemInfo.processorCount;
            int memoryMB = SystemInfo.systemMemorySize;
            string platform = Application.platform.ToString();
            
            // Determine if this is a low-end device
            bool isLowEnd = cores <= 2 || memoryMB < 2048;
            
            // Detect graphics type
            GraphicsDeviceType graphicsType = GraphicsDeviceType.Unknown;
            var unityGraphicsType = SystemInfo.graphicsDeviceType;
            
            if (unityGraphicsType.ToString().Contains("OpenGL"))
                graphicsType = GraphicsDeviceType.OpenGL;
            else if (unityGraphicsType.ToString().Contains("Metal"))
                graphicsType = GraphicsDeviceType.Metal;
            else if (unityGraphicsType.ToString().Contains("Vulkan"))
                graphicsType = GraphicsDeviceType.Vulkan;
            else if (unityGraphicsType.ToString().Contains("Direct"))
                graphicsType = GraphicsDeviceType.DirectX;
            
            return new CpuInfo
            {
                TotalCores = cores,
                Platform = platform,
                SystemMemoryMB = memoryMB,
                IsLowEndDevice = isLowEnd,
                GraphicsType = graphicsType
            };
        }

        public void Dispose()
        {
            if (_disposed) return;
            
            _backgroundSemaphore?.Dispose();
            _networkSemaphore?.Dispose();
            _computeSemaphore?.Dispose();
            
            _disposed = true;
        }

        /// <summary>
        /// Resets the singleton instance (for testing purposes).
        /// </summary>
        public static void Reset()
        {
            lock (_lock)
            {
                _instance?.Dispose();
                _instance = null;
            }
        }
    }
}
