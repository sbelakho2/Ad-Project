using System;
using System.Collections.Concurrent;
using System.Collections.Generic;
using System.Net;
using System.Net.Http;
using System.Threading;
using System.Threading.Tasks;
using UnityEngine;

namespace ApexMediation.Core
{
    /// <summary>
    /// NetworkWarmer provides HTTP preconnect and DNS prefetch functionality
    /// to reduce cold-start latency for auction requests.
    /// 
    /// Key features:
    /// - DNS prefetch on SDK init (caches DNS resolution)
    /// - HTTP preconnect (establishes TLS early)
    /// - Connection pooling warmup
    /// - Thread-safe singleton pattern
    /// </summary>
    public sealed class NetworkWarmer
    {
        private static readonly Lazy<NetworkWarmer> _instance = 
            new Lazy<NetworkWarmer>(() => new NetworkWarmer());
        
        public static NetworkWarmer Instance => _instance.Value;
        
        private readonly ConcurrentDictionary<string, IPAddress[]> _dnsCache = 
            new ConcurrentDictionary<string, IPAddress[]>();
        private readonly HashSet<string> _warmedEndpoints = new HashSet<string>();
        private readonly object _lock = new object();
        private volatile bool _isWarmedUp;
        
        // Shared HttpClient with connection pooling
        private readonly HttpClient _httpClient;
        
        private NetworkWarmer()
        {
            var handler = new HttpClientHandler
            {
                MaxConnectionsPerServer = Math.Max(2, Math.Min(Environment.ProcessorCount, 8)),
                AutomaticDecompression = DecompressionMethods.GZip | DecompressionMethods.Deflate
            };
            
            _httpClient = new HttpClient(handler)
            {
                Timeout = TimeSpan.FromSeconds(5)
            };
        }
        
        /// <summary>
        /// Whether at least one endpoint has been warmed up.
        /// </summary>
        public bool IsWarmedUp => _isWarmedUp;
        
        /// <summary>
        /// Warm up connections to the specified endpoint.
        /// This performs:
        /// 1. DNS prefetch (resolves hostname to IP addresses)
        /// 2. HTTP preconnect (establishes TCP + TLS connection)
        /// </summary>
        /// <param name="endpoint">Base URL of the auction endpoint</param>
        public void Warmup(string endpoint)
        {
            lock (_lock)
            {
                if (_warmedEndpoints.Contains(endpoint))
                    return;
            }
            
            // Run warmup on thread pool to avoid blocking
            Task.Run(async () =>
            {
                try
                {
                    var uri = new Uri(endpoint);
                    var hostname = uri.Host;
                    
                    // Step 1: DNS prefetch
                    await PrefetchDnsAsync(hostname);
                    
                    // Step 2: HTTP preconnect
                    await PreconnectAsync(endpoint);
                    
                    lock (_lock)
                    {
                        _warmedEndpoints.Add(endpoint);
                        _isWarmedUp = true;
                    }
                }
                catch (Exception ex)
                {
                    // Warmup is best-effort
                    Debug.Log($"[ApexMediation] NetworkWarmer warmup failed: {ex.Message}");
                }
            });
        }
        
        /// <summary>
        /// Warm up connections asynchronously.
        /// </summary>
        public async Task WarmupAsync(string endpoint)
        {
            lock (_lock)
            {
                if (_warmedEndpoints.Contains(endpoint))
                    return;
            }
            
            try
            {
                var uri = new Uri(endpoint);
                var hostname = uri.Host;
                
                await PrefetchDnsAsync(hostname);
                await PreconnectAsync(endpoint);
                
                lock (_lock)
                {
                    _warmedEndpoints.Add(endpoint);
                    _isWarmedUp = true;
                }
            }
            catch (Exception ex)
            {
                Debug.Log($"[ApexMediation] NetworkWarmer warmup failed: {ex.Message}");
            }
        }
        
        private async Task PrefetchDnsAsync(string hostname)
        {
            if (_dnsCache.ContainsKey(hostname))
                return;
            
            try
            {
                var addresses = await Dns.GetHostAddressesAsync(hostname);
                if (addresses.Length > 0)
                {
                    _dnsCache[hostname] = addresses;
                }
            }
            catch
            {
                // DNS resolution failed; will retry on actual request
            }
        }
        
        private async Task PreconnectAsync(string endpoint)
        {
            try
            {
                using var cts = new CancellationTokenSource(TimeSpan.FromSeconds(2));
                using var request = new HttpRequestMessage(HttpMethod.Head, endpoint + "/health");
                
                var response = await _httpClient.SendAsync(request, cts.Token);
                response.Dispose();
            }
            catch
            {
                // Preconnect is best-effort
            }
        }
        
        /// <summary>
        /// Get cached DNS entries for a hostname, if available.
        /// </summary>
        public IPAddress[] GetCachedDns(string hostname)
        {
            return _dnsCache.TryGetValue(hostname, out var addresses) ? addresses : null;
        }
        
        /// <summary>
        /// Get the shared HttpClient with pre-warmed connections.
        /// </summary>
        public HttpClient GetWarmedClient() => _httpClient;
        
        /// <summary>
        /// Clear all cached DNS entries and warmed endpoints.
        /// Primarily for testing.
        /// </summary>
        public void Reset()
        {
            lock (_lock)
            {
                _dnsCache.Clear();
                _warmedEndpoints.Clear();
                _isWarmedUp = false;
            }
        }
        
        /// <summary>
        /// Get diagnostics about current warmup state.
        /// </summary>
        public Dictionary<string, object> GetDiagnostics()
        {
            lock (_lock)
            {
                return new Dictionary<string, object>
                {
                    { "isWarmedUp", _isWarmedUp },
                    { "warmedEndpoints", new List<string>(_warmedEndpoints) },
                    { "dnsCacheSize", _dnsCache.Count },
                    { "cachedHosts", new List<string>(_dnsCache.Keys) },
                    { "connectionPoolSize", Math.Max(2, Math.Min(Environment.ProcessorCount, 8)) }
                };
            }
        }
    }
}
