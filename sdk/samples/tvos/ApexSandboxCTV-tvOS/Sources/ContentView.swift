import SwiftUI

struct ContentView: View {
    @Environment(\.scenePhase) private var scenePhase
    @StateObject private var vm = SandboxViewModel()
    @FocusState private var focusedButton: FocusTarget?

    enum FocusTarget: Hashable { case initBtn, showInt, showRwd }

    var body: some View {
        VStack(spacing: 24) {
            Text("Apex Sandbox CTV (tvOS)")
                .font(.largeTitle)
            if let err = vm.lastError, !err.isEmpty {
                Text(err).font(.footnote).foregroundStyle(.red)
            }
            HStack(spacing: 18) {
                Button(vm.initialized ? "Initialized" : "Initialize") {
                    vm.initializeSDK()
                }
                .buttonStyle(.borderedProminent)
                .disabled(vm.initialized)
                .focusable(true)
                .focused($focusedButton, equals: .initBtn)

                Button("Show Interstitial") {
                    vm.showInterstitial()
                }
                .buttonStyle(.bordered)
                .disabled(!(vm.initialized) || vm.isPresentingAd)
                .focusable(true)
                .focused($focusedButton, equals: .showInt)

                Button("Show Rewarded") {
                    vm.showRewarded()
                }
                .buttonStyle(.bordered)
                .disabled(!(vm.initialized) || vm.isPresentingAd)
                .focusable(true)
                .focused($focusedButton, equals: .showRwd)
            }

            // Simple status console
            VStack(alignment: .leading, spacing: 6) {
                Text("Status: \(vm.initialized ? "Initialized" : "Not initialized")  presenting=\(vm.isPresentingAd ? "yes" : "no")")
                    .font(.subheadline)
                    .foregroundStyle(vm.initialized ? .green : .secondary)
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 2) {
                        ForEach(Array(vm.logLines.suffix(30).enumerated()), id: \.offset) { _, line in
                            Text(line).font(.system(size: 12, design: .monospaced)).frame(maxWidth: .infinity, alignment: .leading)
                        }
                    }
                }
                .frame(minHeight: 120, maxHeight: 200)
            }
            .frame(maxWidth: 840)
        }
        .padding(32)
        .onAppear { if !vm.initialized { focusedButton = .initBtn } }
        .onChange(of: scenePhase) { _, phase in
            switch phase {
            case .background: vm.onBackground()
            case .active: vm.onForeground()
            case .inactive: break
            @unknown default: break
            }
        }
    }
}
