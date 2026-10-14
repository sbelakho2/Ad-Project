// Tier-1 Dart tests for the Flutter SDK: pure models and the method-channel initialize path.
// Channels are mocked with the standard test binding so no platform host is required.
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:adproject_sdk/adproject_sdk.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  const channel = MethodChannel('io.adproject/sdk');
  final calls = <MethodCall>[];

  setUp(() {
    calls.clear();
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(channel, (call) async {
      calls.add(call);
      switch (call.method) {
        case 'initialize':
          return <String, dynamic>{'isInitialized': true, 'adapters': <dynamic>[]};
        case 'getInitializationStatus':
          return <String, dynamic>{'isInitialized': true, 'adapters': <dynamic>[]};
        default:
          return null;
      }
    });
    // The SDK singleton subscribes to the event channel at construction; keep it silent.
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockStreamHandler(
      const EventChannel('io.adproject/sdk_events'),
      MockStreamHandler.inline(onListen: (arguments, events) {}),
    );
  });

  tearDown(() {
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(channel, null);
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockStreamHandler(const EventChannel('io.adproject/sdk_events'), null);
  });

  test('AdSize helpers produce documented shapes', () {
    expect(AdSize.banner.width, 320);
    expect(AdSize.banner.height, 50);
    expect(AdSize.adaptive(375).isAdaptive, isTrue);
    expect(AdSize.adaptive(375).height, 0);
    expect(AdSize.custom(300, 250).toMap(),
        {'width': 300, 'height': 250, 'isAdaptive': false});
  });

  test('AdType enum covers every supported format', () {
    expect(AdType.values, containsAll(<AdType>[
      AdType.banner,
      AdType.interstitial,
      AdType.rewarded,
      AdType.rewardedInterstitial,
      AdType.native,
      AdType.appOpen,
    ]));
  });

  test('initialize() calls the native channel once and flips isInitialized', () async {
    final sdk = AdProjectSdk.instance;
    final status = await sdk.initialize('app-1', testMode: true, verbose: false);
    expect(status.isInitialized, isTrue);
    expect(sdk.isInitialized, isTrue);
    expect(calls.where((c) => c.method == 'initialize'), hasLength(1));
    expect(calls.first.arguments['appId'], 'app-1');
    expect(calls.first.arguments['testMode'], isTrue);

    // Second initialize() must short-circuit to the status query.
    await sdk.initialize('app-1');
    expect(calls.where((c) => c.method == 'initialize'), hasLength(1));
    expect(calls.where((c) => c.method == 'getInitializationStatus'), hasLength(1));
  });
}
