/// Ad types and common models for the AdProject SDK
library adproject_sdk.ad_types;

/// Types of ads supported by the SDK
enum AdType {
  banner,
  interstitial,
  rewarded,
  rewardedInterstitial,
  native,
  appOpen,
}

/// Standard banner ad sizes
class AdSize {
  final int width;
  final int height;
  final bool isAdaptive;

  const AdSize({
    required this.width,
    required this.height,
    this.isAdaptive = false,
  });

  /// Standard banner (320x50)
  static const banner = AdSize(width: 320, height: 50);

  /// Large banner (320x100)
  static const largeBanner = AdSize(width: 320, height: 100);

  /// Medium rectangle (300x250)
  static const mediumRectangle = AdSize(width: 300, height: 250);

  /// Full banner (468x60)
  static const fullBanner = AdSize(width: 468, height: 60);

  /// Leaderboard (728x90)
  static const leaderboard = AdSize(width: 728, height: 90);

  /// Create an adaptive banner size for the given width
  static AdSize adaptive(int width) => AdSize(
        width: width,
        height: 0, // Calculated natively
        isAdaptive: true,
      );

  /// Create a custom size
  static AdSize custom(int width, int height) => AdSize(
        width: width,
        height: height,
      );

  Map<String, dynamic> toMap() => {
        'width': width,
        'height': height,
        'isAdaptive': isAdaptive,
      };
}

/// Result of an ad load operation
class AdLoadResult {
  final bool success;
  final String adUnitId;
  final String? networkName;
  final AdError? error;
  final int? loadTimeMs;

  const AdLoadResult({
    required this.success,
    required this.adUnitId,
    this.networkName,
    this.error,
    this.loadTimeMs,
  });

  factory AdLoadResult.fromMap(Map<String, dynamic> map) => AdLoadResult(
        success: map['success'] as bool,
        adUnitId: map['adUnitId'] as String,
        networkName: map['networkName'] as String?,
        error: map['error'] != null
            ? AdError.fromMap(map['error'] as Map<String, dynamic>)
            : null,
        loadTimeMs: map['loadTimeMs'] as int?,
      );
}

/// Ad loading or display error
class AdError {
  final String code;
  final String message;
  final String? networkName;

  const AdError({
    required this.code,
    required this.message,
    this.networkName,
  });

  factory AdError.fromMap(Map<String, dynamic> map) => AdError(
        code: map['code'] as String,
        message: map['message'] as String,
        networkName: map['networkName'] as String?,
      );

  @override
  String toString() => 'AdError($code): $message';
}

/// Reward earned from a rewarded ad
class Reward {
  final String type;
  final int amount;

  const Reward({
    required this.type,
    required this.amount,
  });

  factory Reward.fromMap(Map<String, dynamic> map) => Reward(
        type: map['type'] as String,
        amount: map['amount'] as int,
      );
}

/// Impression data for revenue tracking
class ImpressionData {
  final String adUnitId;
  final String networkName;
  final double revenue;
  final String revenuePrecision;
  final AdType adType;
  final String? placement;

  const ImpressionData({
    required this.adUnitId,
    required this.networkName,
    required this.revenue,
    required this.revenuePrecision,
    required this.adType,
    this.placement,
  });

  factory ImpressionData.fromMap(Map<String, dynamic> map) => ImpressionData(
        adUnitId: map['adUnitId'] as String,
        networkName: map['networkName'] as String,
        revenue: (map['revenue'] as num).toDouble(),
        revenuePrecision: map['revenuePrecision'] as String,
        adType: AdType.values.firstWhere(
          (e) => e.name == map['adType'],
          orElse: () => AdType.banner,
        ),
        placement: map['placement'] as String?,
      );
}

/// SDK initialization status
class InitializationStatus {
  final bool isInitialized;
  final List<AdapterStatus> adapters;

  const InitializationStatus({
    required this.isInitialized,
    required this.adapters,
  });

  factory InitializationStatus.fromMap(Map<String, dynamic> map) =>
      InitializationStatus(
        isInitialized: map['isInitialized'] as bool,
        adapters: (map['adapters'] as List<dynamic>)
            .map((e) => AdapterStatus.fromMap(e as Map<String, dynamic>))
            .toList(),
      );
}

/// Individual adapter initialization status
class AdapterStatus {
  final String networkName;
  final AdapterInitStatus status;
  final String? sdkVersion;
  final String? adapterVersion;
  final String? errorMessage;

  const AdapterStatus({
    required this.networkName,
    required this.status,
    this.sdkVersion,
    this.adapterVersion,
    this.errorMessage,
  });

  factory AdapterStatus.fromMap(Map<String, dynamic> map) => AdapterStatus(
        networkName: map['networkName'] as String,
        status: AdapterInitStatus.values.firstWhere(
          (e) => e.name == map['status'],
          orElse: () => AdapterInitStatus.notInitialized,
        ),
        sdkVersion: map['sdkVersion'] as String?,
        adapterVersion: map['adapterVersion'] as String?,
        errorMessage: map['errorMessage'] as String?,
      );

  bool get isReady =>
      status == AdapterInitStatus.initializedSuccess ||
      status == AdapterInitStatus.doesNotApply;
}

/// Adapter initialization status enum
enum AdapterInitStatus {
  notInitialized,
  initializing,
  initializedSuccess,
  initializedFailure,
  initializedUnknown,
  doesNotApply,
}
