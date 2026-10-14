import Foundation

@available(iOS 13.0, tvOS 13.0, macOS 10.15, *)
extension URLSession {
    /// Provides async data loading that works on older Apple platforms by falling back to completion handlers.
    func apexData(for request: URLRequest) async throws -> (Data, URLResponse) {
        if #available(iOS 15.0, tvOS 15.0, macOS 12.0, *) {
            return try await data(for: request, delegate: nil)
        }

        return try await withCheckedThrowingContinuation { continuation in
            let task = dataTask(with: request) { data, response, error in
                if let error = error {
                    continuation.resume(throwing: error)
                    return
                }

                guard let data = data, let response = response else {
                    continuation.resume(throwing: URLError(.badServerResponse))
                    return
                }

                continuation.resume(returning: (data, response))
            }

            task.resume()
        }
    }
}
