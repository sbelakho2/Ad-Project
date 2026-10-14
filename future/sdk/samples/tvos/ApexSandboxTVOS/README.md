# ApexSandboxTVOS

tvOS sandbox app (Apple TV Simulator only) for exercising CTV SDK flows.

Requirements
- Xcode 16+
- XcodeGen (`brew install xcodegen`)

Quick start
1. `cd Test Apps/tvos/ApexSandboxTVOS`
2. `xcodegen generate`
3. Open `ApexSandboxTVOS.xcodeproj` in Xcode.
4. Select an Apple TV Simulator and Run.

Notes
- The app terminates immediately if built for a physical Apple TV device; it is Simulator-only (see `#if targetEnvironment(simulator)`).
- Configure staging endpoints and placement IDs in `Sources/SandboxConfig.json`.
