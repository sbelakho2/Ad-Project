import Foundation
#if canImport(FoundationNetworking)
import FoundationNetworking
#endif

enum Beacon {
    static func fire(_ urlString: String?, eventName: String? = nil) {
        guard let urlString = urlString, let url = URL(string: urlString) else { return }
        var request = URLRequest(url: url)
        request.httpMethod = "GET"
        URLSession.shared.dataTask(with: request) { _, response, error in
            guard let name = eventName else { return }
            if error != nil {
                MetricsRecorder.shared.recordTracker(eventName: name, success: false)
                return
            }
            if let http = response as? HTTPURLResponse {
                let ok = (200..<400).contains(http.statusCode)
                MetricsRecorder.shared.recordTracker(eventName: name, success: ok)
            } else {
                MetricsRecorder.shared.recordTracker(eventName: name, success: false)
            }
        }.resume()
    }
}
