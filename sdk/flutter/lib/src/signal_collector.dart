import 'package:flutter/services.dart';

/// Signal collector for S2S (Server-to-Server) header bidding
class SignalCollector {
  static const MethodChannel _channel = MethodChannel('io.adproject/sdk');

  /// Collect bid signals from all available networks
  static Future<List<BidSignal>> collectSignals() async {
    final result = await _channel.invokeListMethod<Map<dynamic, dynamic>>(
      'collectSignals',
      {},
    );

    return result
            ?.map((e) => BidSignal.fromMap(Map<String, dynamic>.from(e)))
            .toList() ??
        [];
  }

  /// Collect bid signals from specific networks
  static Future<List<BidSignal>> collectSignalsFromNetworks(
    List<String> networkNames,
  ) async {
    final result = await _channel.invokeListMethod<Map<dynamic, dynamic>>(
      'collectSignals',
      {'networks': networkNames},
    );

    return result
            ?.map((e) => BidSignal.fromMap(Map<String, dynamic>.from(e)))
            .toList() ??
        [];
  }

  /// Collect bid signal from a single network
  static Future<BidSignal?> collectSignal(String networkName) async {
    final result = await _channel.invokeMapMethod<String, dynamic>(
      'collectSignal',
      {'networkName': networkName},
    );

    if (result == null) return null;
    return BidSignal.fromMap(result);
  }

  /// Get device signals for contextual targeting
  static Future<DeviceSignals> getDeviceSignals() async {
    final result = await _channel.invokeMapMethod<String, dynamic>(
      'getDeviceSignals',
    );
    return DeviceSignals.fromMap(result!);
  }

  /// Get consent signals for privacy compliance
  static Future<ConsentSignals> getConsentSignals() async {
    final result = await _channel.invokeMapMethod<String, dynamic>(
      'getConsentSignals',
    );
    return ConsentSignals.fromMap(result!);
  }
}

/// Bid signal from a network adapter
class BidSignal {
  final String networkName;
  final String token;
  final int timestamp;
  final int expiresAt;
  final Map<String, dynamic>? extras;

  const BidSignal({
    required this.networkName,
    required this.token,
    required this.timestamp,
    required this.expiresAt,
    this.extras,
  });

  factory BidSignal.fromMap(Map<String, dynamic> map) => BidSignal(
        networkName: map['networkName'] as String,
        token: map['token'] as String,
        timestamp: map['timestamp'] as int,
        expiresAt: map['expiresAt'] as int,
        extras: map['extras'] != null
            ? Map<String, dynamic>.from(map['extras'])
            : null,
      );

  Map<String, dynamic> toMap() => {
        'networkName': networkName,
        'token': token,
        'timestamp': timestamp,
        'expiresAt': expiresAt,
        if (extras != null) 'extras': extras,
      };

  /// Check if signal is still valid
  bool get isValid =>
      DateTime.now().millisecondsSinceEpoch < expiresAt;

  /// Time remaining until expiration in milliseconds
  int get timeToLive =>
      expiresAt - DateTime.now().millisecondsSinceEpoch;
}

/// Device signals for contextual targeting
class DeviceSignals {
  final String? deviceId;
  final String? advertisingId;
  final bool? limitAdTracking;
  final String? osVersion;
  final String? deviceModel;
  final String? deviceManufacturer;
  final String? carrier;
  final String? connectionType;
  final String? language;
  final String? country;
  final int? screenWidth;
  final int? screenHeight;
  final double? screenDensity;
  final String? bundleId;
  final String? appVersion;

  const DeviceSignals({
    this.deviceId,
    this.advertisingId,
    this.limitAdTracking,
    this.osVersion,
    this.deviceModel,
    this.deviceManufacturer,
    this.carrier,
    this.connectionType,
    this.language,
    this.country,
    this.screenWidth,
    this.screenHeight,
    this.screenDensity,
    this.bundleId,
    this.appVersion,
  });

  factory DeviceSignals.fromMap(Map<String, dynamic> map) => DeviceSignals(
        deviceId: map['deviceId'] as String?,
        advertisingId: map['advertisingId'] as String?,
        limitAdTracking: map['limitAdTracking'] as bool?,
        osVersion: map['osVersion'] as String?,
        deviceModel: map['deviceModel'] as String?,
        deviceManufacturer: map['deviceManufacturer'] as String?,
        carrier: map['carrier'] as String?,
        connectionType: map['connectionType'] as String?,
        language: map['language'] as String?,
        country: map['country'] as String?,
        screenWidth: map['screenWidth'] as int?,
        screenHeight: map['screenHeight'] as int?,
        screenDensity: (map['screenDensity'] as num?)?.toDouble(),
        bundleId: map['bundleId'] as String?,
        appVersion: map['appVersion'] as String?,
      );

  Map<String, dynamic> toMap() => {
        if (deviceId != null) 'deviceId': deviceId,
        if (advertisingId != null) 'advertisingId': advertisingId,
        if (limitAdTracking != null) 'limitAdTracking': limitAdTracking,
        if (osVersion != null) 'osVersion': osVersion,
        if (deviceModel != null) 'deviceModel': deviceModel,
        if (deviceManufacturer != null) 'deviceManufacturer': deviceManufacturer,
        if (carrier != null) 'carrier': carrier,
        if (connectionType != null) 'connectionType': connectionType,
        if (language != null) 'language': language,
        if (country != null) 'country': country,
        if (screenWidth != null) 'screenWidth': screenWidth,
        if (screenHeight != null) 'screenHeight': screenHeight,
        if (screenDensity != null) 'screenDensity': screenDensity,
        if (bundleId != null) 'bundleId': bundleId,
        if (appVersion != null) 'appVersion': appVersion,
      };
}

/// Consent signals for privacy compliance
class ConsentSignals {
  final bool? hasConsent;
  final String? tcfConsentString;
  final String? uspString;
  final bool? gdprApplies;
  final bool? coppaApplies;
  final Map<String, bool>? vendorConsents;

  const ConsentSignals({
    this.hasConsent,
    this.tcfConsentString,
    this.uspString,
    this.gdprApplies,
    this.coppaApplies,
    this.vendorConsents,
  });

  factory ConsentSignals.fromMap(Map<String, dynamic> map) => ConsentSignals(
        hasConsent: map['hasConsent'] as bool?,
        tcfConsentString: map['tcfConsentString'] as String?,
        uspString: map['uspString'] as String?,
        gdprApplies: map['gdprApplies'] as bool?,
        coppaApplies: map['coppaApplies'] as bool?,
        vendorConsents: map['vendorConsents'] != null
            ? Map<String, bool>.from(map['vendorConsents'])
            : null,
      );

  Map<String, dynamic> toMap() => {
        if (hasConsent != null) 'hasConsent': hasConsent,
        if (tcfConsentString != null) 'tcfConsentString': tcfConsentString,
        if (uspString != null) 'uspString': uspString,
        if (gdprApplies != null) 'gdprApplies': gdprApplies,
        if (coppaApplies != null) 'coppaApplies': coppaApplies,
        if (vendorConsents != null) 'vendorConsents': vendorConsents,
      };
}
