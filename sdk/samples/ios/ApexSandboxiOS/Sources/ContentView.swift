import SwiftUI
import RivalApexMediationSDK

struct ContentView: View {
    @StateObject private var vm = SandboxViewModel()
    @State private var showingLogs = false

    var body: some View {
        NavigationView {
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    header

                    GroupBox(label: Text("Adapters")) {
                        VStack(alignment: .leading, spacing: 8) {
                            HStack {
                                Picker("Adapter", selection: $vm.selectedAdapter) {
                                    ForEach(vm.adapterNamesList, id: \.self) { name in
                                        Text(name).tag(Optional(name))
                                    }
                                }
                                .pickerStyle(.menu)
                                .frame(maxWidth: 240)
                                Button("Refresh list") { vm.refreshAdapters() }
                                    .buttonStyle(.bordered)
                                    .disabled(!vm.isInitialized)
                            }
                            HStack(spacing: 12) {
                                Button("Run Selected (Interstitial)") { vm.runSelectedAdapterInterstitial() }
                                    .buttonStyle(.bordered)
                                    .disabled(!vm.isInitialized)
                                Button("Run All") { vm.runAllAdapters() }
                                    .buttonStyle(.borderedProminent)
                                    .disabled(!vm.isInitialized)
                            }
                        }
                        .padding(.top, 4)
                    }

                    GroupBox(label: Text("Lifecycle")) {
                        VStack(alignment: .leading, spacing: 8) {
                            HStack {
                                Button(action: vm.initializeSDK) {
                                    Label(vm.isInitialized ? "Initialized" : "Initialize",
                                          systemImage: vm.isInitialized ? "checkmark.seal.fill" : "play.circle")
                                }
                                .buttonStyle(.borderedProminent)
                                // Idempotent: keep enabled to demonstrate repeated safe calls
                                .disabled(false)

                                Button(action: { Task { await vm.requestATT() } }) {
                                    Label("Request ATT", systemImage: "hand.raised")
                                }
                                .buttonStyle(.bordered)
                            }

                            Toggle("Test mode", isOn: $vm.testMode)
                                .onChange(of: vm.testMode) { _ in
                                    if vm.isInitialized { vm.initializeSDK() }
                                }

                            // Error handling helpers
                            Toggle("Use invalid placements", isOn: $vm.useInvalidPlacements)
                            Toggle("Simulate timeout (debug)", isOn: $vm.simulateTimeout)

                            // Soak controls
                            HStack(spacing: 12) {
                                Button(vm.isSoakRunning ? "Soak Running…" : "Start Soak (30m)") {
                                    vm.startSoakRun(minutes: 30)
                                }
                                .buttonStyle(.bordered)
                                .disabled(vm.isSoakRunning == true)

                                Button("Stop Soak") { vm.stopSoakRun() }
                                    .buttonStyle(.bordered)
                                    .disabled(vm.isSoakRunning == false)
                            }
                        }
                        .padding(.top, 4)
                    }

                    GroupBox(label: Text("Consent")) {
                        VStack(alignment: .leading, spacing: 8) {
                            Toggle("GDPR applies", isOn: $vm.consentGDPR).onChange(of: vm.consentGDPR) { _ in vm.applyConsent() }
                            Toggle("CCPA opt-out", isOn: $vm.consentCCPA).onChange(of: vm.consentCCPA) { _ in vm.applyConsent() }
                            Toggle("COPPA", isOn: $vm.consentCOPPA).onChange(of: vm.consentCOPPA) { _ in vm.applyConsent() }
                            Text("Summary: \(vm.consentSummaryPretty())")
                                .font(.footnote).foregroundColor(.secondary)
                        }
                        .padding(.top, 4)
                    }

                    GroupBox(label: Text("Interstitial")) {
                        HStack(spacing: 12) {
                            Button("Load") { vm.loadInterstitial() }
                                .buttonStyle(.bordered)
                                .disabled(!vm.isInitialized)
                            Button("Show") { vm.showInterstitial() }
                                .buttonStyle(.borderedProminent)
                                .disabled(!(vm.isInitialized && vm.interstitialLoaded) || vm.isPresentingAd)
                            Spacer()
                            statusDot(loaded: vm.interstitialLoaded)
                        }
                        .padding(.top, 4)
                    }

                    GroupBox(label: Text("Rewarded")) {
                        HStack(spacing: 12) {
                            Button("Load") { vm.loadRewarded() }
                                .buttonStyle(.bordered)
                                .disabled(!vm.isInitialized)
                            Button("Show") { vm.showRewarded() }
                                .buttonStyle(.borderedProminent)
                                .disabled(!(vm.isInitialized && vm.rewardedLoaded) || vm.isPresentingAd)
                            Spacer()
                            statusDot(loaded: vm.rewardedLoaded)
                        }
                        .padding(.top, 4)
                    }

                    GroupBox(label: Text("Debug")) {
                        VStack(alignment: .leading, spacing: 8) {
                            HStack {
                                Text("Config ver: \(vm.configVersionText)")
                                Spacer()
                                Button(showingLogs ? "Hide Logs" : "Show Logs") { showingLogs.toggle() }
                                    .font(.footnote)
                            }
                            HStack(spacing: 12) {
                                Text("ATT: \(vm.attStatusText)")
                                    .font(.footnote)
                                    .foregroundColor(.secondary)
                                Text("Last req ID: \(vm.lastRequestId)")
                                    .font(.footnote)
                                    .foregroundColor(.secondary)
                                if vm.isSoakRunning {
                                    Text("Soak: running")
                                        .font(.footnote)
                                        .foregroundColor(.blue)
                                }
                            }
                            if let err = vm.lastError, !err.isEmpty {
                                Text("Last error: \(err)")
                                    .font(.footnote)
                                    .foregroundColor(.red)
                            }
                            if showingLogs {
                                ScrollView {
                                    VStack(alignment: .leading, spacing: 4) {
                                        ForEach(Array(vm.logs.enumerated()), id: \.offset) { _, line in
                                            Text(line).font(.system(size: 12, design: .monospaced))
                                        }
                                    }
                                    .frame(maxWidth: .infinity, alignment: .leading)
                                }
                                .frame(minHeight: 120, maxHeight: 200)
                                .background(Color(UIColor.secondarySystemBackground))
                                .cornerRadius(8)
                            }
                        }
                        .padding(.top, 4)
                    }

                    Spacer(minLength: 20)
                }
                .padding()
            }
            .navigationTitle("Apex Sandbox iOS")
        }
        .navigationViewStyle(.stack)
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("Mediation SDK v\(MediationSDK.shared.sdkVersion)")
                .font(.headline)
            Text(vm.isInitialized ? "Initialized" : "Not initialized")
                .font(.subheadline)
                .foregroundColor(vm.isInitialized ? .green : .secondary)
        }
    }

    private func statusDot(loaded: Bool) -> some View {
        Circle()
            .fill(loaded ? Color.green : Color.red)
            .frame(width: 12, height: 12)
            .overlay(Circle().stroke(Color.secondary, lineWidth: 0.5))
            .accessibilityLabel(loaded ? "Loaded" : "Not loaded")
    }
}
