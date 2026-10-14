import Foundation

final class AdCache {
    static let shared = AdCache()

    private struct Entry {
        let win: AuctionWin
        let expiry: TimeInterval // monotonic uptime seconds
    }

    private var storage: [String: Entry] = [:]
    private let lock = NSLock()
    private let clock: ClockProtocol

    init(clock: ClockProtocol = Clock.shared) {
        self.clock = clock
    }

    func store(win: AuctionWin, for placementId: String) {
        lock.lock(); defer { lock.unlock() }
        let ttl = max(0, win.ttlSeconds)
        let expiry = clock.monotonicNow() + TimeInterval(ttl)
        storage[placementId] = Entry(win: win, expiry: expiry)
    }

    func peek(placementId: String) -> AuctionWin? {
        lock.lock(); defer { lock.unlock() }
        guard let entry = storage[placementId] else { return nil }
        guard entry.expiry > clock.monotonicNow() else {
            storage.removeValue(forKey: placementId)
            return nil
        }
        return entry.win
    }

    func take(placementId: String) -> AuctionWin? {
        lock.lock(); defer { lock.unlock() }
        guard let entry = storage[placementId] else { return nil }
        guard entry.expiry > clock.monotonicNow() else {
            storage.removeValue(forKey: placementId)
            return nil
        }
        storage.removeValue(forKey: placementId)
        return entry.win
    }
}
