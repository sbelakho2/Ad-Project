using System;
using System.Collections.Generic;
using System.Net.Http;
using System.Net.Http.Headers;
using System.Threading;
using System.Threading.Tasks;
using Newtonsoft.Json;
using Newtonsoft.Json.Linq;
using UnityEngine;

namespace Apex.Mediation.Editor.Console
{
    internal sealed class ConsoleApiClient : IDisposable
    {
        private readonly HttpClient _httpClient;

        public ConsoleApiClient(HttpClient? httpClient = null)
        {
            _httpClient = httpClient ?? new HttpClient();
        }

        public async Task<ConsoleSummary> FetchSummaryAsync(CancellationToken token)
        {
            var settings = ConsoleApiSettings.Instance;
            if (settings.UseMockData)
            {
                await Task.Yield();
                return ConsoleMockDataset.Create();
            }

            var placementsTask = FetchPlacementsAsync(token);
            var adaptersTask = FetchAdaptersAsync(token);
            await Task.WhenAll(placementsTask, adaptersTask);
            return new ConsoleSummary(await placementsTask, await adaptersTask);
        }

        public void Dispose()
        {
            _httpClient.Dispose();
        }

        private async Task<IReadOnlyList<ConsolePlacement>> FetchPlacementsAsync(CancellationToken token)
        {
            var body = await SendAsync("/placements?limit=200&page=1", token);
            return ParseArrayResponse<ConsolePlacement>(body);
        }

        private async Task<IReadOnlyList<ConsoleAdapterConfig>> FetchAdaptersAsync(CancellationToken token)
        {
            var body = await SendAsync("/adapters", token);
            return ParseArrayResponse<ConsoleAdapterConfig>(body);
        }

        private async Task<string> SendAsync(string path, CancellationToken token)
        {
            var settings = ConsoleApiSettings.Instance;
            var baseUrl = settings.ApiBaseUrl.TrimEnd('/') + path;
            using var request = new HttpRequestMessage(HttpMethod.Get, baseUrl);
            request.Headers.Accept.Add(new MediaTypeWithQualityHeaderValue("application/json"));
            request.Headers.UserAgent.ParseAdd("ApexMediationUnityEditor/0.1.0");

            if (!string.IsNullOrEmpty(settings.AccessToken))
            {
                request.Headers.Authorization = new AuthenticationHeaderValue("Bearer", settings.AccessToken);
            }

            _httpClient.Timeout = TimeSpan.FromSeconds(settings.RequestTimeoutSeconds);

            try
            {
                using var response = await _httpClient.SendAsync(request, token);
                var payload = await response.Content.ReadAsStringAsync();
                if (!response.IsSuccessStatusCode)
                {
                    throw new ConsoleApiException($"Console API {response.StatusCode}: {payload}");
                }

                return payload;
            }
            catch (OperationCanceledException) when (token.IsCancellationRequested)
            {
                throw;
            }
            catch (Exception ex)
            {
                Debug.LogWarning($"Console API request failed: {ex.Message}");
                throw new ConsoleApiException("Failed to reach console API", ex);
            }
        }

        private static IReadOnlyList<T> ParseArrayResponse<T>(string json)
        {
            if (string.IsNullOrWhiteSpace(json))
            {
                return Array.Empty<T>();
            }

            try
            {
                var root = JObject.Parse(json);
                var dataNode = root["data"];

                if (dataNode is JObject dataObj && dataObj["items"] is JArray itemsArray)
                {
                    return DeserializeArray<T>(itemsArray);
                }

                if (dataNode is JArray directArray)
                {
                    return DeserializeArray<T>(directArray);
                }

                // Some endpoints already return the entity directly as data
                if (dataNode != null)
                {
                    var single = dataNode.ToObject<T>();
                    return single == null ? Array.Empty<T>() : new[] { single };
                }
            }
            catch (JsonException ex)
            {
                Debug.LogWarning($"Console API parse error: {ex.Message}");
            }

            return Array.Empty<T>();
        }

        private static IReadOnlyList<T> DeserializeArray<T>(JArray array)
        {
            var list = array.ToObject<List<T>>();
            return list ?? Array.Empty<T>();
        }
    }
}
