#if canImport(UIKit)
@preconcurrency import Foundation
@preconcurrency import UIKit

/// Coordinates ad presentations to guarantee main-thread execution and prevent overlapping shows.
@available(iOSApplicationExtension, unavailable)
@MainActor
final class AdPresentationCoordinator {
    struct Token: Equatable {
        let id = UUID()
        let placementId: String
    }

    static let shared = AdPresentationCoordinator()

    private var activeToken: Token?
    private var pendingWorkItem: DispatchWorkItem?
    private var observers: [NSObjectProtocol] = []

    private init() {
        let center = NotificationCenter.default
        observers.append(center.addObserver(forName: UIApplication.didEnterBackgroundNotification, object: nil, queue: nil) { [weak self] _ in
            Task { @MainActor in
                self?.cancelActivePresentation()
            }
        })
    }

    deinit {
        observers.forEach { NotificationCenter.default.removeObserver($0) }
    }

    /// Attempts to reserve the presentation slot for the placement.
    /// Throws when another ad is already mid-presentation.
    func beginPresentation(placementId: String) throws -> Token {
        if activeToken != nil {
            throw SDKError.presentationInProgress
        }
        let token = Token(placementId: placementId)
        activeToken = token
        return token
    }

    /// Releases the presentation slot and clears any scheduled callbacks.
    func finishPresentation(_ token: Token) {
        guard activeToken == token else { return }
        activeToken = nil
        pendingWorkItem?.cancel()
        pendingWorkItem = nil
    }

    /// Schedules a main-thread callback that is automatically cancelled if the app backgrounds.
    func schedule(after delay: TimeInterval, perform block: @escaping () -> Void) {
        pendingWorkItem?.cancel()
        let workItem = DispatchWorkItem(block: block)
        pendingWorkItem = workItem
        DispatchQueue.main.asyncAfter(deadline: .now() + delay, execute: workItem)
    }

    private func cancelActivePresentation() {
        activeToken = nil
        pendingWorkItem?.cancel()
        pendingWorkItem = nil
    }

    #if DEBUG
    /// Test-only helper to reset state between XCTest cases.
    func resetForTests() {
        cancelActivePresentation()
    }
    #endif
}
#endif
