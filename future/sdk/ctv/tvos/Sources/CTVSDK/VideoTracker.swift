#if canImport(AVFoundation)
import Foundation
import AVFoundation

final class VideoTracker {
    enum Event: Hashable {
        case start, firstQuartile, midpoint, thirdQuartile, complete, pause, resume, mute, unmute, close
        var metricName: String {
            switch self {
            case .start: return "start"
            case .firstQuartile: return "first_quartile"
            case .midpoint: return "midpoint"
            case .thirdQuartile: return "third_quartile"
            case .complete: return "complete"
            case .pause: return "pause"
            case .resume: return "resume"
            case .mute: return "mute"
            case .unmute: return "unmute"
            case .close: return "close"
            }
        }
    }

    private let tracking: AuctionWin.Tracking
    private weak var player: AVPlayer?
    private var timeObserver: Any?
    private var timeControlObservation: NSKeyValueObservation?
    private var muteObservation: NSKeyValueObservation?
    private var firedEvents = Set<Event>()
    private var lastMuteState: Bool = false

    init(tracking: AuctionWin.Tracking) {
        self.tracking = tracking
    }

    func attach(to player: AVPlayer) {
        self.player = player
        lastMuteState = player.isMuted
        let interval = CMTime(seconds: 0.5, preferredTimescale: 600)
        timeObserver = player.addPeriodicTimeObserver(forInterval: interval, queue: .main) { [weak self] time in
            self?.handleProgress(currentTime: time)
        }
        timeControlObservation = player.observe(\AVPlayer.timeControlStatus, options: [.new]) { [weak self] _, change in
            guard let status = change.newValue else { return }
            self?.handleTimeControl(status)
        }
        muteObservation = player.observe(\AVPlayer.isMuted, options: [.new]) { [weak self] _, change in
            guard let muted = change.newValue else { return }
            self?.handleMuteChange(isMuted: muted)
        }
    }

    func markComplete() { emit(event: .complete, url: tracking.complete) }

    func markClose() { emit(event: .close, url: tracking.close) }

    func stop() {
        if let observer = timeObserver, let player = player {
            player.removeTimeObserver(observer)
        }
        timeObserver = nil
        timeControlObservation?.invalidate()
        muteObservation?.invalidate()
        timeControlObservation = nil
        muteObservation = nil
        player = nil
    }

    private func handleProgress(currentTime: CMTime) {
        guard let item = player?.currentItem else { return }
        let durationSeconds = CMTimeGetSeconds(item.duration)
        let currentSeconds = CMTimeGetSeconds(currentTime)
        guard durationSeconds.isFinite && durationSeconds > 0 && currentSeconds.isFinite else { return }
        let startThreshold = min(2.0, durationSeconds * 0.05)
        if currentSeconds >= startThreshold { emit(event: .start, url: tracking.start) }
        let percent = currentSeconds / durationSeconds
        if percent >= 0.25 { emit(event: .firstQuartile, url: tracking.firstQuartile) }
        if percent >= 0.50 { emit(event: .midpoint, url: tracking.midpoint) }
        if percent >= 0.75 { emit(event: .thirdQuartile, url: tracking.thirdQuartile) }
    }

    private func handleTimeControl(_ status: AVPlayer.TimeControlStatus) {
        switch status {
        case .playing:
            emit(event: .resume, url: tracking.resume)
        case .paused:
            emit(event: .pause, url: tracking.pause)
        default:
            break
        }
    }

    private func handleMuteChange(isMuted: Bool) {
        guard isMuted != lastMuteState else { return }
        lastMuteState = isMuted
        emit(event: isMuted ? .mute : .unmute, url: isMuted ? tracking.mute : tracking.unmute)
    }

    private func emit(event: Event, url: String?) {
        guard !firedEvents.contains(event) else { return }
        firedEvents.insert(event)
        MetricsRecorder.shared.recordPlayback(eventName: event.metricName)
        guard let url = url else { return }
        Beacon.fire(url, eventName: event.metricName)
    }
}
#else
import Foundation

final class VideoTracker {
    init(tracking: AuctionWin.Tracking) {}
    func attach(to player: AnyObject) {}
    func markComplete() {}
    func markClose() {}
    func stop() {}
}
#endif
