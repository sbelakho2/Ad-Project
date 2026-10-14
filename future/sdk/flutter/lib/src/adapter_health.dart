import 'package:flutter/services.dart';
import 'ad_types.dart';

/// Adapter health monitoring
class AdapterHealthManager {
  static const MethodChannel _channel = MethodChannel('io.adproject/sdk');

  /// Get health status for a specific adapter
  static Future<AdapterHealth?> getAdapterHealth(String networkName) async {
    final result = await _channel.invokeMapMethod<String, dynamic>(
      'getAdapterHealth',
      {'networkName': networkName},
    );

    if (result == null) return null;
    return AdapterHealth.fromMap(result);
  }

  /// Get health status for all adapters
  static Future<Map<String, AdapterHealth>> getAllAdapterHealth() async {
    final result = await _channel.invokeMapMethod<String, dynamic>(
      'getAllAdapterHealth',
    );

    if (result == null) return {};

    return result.map((key, value) => MapEntry(
          key,
          AdapterHealth.fromMap(Map<String, dynamic>.from(value)),
        ));
  }

  /// Get list of adapters that are ready to serve ads
  static Future<List<String>> getReadyAdapters() async {
    final result = await _channel.invokeListMethod<String>('getReadyAdapters');
    return result ?? [];
  }

  /// Get list of adapters that failed initialization
  static Future<List<AdapterHealth>> getFailedAdapters() async {
    final result = await _channel.invokeListMethod<Map<dynamic, dynamic>>(
      'getFailedAdapters',
    );

    return result
            ?.map((e) => AdapterHealth.fromMap(Map<String, dynamic>.from(e)))
            .toList() ??
        [];
  }

  /// Check if a specific adapter is ready
  static Future<bool> isAdapterReady(String networkName) async {
    final health = await getAdapterHealth(networkName);
    return health?.isReady ?? false;
  }
}

/// Health status for an individual adapter
class AdapterHealth {
  final String networkName;
  final AdapterInitStatus status;
  final String? sdkVersion;
  final String? adapterVersion;
  final DateTime lastStatusChange;
  final String? errorMessage;
  final String? errorCode;
  final AdapterCapabilities capabilities;
  final AdapterMetrics metrics;

  const AdapterHealth({
    required this.networkName,
    required this.status,
    this.sdkVersion,
    this.adapterVersion,
    required this.lastStatusChange,
    this.errorMessage,
    this.errorCode,
    required this.capabilities,
    required this.metrics,
  });

  factory AdapterHealth.fromMap(Map<String, dynamic> map) => AdapterHealth(
        networkName: map['networkName'] as String,
        status: AdapterInitStatus.values.firstWhere(
          (e) => e.name == map['status'] || e.toString() == map['status'],
          orElse: () => AdapterInitStatus.notInitialized,
        ),
        sdkVersion: map['sdkVersion'] as String?,
        adapterVersion: map['adapterVersion'] as String?,
        lastStatusChange: DateTime.fromMillisecondsSinceEpoch(
          map['lastStatusChange'] as int,
        ),
        errorMessage: map['errorMessage'] as String?,
        errorCode: map['errorCode'] as String?,
        capabilities: AdapterCapabilities.fromMap(
          Map<String, dynamic>.from(map['capabilities'] ?? {}),
        ),
        metrics: AdapterMetrics.fromMap(
          Map<String, dynamic>.from(map['metrics'] ?? {}),
        ),
      );

  bool get isReady =>
      status == AdapterInitStatus.initializedSuccess ||
      status == AdapterInitStatus.doesNotApply;

  Map<String, dynamic> toMap() => {
        'networkName': networkName,
        'status': status.name,
        'sdkVersion': sdkVersion,
        'adapterVersion': adapterVersion,
        'lastStatusChange': lastStatusChange.millisecondsSinceEpoch,
        'errorMessage': errorMessage,
        'errorCode': errorCode,
        'isReady': isReady,
        'capabilities': capabilities.toMap(),
        'metrics': metrics.toMap(),
      };
}

/// Capabilities supported by an adapter
class AdapterCapabilities {
  final bool supportsBanner;
  final bool supportsInterstitial;
  final bool supportsRewarded;
  final bool supportsRewardedInterstitial;
  final bool supportsNative;
  final bool supportsAppOpen;
  final bool supportsSignalCollection;
  final bool supportsS2SBidding;

  const AdapterCapabilities({
    this.supportsBanner = false,
    this.supportsInterstitial = false,
    this.supportsRewarded = false,
    this.supportsRewardedInterstitial = false,
    this.supportsNative = false,
    this.supportsAppOpen = false,
    this.supportsSignalCollection = false,
    this.supportsS2SBidding = false,
  });

  factory AdapterCapabilities.fromMap(Map<String, dynamic> map) =>
      AdapterCapabilities(
        supportsBanner: map['banner'] as bool? ?? false,
        supportsInterstitial: map['interstitial'] as bool? ?? false,
        supportsRewarded: map['rewarded'] as bool? ?? false,
        supportsRewardedInterstitial:
            map['rewardedInterstitial'] as bool? ?? false,
        supportsNative: map['native'] as bool? ?? false,
        supportsAppOpen: map['appOpen'] as bool? ?? false,
        supportsSignalCollection: map['signalCollection'] as bool? ?? false,
        supportsS2SBidding: map['s2sBidding'] as bool? ?? false,
      );

  Map<String, bool> toMap() => {
        'banner': supportsBanner,
        'interstitial': supportsInterstitial,
        'rewarded': supportsRewarded,
        'rewardedInterstitial': supportsRewardedInterstitial,
        'native': supportsNative,
        'appOpen': supportsAppOpen,
        'signalCollection': supportsSignalCollection,
        's2sBidding': supportsS2SBidding,
      };

  static const empty = AdapterCapabilities();
}

/// Runtime metrics for an adapter
class AdapterMetrics {
  final int totalRequests;
  final int successfulFills;
  final int noFills;
  final int errors;
  final int timeouts;
  final double avgLoadTimeMs;
  final DateTime? lastRequestTime;

  const AdapterMetrics({
    this.totalRequests = 0,
    this.successfulFills = 0,
    this.noFills = 0,
    this.errors = 0,
    this.timeouts = 0,
    this.avgLoadTimeMs = 0,
    this.lastRequestTime,
  });

  factory AdapterMetrics.fromMap(Map<String, dynamic> map) => AdapterMetrics(
        totalRequests: map['totalRequests'] as int? ?? 0,
        successfulFills: map['successfulFills'] as int? ?? 0,
        noFills: map['noFills'] as int? ?? 0,
        errors: map['errors'] as int? ?? 0,
        timeouts: map['timeouts'] as int? ?? 0,
        avgLoadTimeMs: (map['avgLoadTimeMs'] as num?)?.toDouble() ?? 0,
        lastRequestTime: map['lastRequestTime'] != null
            ? DateTime.fromMillisecondsSinceEpoch(map['lastRequestTime'] as int)
            : null,
      );

  double get fillRate {
    if (totalRequests == 0) return 0;
    return (successfulFills / totalRequests) * 100;
  }

  Map<String, dynamic> toMap() => {
        'totalRequests': totalRequests,
        'successfulFills': successfulFills,
        'noFills': noFills,
        'errors': errors,
        'timeouts': timeouts,
        'avgLoadTimeMs': avgLoadTimeMs,
        'lastRequestTime': lastRequestTime?.millisecondsSinceEpoch,
        'fillRate': fillRate,
      };

  static const empty = AdapterMetrics();
}
