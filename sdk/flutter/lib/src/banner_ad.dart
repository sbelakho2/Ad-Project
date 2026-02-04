import 'dart:async';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'ad_types.dart';

/// Banner ad widget
class BannerAd extends StatefulWidget {
  final String adUnitId;
  final AdSize size;
  final bool autoRefresh;
  final Duration refreshInterval;
  final void Function(AdLoadResult)? onAdLoaded;
  final void Function(AdError)? onAdFailedToLoad;
  final VoidCallback? onAdOpened;
  final VoidCallback? onAdClosed;
  final VoidCallback? onAdClicked;
  final void Function(ImpressionData)? onAdImpression;

  const BannerAd({
    super.key,
    required this.adUnitId,
    this.size = AdSize.banner,
    this.autoRefresh = false,
    this.refreshInterval = const Duration(seconds: 60),
    this.onAdLoaded,
    this.onAdFailedToLoad,
    this.onAdOpened,
    this.onAdClosed,
    this.onAdClicked,
    this.onAdImpression,
  });

  @override
  State<BannerAd> createState() => _BannerAdState();
}

class _BannerAdState extends State<BannerAd> {
  static const String _viewType = 'io.adproject/banner_ad';
  bool _isLoaded = false;
  Timer? _refreshTimer;

  @override
  void initState() {
    super.initState();
    if (widget.autoRefresh) {
      _startRefreshTimer();
    }
  }

  @override
  void dispose() {
    _refreshTimer?.cancel();
    super.dispose();
  }

  void _startRefreshTimer() {
    _refreshTimer?.cancel();
    _refreshTimer = Timer.periodic(widget.refreshInterval, (_) {
      if (_isLoaded) {
        // Trigger refresh via method channel
        _refreshAd();
      }
    });
  }

  Future<void> _refreshAd() async {
    try {
      await const MethodChannel('io.adproject/sdk').invokeMethod(
        'refreshBannerAd',
        {'adUnitId': widget.adUnitId},
      );
    } catch (e) {
      debugPrint('Banner refresh failed: $e');
    }
  }

  void _onPlatformViewCreated(int id) {
    final channel = MethodChannel('io.adproject/banner_ad_$id');
    channel.setMethodCallHandler(_handleMethodCall);
  }

  Future<dynamic> _handleMethodCall(MethodCall call) async {
    switch (call.method) {
      case 'onAdLoaded':
        setState(() => _isLoaded = true);
        final result = AdLoadResult.fromMap(
          Map<String, dynamic>.from(call.arguments),
        );
        widget.onAdLoaded?.call(result);
        break;

      case 'onAdFailedToLoad':
        setState(() => _isLoaded = false);
        final error = AdError.fromMap(
          Map<String, dynamic>.from(call.arguments),
        );
        widget.onAdFailedToLoad?.call(error);
        break;

      case 'onAdOpened':
        widget.onAdOpened?.call();
        break;

      case 'onAdClosed':
        widget.onAdClosed?.call();
        break;

      case 'onAdClicked':
        widget.onAdClicked?.call();
        break;

      case 'onAdImpression':
        final data = ImpressionData.fromMap(
          Map<String, dynamic>.from(call.arguments),
        );
        widget.onAdImpression?.call(data);
        break;
    }
  }

  @override
  Widget build(BuildContext context) {
    final creationParams = <String, dynamic>{
      'adUnitId': widget.adUnitId,
      'width': widget.size.width,
      'height': widget.size.height,
      'isAdaptive': widget.size.isAdaptive,
    };

    return SizedBox(
      width: widget.size.width.toDouble(),
      height: widget.size.isAdaptive ? null : widget.size.height.toDouble(),
      child: defaultTargetPlatform == TargetPlatform.android
          ? AndroidView(
              viewType: _viewType,
              creationParams: creationParams,
              creationParamsCodec: const StandardMessageCodec(),
              onPlatformViewCreated: _onPlatformViewCreated,
            )
          : UiKitView(
              viewType: _viewType,
              creationParams: creationParams,
              creationParamsCodec: const StandardMessageCodec(),
              onPlatformViewCreated: _onPlatformViewCreated,
            ),
    );
  }
}

/// Controller for managing banner ads imperatively
class BannerAdController {
  static const MethodChannel _channel = MethodChannel('io.adproject/sdk');
  final String adUnitId;
  final AdSize size;

  bool _isLoaded = false;
  bool get isLoaded => _isLoaded;

  BannerAdController({
    required this.adUnitId,
    this.size = AdSize.banner,
  });

  /// Load the banner ad
  Future<AdLoadResult> load() async {
    final result = await _channel.invokeMapMethod<String, dynamic>(
      'loadBannerAd',
      {
        'adUnitId': adUnitId,
        'width': size.width,
        'height': size.height,
        'isAdaptive': size.isAdaptive,
      },
    );

    final loadResult = AdLoadResult.fromMap(result!);
    _isLoaded = loadResult.success;
    return loadResult;
  }

  /// Show the banner ad
  Future<void> show() async {
    await _channel.invokeMethod('showBannerAd', {'adUnitId': adUnitId});
  }

  /// Hide the banner ad
  Future<void> hide() async {
    await _channel.invokeMethod('hideBannerAd', {'adUnitId': adUnitId});
  }

  /// Destroy the banner ad and release resources
  Future<void> dispose() async {
    await _channel.invokeMethod('destroyBannerAd', {'adUnitId': adUnitId});
    _isLoaded = false;
  }
}
