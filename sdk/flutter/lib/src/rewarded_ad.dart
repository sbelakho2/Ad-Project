import 'dart:async';
import 'package:flutter/services.dart';
import 'ad_types.dart';

/// Rewarded ad manager
class RewardedAd {
  static const MethodChannel _channel = MethodChannel('io.adproject/sdk');

  final String adUnitId;
  bool _isLoaded = false;

  final _onLoadedController = StreamController<AdLoadResult>.broadcast();
  final _onFailedController = StreamController<AdError>.broadcast();
  final _onOpenedController = StreamController<void>.broadcast();
  final _onClosedController = StreamController<void>.broadcast();
  final _onClickedController = StreamController<void>.broadcast();
  final _onRewardController = StreamController<Reward>.broadcast();
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

  /// Stream emitted when user earns a reward
  Stream<Reward> get onReward => _onRewardController.stream;

  /// Stream emitted when impression is recorded
  Stream<ImpressionData> get onImpression => _onImpressionController.stream;

  /// Whether the ad is loaded and ready to show
  bool get isLoaded => _isLoaded;

  RewardedAd({required this.adUnitId}) {
    _setupCallbackChannel();
  }

  void _setupCallbackChannel() {
    final callbackChannel = MethodChannel('io.adproject/rewarded_$adUnitId');
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

      case 'onRewardEarned':
        final reward = Reward.fromMap(
          Map<String, dynamic>.from(call.arguments),
        );
        _onRewardController.add(reward);
        break;

      case 'onAdImpression':
        final data = ImpressionData.fromMap(
          Map<String, dynamic>.from(call.arguments),
        );
        _onImpressionController.add(data);
        break;
    }
  }

  /// Load the rewarded ad
  Future<AdLoadResult> load() async {
    final result = await _channel.invokeMapMethod<String, dynamic>(
      'loadRewardedAd',
      {'adUnitId': adUnitId},
    );

    final loadResult = AdLoadResult.fromMap(result!);
    _isLoaded = loadResult.success;
    return loadResult;
  }

  /// Show the rewarded ad
  /// Returns the reward if the user completes the ad, null otherwise
  Future<Reward?> show() async {
    if (!_isLoaded) {
      throw StateError('Rewarded ad not loaded. Call load() first.');
    }

    final result = await _channel.invokeMapMethod<String, dynamic>(
      'showRewardedAd',
      {'adUnitId': adUnitId},
    );

    if (result != null && result.containsKey('type')) {
      return Reward.fromMap(result);
    }
    return null;
  }

  /// Check if a rewarded ad is ready to show
  Future<bool> isReady() async {
    final result = await _channel.invokeMethod<bool>(
      'isRewardedReady',
      {'adUnitId': adUnitId},
    );
    return result ?? false;
  }

  /// Dispose of the rewarded ad and release resources
  void dispose() {
    _onLoadedController.close();
    _onFailedController.close();
    _onOpenedController.close();
    _onClosedController.close();
    _onClickedController.close();
    _onRewardController.close();
    _onImpressionController.close();
  }
}

/// Rewarded interstitial ad manager
class RewardedInterstitialAd {
  static const MethodChannel _channel = MethodChannel('io.adproject/sdk');

  final String adUnitId;
  bool _isLoaded = false;

  final _onLoadedController = StreamController<AdLoadResult>.broadcast();
  final _onFailedController = StreamController<AdError>.broadcast();
  final _onOpenedController = StreamController<void>.broadcast();
  final _onClosedController = StreamController<void>.broadcast();
  final _onRewardController = StreamController<Reward>.broadcast();

  Stream<AdLoadResult> get onLoaded => _onLoadedController.stream;
  Stream<AdError> get onFailed => _onFailedController.stream;
  Stream<void> get onOpened => _onOpenedController.stream;
  Stream<void> get onClosed => _onClosedController.stream;
  Stream<Reward> get onReward => _onRewardController.stream;

  bool get isLoaded => _isLoaded;

  RewardedInterstitialAd({required this.adUnitId}) {
    _setupCallbackChannel();
  }

  void _setupCallbackChannel() {
    final channel = MethodChannel('io.adproject/rewarded_interstitial_$adUnitId');
    channel.setMethodCallHandler(_handleMethodCall);
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

      case 'onRewardEarned':
        final reward = Reward.fromMap(
          Map<String, dynamic>.from(call.arguments),
        );
        _onRewardController.add(reward);
        break;
    }
  }

  Future<AdLoadResult> load() async {
    final result = await _channel.invokeMapMethod<String, dynamic>(
      'loadRewardedInterstitialAd',
      {'adUnitId': adUnitId},
    );

    final loadResult = AdLoadResult.fromMap(result!);
    _isLoaded = loadResult.success;
    return loadResult;
  }

  Future<Reward?> show() async {
    if (!_isLoaded) {
      throw StateError('Rewarded interstitial not loaded. Call load() first.');
    }

    final result = await _channel.invokeMapMethod<String, dynamic>(
      'showRewardedInterstitialAd',
      {'adUnitId': adUnitId},
    );

    if (result != null && result.containsKey('type')) {
      return Reward.fromMap(result);
    }
    return null;
  }

  void dispose() {
    _onLoadedController.close();
    _onFailedController.close();
    _onOpenedController.close();
    _onClosedController.close();
    _onRewardController.close();
  }
}

/// App open ad manager
class AppOpenAd {
  static const MethodChannel _channel = MethodChannel('io.adproject/sdk');

  final String adUnitId;
  bool _isLoaded = false;

  final _onLoadedController = StreamController<AdLoadResult>.broadcast();
  final _onFailedController = StreamController<AdError>.broadcast();
  final _onOpenedController = StreamController<void>.broadcast();
  final _onClosedController = StreamController<void>.broadcast();

  Stream<AdLoadResult> get onLoaded => _onLoadedController.stream;
  Stream<AdError> get onFailed => _onFailedController.stream;
  Stream<void> get onOpened => _onOpenedController.stream;
  Stream<void> get onClosed => _onClosedController.stream;

  bool get isLoaded => _isLoaded;

  AppOpenAd({required this.adUnitId}) {
    _setupCallbackChannel();
  }

  void _setupCallbackChannel() {
    final channel = MethodChannel('io.adproject/app_open_$adUnitId');
    channel.setMethodCallHandler(_handleMethodCall);
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
    }
  }

  Future<AdLoadResult> load() async {
    final result = await _channel.invokeMapMethod<String, dynamic>(
      'loadAppOpenAd',
      {'adUnitId': adUnitId},
    );

    final loadResult = AdLoadResult.fromMap(result!);
    _isLoaded = loadResult.success;
    return loadResult;
  }

  Future<void> show() async {
    if (!_isLoaded) {
      throw StateError('App open ad not loaded. Call load() first.');
    }

    await _channel.invokeMethod('showAppOpenAd', {'adUnitId': adUnitId});
  }

  void dispose() {
    _onLoadedController.close();
    _onFailedController.close();
    _onOpenedController.close();
    _onClosedController.close();
  }
}
