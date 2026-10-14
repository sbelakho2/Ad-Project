import Foundation

public enum LoadError: Equatable {
    case noFill
    case timeout
    case network
    case status(code: Int, label: String)
    case generic(_ reason: String)

    public var reason: String {
        switch self {
        case .noFill: return "no_fill"
        case .timeout: return "timeout"
        case .network: return "network_error"
        case let .status(_, label): return label
        case let .generic(reason): return reason
        }
    }
}
