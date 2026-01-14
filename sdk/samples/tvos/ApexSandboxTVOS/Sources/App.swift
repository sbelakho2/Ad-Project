import SwiftUI

@main
struct ApexSandboxTVOSApp: App {
    var body: some Scene {
        WindowGroup {
#if targetEnvironment(simulator)
            ContentView()
#else
            Text("Simulator Only\nThis app may only run in the Apple TV Simulator.")
                .multilineTextAlignment(.center)
                .padding()
                .onAppear {
                    exit(0)
                }
#endif
        }
    }
}

struct ContentView: View {
    @State private var status: String = "SDK: not initialized"

    var body: some View {
        VStack(spacing: 20) {
            Text("Apex Sandbox tvOS").font(.title)
            Text(status)
            Button("Initialize") { status = "SDK: initialized (stub)" }
            Button("Load Interstitial") { status = "Interstitial: loaded (stub)" }
            Button("Show Interstitial") { status = "Interstitial: shown (stub)" }
            Button("Load Rewarded") { status = "Rewarded: loaded (stub)" }
            Button("Show Rewarded") { status = "Rewarded: shown (stub)" }
        }
        .padding()
    }
}