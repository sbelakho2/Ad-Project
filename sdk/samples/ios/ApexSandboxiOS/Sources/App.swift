import SwiftUI
#if canImport(FirebaseCore)
import FirebaseCore
#endif

@main
struct ApexSandboxiOSApp: App {
    init() {
        // Initialize Crashlytics only if Firebase config is present in the bundle
        #if canImport(FirebaseCore)
        if Bundle.main.path(forResource: "GoogleService-Info", ofType: "plist") != nil {
            FirebaseApp.configure()
        }
        #endif
    }
    var body: some Scene {
        WindowGroup {
#if targetEnvironment(simulator)
            ContentView()
#else
            Text("Simulator Only\nThis app may only run in the iOS Simulator.")
                .multilineTextAlignment(.center)
                .padding()
                .onAppear {
                    // Immediately exit on physical devices
                    exit(0)
                }
#endif
        }
    }
}
