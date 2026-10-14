import 'dart:async';
import 'package:flutter/services.dart';
import 'ad_types.dart';

/// Interstitial ad manager
class InterstitialAd {
  static const MethodChannel _channel = MethodChannel('io.adproject/sdk');

  final String adUnitId;
  bool _isLoaded = false;

  final _onLoadedController = StreamController<AdLoadResult>.broadcast();
  final _onFailedController = StreamController<AdError>.broadcast();
  final _onOpenedController = StreamController<void>.broadcast();
  final _onClosedController = StreamController<void>.broadcast();
  final _onClickedController = StreamController<void>.broadcast();
  final _onImpressionController = StreamController<ImpressionData>.broadcast();

  /// Stream emitted when ad is loaded
  Stream<AdLoadResult> get onLoaded => _onLoadedController.stream;

  /// Stream emitted when ad fails to load
  Stream<AdError> get onFailed => _onFailedController.stream;

  /// Stream emitted when ad opens
  Stream<void> get onOpened => _onOpenedController.stream;

  /// Stream emitted when ad closes
  Stream<void> get onClosed => _onClosedController.stream;

  /// Stream emitted when ad is clicked
  Stream<void> get onClicked => _onClickedController.stream;

  /// Stream emitted when impression is recorded
  Stream<ImpressionData> get onImpression => _onImpressionController.stream;

  /// Whether the ad is loaded and ready to show
  bool get isLoaded => _isLoaded;

  InterstitialAd({required this.adUnitId}) {
    _setupCallbackChannel();
  }

  void _setupCallbackChannel() {
    final callbackChannel = MethodChannel('io.adproject/interstitial_$adUnitId');
    callbackChannel.setMethodCallHandler(_handleMethodCall);
  }

  Future<void> _handleMethodCall(MethodCall call) async {
    switch (call.method) {
      case 'onAdLoaded':
        _isLoaded = true;
        final result = AdLoadResult.fromMap(
          Map<String, dynamic>.from(call.arguments),
        );
        _onLoadedController.add(result);
        break;

      case 'onAdFailedToLoad':
        _isLoaded = false;
        final error = AdError.fromMap(
          Map<String, dynamic>.from(call.arguments),
        );
        _onFailedController.add(error);
        break;

      case 'onAdOpened':
        _onOpenedController.add(null);
        break;

      case 'onAdClosed':
        _isLoaded = false;
        _onClosedController.add(null);
        break;

      case 'onAdClicked':
        _onClickedController.add(null);
        break;

      case 'onAdImpression':
        final data = ImpressionData.fromMap(
          Map<String, dynamic>.from(call.arguments),
        );
        _onImpressionController.add(data);
        break;
    }
  }

  /// Load the interstitial ad
  Future<AdLoadResult> load() async {
    final result = await _channel.invokeMapMethod<String, dynamic>(
      'loadInterstitialAd',
      {'adUnitId': adUnitId},
    );

    final loadResult = AdLoadResult.fromMap(result!);
    _isLoaded = loadResult.success;
    return loadResult;
  }

  /// Show the interstitial ad
  /// Throws if ad is not loaded
  Future<void> show() async {
    if (!_isLoaded) {
      throw StateError('Interstitial ad not loaded. Call load() first.');
    }

    await _channel.invokeMethod('showInterstitialAd', {'adUnitId': adUnitId});
  }

  /// Check if an interstitial is ready to show
  Future<bool> isReady() async {
    final result = await _channel.invokeMethod<bool>(
      'isInterstitialReady',
      {'adUnitId': adUnitId},
    );
    return result ?? false;
  }

  /// Dispose of the interstitial and release resources
  void dispose() {
    _onLoadedController.close();
    _onFailedController.close();
    _onOpenedController.close();
    _onClosedController.close();
    _onClickedController.close();
    _onImpressionController.close();
  }
}

/// Static helper for loading and showing interstitials
class InterstitialAdLoader {
  static const MethodChannel _channel = MethodChannel('io.adproject/sdk');

  /// Load an interstitial ad
  static Future<AdLoadResult> load(String adUnitId) async {
    final result = await _channel.invokeMapMethod<String, dynamic>(
      'loadInterstitialAd',
      {'adUnitId': adUnitId},
    );
    return AdLoadResult.fromMap(result!);
  }

  /// Show a loaded interstitial ad
  static Future<void> show(String adUnitId) async {
    await _channel.invokeMethod('showInterstitialAd', {'adUnitId': adUnitId});
  }

  /// Check if an interstitial is ready
  static Future<bool> isReady(String adUnitId) async {
    final result = await _channel.invokeMethod<bool>(
      'isInterstitialReady',
      {'adUnitId': adUnitId},
    );
    return result ?? false;
  }
}
