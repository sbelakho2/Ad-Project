Changelog — @rivalapex/web-sdk

Unreleased
- Add consent parity with mobile/CTV: request payload includes GDPR/TCF, CCPA/USP, GPP (if provided), and COPPA flags.
- Add Typedoc generation and CI artifact upload (web-sdk-typedoc).
- Tests: Jest + MSW cover consent propagation, success, and timeout scenarios.

0.1.0 — 2025-11-14
- Initial release of the Web SDK with public API: init, setConsent, requestAd, on(event).
- Rollup builds to ESM and UMD; type declarations emitted.