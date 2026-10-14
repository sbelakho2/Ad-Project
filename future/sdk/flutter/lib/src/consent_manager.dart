import 'package:flutter/services.dart';

/// Consent management for GDPR, CCPA, and other privacy regulations
class ConsentManager {
  static const MethodChannel _channel = MethodChannel('io.adproject/sdk');

  /// Current consent status
  static ConsentStatus? _currentStatus;

  /// Get current consent status
  static ConsentStatus? get currentStatus => _currentStatus;

  /// Set full consent status
  static Future<void> setConsentStatus(ConsentStatus status) async {
    await _channel.invokeMethod('setConsentStatus', status.toMap());
    _currentStatus = status;
  }

  /// Get consent status from native SDK
  static Future<ConsentStatus> getConsentStatus() async {
    final result = await _channel.invokeMapMethod<String, dynamic>(
      'getConsentStatus',
    );
    final status = ConsentStatus.fromMap(result!);
    _currentStatus = status;
    return status;
  }

  /// Set GDPR consent
  static Future<void> setHasUserConsent(bool hasConsent) async {
    await _channel.invokeMethod('setHasUserConsent', {'hasConsent': hasConsent});
    _currentStatus = _currentStatus?.copyWith(hasConsent: hasConsent) ??
        ConsentStatus(hasConsent: hasConsent);
  }

  /// Set COPPA age restriction
  static Future<void> setIsAgeRestrictedUser(bool isRestricted) async {
    await _channel.invokeMethod('setIsAgeRestrictedUser', {
      'isRestricted': isRestricted,
    });
    _currentStatus = _currentStatus?.copyWith(isAgeRestricted: isRestricted) ??
        ConsentStatus(isAgeRestricted: isRestricted);
  }

  /// Set CCPA do-not-sell
  static Future<void> setDoNotSell(bool doNotSell) async {
    await _channel.invokeMethod('setDoNotSell', {'doNotSell': doNotSell});
    _currentStatus = _currentStatus?.copyWith(doNotSell: doNotSell) ??
        ConsentStatus(doNotSell: doNotSell);
  }

  /// Set TCF consent string (IAB standard)
  static Future<void> setTCFConsentString(String consentString) async {
    await _channel.invokeMethod('setTCFConsentString', {
      'consentString': consentString,
    });
    _currentStatus = _currentStatus?.copyWith(tcfConsentString: consentString) ??
        ConsentStatus(tcfConsentString: consentString);
  }

  /// Set USP string (US Privacy)
  static Future<void> setUSPString(String uspString) async {
    await _channel.invokeMethod('setUSPString', {'uspString': uspString});
    _currentStatus = _currentStatus?.copyWith(uspString: uspString) ??
        ConsentStatus(uspString: uspString);
  }

  /// Set consent for specific vendor
  static Future<void> setVendorConsent(String vendorId, bool hasConsent) async {
    await _channel.invokeMethod('setVendorConsent', {
      'vendorId': vendorId,
      'hasConsent': hasConsent,
    });

    final vendors = Map<String, bool>.from(_currentStatus?.vendorConsents ?? {});
    vendors[vendorId] = hasConsent;
    _currentStatus = _currentStatus?.copyWith(vendorConsents: vendors) ??
        ConsentStatus(vendorConsents: vendors);
  }

  /// Clear all consent data
  static Future<void> clearConsent() async {
    await _channel.invokeMethod('clearConsent');
    _currentStatus = null;
  }
}

/// Full consent status
class ConsentStatus {
  final bool? hasConsent;
  final bool? isAgeRestricted;
  final bool? doNotSell;
  final String? tcfConsentString;
  final String? uspString;
  final Map<String, bool>? vendorConsents;

  const ConsentStatus({
    this.hasConsent,
    this.isAgeRestricted,
    this.doNotSell,
    this.tcfConsentString,
    this.uspString,
    this.vendorConsents,
  });

  factory ConsentStatus.fromMap(Map<String, dynamic> map) => ConsentStatus(
        hasConsent: map['hasConsent'] as bool?,
        isAgeRestricted: map['isAgeRestricted'] as bool?,
        doNotSell: map['doNotSell'] as bool?,
        tcfConsentString: map['tcfConsentString'] as String?,
        uspString: map['uspString'] as String?,
        vendorConsents: map['vendorConsents'] != null
            ? Map<String, bool>.from(map['vendorConsents'])
            : null,
      );

  Map<String, dynamic> toMap() => {
        if (hasConsent != null) 'hasConsent': hasConsent,
        if (isAgeRestricted != null) 'isAgeRestricted': isAgeRestricted,
        if (doNotSell != null) 'doNotSell': doNotSell,
        if (tcfConsentString != null) 'tcfConsentString': tcfConsentString,
        if (uspString != null) 'uspString': uspString,
        if (vendorConsents != null) 'vendorConsents': vendorConsents,
      };

  ConsentStatus copyWith({
    bool? hasConsent,
    bool? isAgeRestricted,
    bool? doNotSell,
    String? tcfConsentString,
    String? uspString,
    Map<String, bool>? vendorConsents,
  }) =>
      ConsentStatus(
        hasConsent: hasConsent ?? this.hasConsent,
        isAgeRestricted: isAgeRestricted ?? this.isAgeRestricted,
        doNotSell: doNotSell ?? this.doNotSell,
        tcfConsentString: tcfConsentString ?? this.tcfConsentString,
        uspString: uspString ?? this.uspString,
        vendorConsents: vendorConsents ?? this.vendorConsents,
      );

  /// Check if user has given consent
  bool get isConsentGiven => hasConsent == true;

  /// Check if user is under COPPA
  bool get isCoppaApplicable => isAgeRestricted == true;

  /// Check if user has opted out of sale (CCPA)
  bool get hasOptedOutOfSale => doNotSell == true;
}
