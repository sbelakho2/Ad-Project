import 'dart:async';
import 'package:flutter/services.dart';
import 'ad_types.dart';

/// Main entry point for the AdProject SDK
class AdProjectSdk {
  static const MethodChannel _channel = MethodChannel('io.adproject/sdk');
  static const EventChannel _eventChannel =
      EventChannel('io.adproject/sdk_events');

  static AdProjectSdk? _instance;
  static AdProjectSdk get instance => _instance ??= AdProjectSdk._();

  bool _initialized = false;
  StreamSubscription? _eventSubscription;
  final _eventController = StreamController<AdEvent>.broadcast();

  AdProjectSdk._() {
    _setupEventListener();
  }

  void _setupEventListener() {
    _eventSubscription = _eventChannel
        .receiveBroadcastStream()
        .map((event) => AdEvent.fromMap(event as Map<String, dynamic>))
        .listen((event) {
      _eventController.add(event);
    });
  }

  /// Stream of all ad events
  Stream<AdEvent> get events => _eventController.stream;

  /// Whether the SDK has been initialized
  bool get isInitialized => _initialized;

  /// Initialize the SDK
  Future<InitializationStatus> initialize(
    String appId, {
    bool testMode = false,
    bool verbose = false,
  }) async {
    if (_initialized) {
      return getInitializationStatus();
    }

    final result = await _channel.invokeMapMethod<String, dynamic>(
      'initialize',
      {
        'appId': appId,
        'testMode': testMode,
        'verbose': verbose,
      },
    );

    final status = InitializationStatus.fromMap(result!);
    _initialized = status.isInitialized;
    return status;
  }

  /// Get current initialization status
  Future<InitializationStatus> getInitializationStatus() async {
    final result = await _channel.invokeMapMethod<String, dynamic>(
      'getInitializationStatus',
    );
    return InitializationStatus.fromMap(result!);
  }

  /// Set user ID for cross-device tracking
  Future<void> setUserId(String userId) async {
    await _channel.invokeMethod('setUserId', {'userId': userId});
  }

  /// Set user segments for targeting
  Future<void> setUserSegments(List<String> segments) async {
    await _channel.invokeMethod('setUserSegments', {'segments': segments});
  }

  /// Set extra data for custom targeting
  Future<void> setExtraData(String key, String value) async {
    await _channel.invokeMethod('setExtraData', {'key': key, 'value': value});
  }

  /// Enable/disable verbose logging
  Future<void> setVerboseLogging(bool enabled) async {
    await _channel.invokeMethod('setVerboseLogging', {'enabled': enabled});
  }

  /// Enable/disable test mode
  Future<void> setTestMode(bool enabled) async {
    await _channel.invokeMethod('setTestMode', {'enabled': enabled});
  }

  /// Clean up resources
  void dispose() {
    _eventSubscription?.cancel();
    _eventController.close();
    _instance = null;
  }
}

/// Generic ad event
class AdEvent {
  final String type;
  final String adUnitId;
  final Map<String, dynamic> data;

  const AdEvent({
    required this.type,
    required this.adUnitId,
    required this.data,
  });

  factory AdEvent.fromMap(Map<String, dynamic> map) => AdEvent(
        type: map['type'] as String,
        adUnitId: map['adUnitId'] as String,
        data: Map<String, dynamic>.from(map['data'] ?? {}),
      );
}
