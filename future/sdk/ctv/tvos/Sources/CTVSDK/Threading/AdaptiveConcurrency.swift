import Foundation
import os.log

/// AdaptiveConcurrency - Adaptive dispatch queue and operation queue sizing for tvOS.
///
/// Optimized for Apple TV's hardware characteristics:
/// - Single-socket CPU with fewer cores than mobile devices
/// - Consistent performance without thermal throttling concerns
/// - Larger memory budget for buffering
@available(tvOS 13.0, macOS 10.15, *)
public final class AdaptiveConcurrency {
    
    /// Shared singleton instance
    public static let shared = AdaptiveConcurrency()
    
    // MARK: - Configuration Constants
    
    private static let minConcurrentOperations = 2
    private static let maxBackgroundOperations = 4
    private static let maxNetworkOperations = 8
    private static let maxComputeOperations = 2
    
    // MARK: - CPU Information
    
    /// Detected CPU information for the Apple TV.
    public struct CpuInfo {
        public let totalCores: Int
        public let performanceCores: Int
        public let efficiencyCores: Int
        public let isHeterogeneous: Bool
        public let deviceModel: String
    }
    
    /// Pool configuration for different workload types.
    public struct PoolConfig {
        public let maxConcurrentOperations: Int
        public let qualityOfService: QualityOfService
    }
    
    // MARK: - Properties
    
    /// Detected CPU information
    public let cpuInfo: CpuInfo
    
    /// Background operation queue for general background work
    public let backgroundQueue: OperationQueue
    
    /// Network operation queue for I/O-bound operations
    public let networkQueue: OperationQueue
    
    /// Compute operation queue for CPU-intensive work
    public let computeQueue: OperationQueue
    
    /// Media processing queue for video decoding/encoding
    public let mediaQueue: OperationQueue
    
    /// Dispatch queue for background work
    public let backgroundDispatchQueue: DispatchQueue
    
    /// Dispatch queue for network I/O
    public let networkDispatchQueue: DispatchQueue
    
    /// Dispatch queue for compute-intensive work
    public let computeDispatchQueue: DispatchQueue
    
    private var logger: Any?
    
    // MARK: - Metrics
    
    private let metricsLock = NSLock()
    private var taskCount: Int = 0
    private var completedTaskCount: Int = 0
    
    // MARK: - Initialization
    
    private init() {
        if #available(tvOS 14.0, macOS 11.0, *) {
            self.logger = Logger(subsystem: "com.rivalapexmediation.ctv", category: "AdaptiveConcurrency")
        } else {
            self.logger = nil
        }
        
        // Detect CPU info first - used for config calculations
        let detectedCpuInfo = AdaptiveConcurrency.detectCpuInfo()
        self.cpuInfo = detectedCpuInfo
        let cores = detectedCpuInfo.totalCores
        
        // Create background queue - conservative for tvOS
        // Inline config: max(min, min(cores-1, maxBackground))
        let backgroundMaxOps = max(Self.minConcurrentOperations, min(cores - 1, Self.maxBackgroundOperations))
        self.backgroundQueue = OperationQueue()
        backgroundQueue.name = "com.rivalapexmediation.ctv.background"
        backgroundQueue.maxConcurrentOperationCount = backgroundMaxOps
        backgroundQueue.qualityOfService = .utility
        
        // Create network queue - can be more aggressive
        // Inline config: max(min, min(cores*2, maxNetwork))
        let networkMaxOps = max(Self.minConcurrentOperations, min(cores * 2, Self.maxNetworkOperations))
        self.networkQueue = OperationQueue()
        networkQueue.name = "com.rivalapexmediation.ctv.network"
        networkQueue.maxConcurrentOperationCount = networkMaxOps
        networkQueue.qualityOfService = .userInitiated
        
        // Create compute queue - limited on tvOS
        // Inline config: max(1, min(perfCores, maxCompute))
        let computeCores = detectedCpuInfo.isHeterogeneous ? detectedCpuInfo.performanceCores : detectedCpuInfo.totalCores
        let computeMaxOps = max(1, min(computeCores, Self.maxComputeOperations))
        self.computeQueue = OperationQueue()
        computeQueue.name = "com.rivalapexmediation.ctv.compute"
        computeQueue.maxConcurrentOperationCount = computeMaxOps
        computeQueue.qualityOfService = .userInteractive
        
        // Create media queue - optimized for video processing
        // Hardware decoding handles most work, so limit to 2 ops
        self.mediaQueue = OperationQueue()
        mediaQueue.name = "com.rivalapexmediation.ctv.media"
        mediaQueue.maxConcurrentOperationCount = 2
        mediaQueue.qualityOfService = .userInteractive
        
        // Create dispatch queues
        self.backgroundDispatchQueue = DispatchQueue(
            label: "com.rivalapexmediation.ctv.background.dispatch",
            qos: .utility,
            attributes: .concurrent
        )
        
        self.networkDispatchQueue = DispatchQueue(
            label: "com.rivalapexmediation.ctv.network.dispatch",
            qos: .userInitiated,
            attributes: .concurrent
        )
        
        self.computeDispatchQueue = DispatchQueue(
            label: "com.rivalapexmediation.ctv.compute.dispatch",
            qos: .userInteractive,
            attributes: .concurrent
        )
        
        if #available(tvOS 14.0, macOS 11.0, *) {
            if let log = logger as? Logger {
                log.info("CTV AdaptiveConcurrency initialized: \(self.cpuInfo.totalCores) cores, model: \(self.cpuInfo.deviceModel)")
            }
        }
    }
    
    // MARK: - Pool Configuration
    
    /// Gets the recommended pool configuration for background work.
    public func getBackgroundPoolConfig() -> PoolConfig {
        let cores = cpuInfo.totalCores
        let maxOps = max(Self.minConcurrentOperations, min(cores - 1, Self.maxBackgroundOperations))
        return PoolConfig(
            maxConcurrentOperations: maxOps,
            qualityOfService: .utility
        )
    }
    
    /// Gets the recommended pool configuration for network I/O.
    public func getNetworkPoolConfig() -> PoolConfig {
        let cores = cpuInfo.totalCores
        let maxOps = max(Self.minConcurrentOperations, min(cores * 2, Self.maxNetworkOperations))
        return PoolConfig(
            maxConcurrentOperations: maxOps,
            qualityOfService: .userInitiated
        )
    }
    
    /// Gets the recommended pool configuration for compute-intensive work.
    public func getComputePoolConfig() -> PoolConfig {
        let cores = cpuInfo.isHeterogeneous ? cpuInfo.performanceCores : cpuInfo.totalCores
        let maxOps = max(1, min(cores, Self.maxComputeOperations))
        return PoolConfig(
            maxConcurrentOperations: maxOps,
            qualityOfService: .userInteractive
        )
    }
    
    /// Gets the recommended pool configuration for media processing.
    public func getMediaPoolConfig() -> PoolConfig {
        // Media processing on tvOS should be minimal - hardware decoding handles most of it
        return PoolConfig(
            maxConcurrentOperations: 2,
            qualityOfService: .userInteractive
        )
    }
    
    // MARK: - Task Execution
    
    /// Executes a background task using the background queue.
    public func executeBackground(_ work: @escaping () -> Void) {
        incrementTaskCount()
        backgroundQueue.addOperation {
            work()
            self.incrementCompletedCount()
        }
    }
    
    /// Executes a network I/O task using the network queue.
    public func executeNetwork(_ work: @escaping () -> Void) {
        incrementTaskCount()
        networkQueue.addOperation {
            work()
            self.incrementCompletedCount()
        }
    }
    
    /// Executes a compute-intensive task using the compute queue.
    public func executeCompute(_ work: @escaping () -> Void) {
        incrementTaskCount()
        computeQueue.addOperation {
            work()
            self.incrementCompletedCount()
        }
    }
    
    /// Executes a media processing task using the media queue.
    public func executeMedia(_ work: @escaping () -> Void) {
        incrementTaskCount()
        mediaQueue.addOperation {
            work()
            self.incrementCompletedCount()
        }
    }
    
    /// Executes a background task on the dispatch queue.
    public func dispatchBackground(_ work: @escaping () -> Void) {
        incrementTaskCount()
        backgroundDispatchQueue.async {
            work()
            self.incrementCompletedCount()
        }
    }
    
    /// Executes a network I/O task on the dispatch queue.
    public func dispatchNetwork(_ work: @escaping () -> Void) {
        incrementTaskCount()
        networkDispatchQueue.async {
            work()
            self.incrementCompletedCount()
        }
    }
    
    /// Executes a compute-intensive task on the dispatch queue.
    public func dispatchCompute(_ work: @escaping () -> Void) {
        incrementTaskCount()
        computeDispatchQueue.async {
            work()
            self.incrementCompletedCount()
        }
    }
    
    // MARK: - Async/Await Support
    
    /// Executes a background task asynchronously.
    public func executeBackgroundAsync<T>(_ work: @escaping () throws -> T) async throws -> T {
        incrementTaskCount()
        return try await withCheckedThrowingContinuation { continuation in
            backgroundQueue.addOperation {
                do {
                    let result = try work()
                    self.incrementCompletedCount()
                    continuation.resume(returning: result)
                } catch {
                    self.incrementCompletedCount()
                    continuation.resume(throwing: error)
                }
            }
        }
    }
    
    /// Executes a network task asynchronously.
    public func executeNetworkAsync<T>(_ work: @escaping () throws -> T) async throws -> T {
        incrementTaskCount()
        return try await withCheckedThrowingContinuation { continuation in
            networkQueue.addOperation {
                do {
                    let result = try work()
                    self.incrementCompletedCount()
                    continuation.resume(returning: result)
                } catch {
                    self.incrementCompletedCount()
                    continuation.resume(throwing: error)
                }
            }
        }
    }
    
    // MARK: - Statistics
    
    /// Gets current pool statistics.
    public func getStats() -> [String: Any] {
        metricsLock.lock()
        let tasks = taskCount
        let completed = completedTaskCount
        metricsLock.unlock()
        
        return [
            "cpuCores": cpuInfo.totalCores,
            "isHeterogeneous": cpuInfo.isHeterogeneous,
            "deviceModel": cpuInfo.deviceModel,
            "backgroundQueue": [
                "maxConcurrent": backgroundQueue.maxConcurrentOperationCount,
                "operationCount": backgroundQueue.operationCount
            ],
            "networkQueue": [
                "maxConcurrent": networkQueue.maxConcurrentOperationCount,
                "operationCount": networkQueue.operationCount
            ],
            "computeQueue": [
                "maxConcurrent": computeQueue.maxConcurrentOperationCount,
                "operationCount": computeQueue.operationCount
            ],
            "mediaQueue": [
                "maxConcurrent": mediaQueue.maxConcurrentOperationCount,
                "operationCount": mediaQueue.operationCount
            ],
            "totalTasksSubmitted": tasks,
            "totalTasksCompleted": completed
        ]
    }
    
    // MARK: - Queue Management
    
    /// Cancels all pending operations.
    public func cancelAllOperations() {
        backgroundQueue.cancelAllOperations()
        networkQueue.cancelAllOperations()
        computeQueue.cancelAllOperations()
        mediaQueue.cancelAllOperations()
    }
    
    /// Waits until all operations are finished.
    public func waitUntilAllOperationsAreFinished() {
        backgroundQueue.waitUntilAllOperationsAreFinished()
        networkQueue.waitUntilAllOperationsAreFinished()
        computeQueue.waitUntilAllOperationsAreFinished()
        mediaQueue.waitUntilAllOperationsAreFinished()
    }
    
    // MARK: - Private Helpers
    
    private func incrementTaskCount() {
        metricsLock.lock()
        taskCount += 1
        metricsLock.unlock()
    }
    
    private func incrementCompletedCount() {
        metricsLock.lock()
        completedTaskCount += 1
        metricsLock.unlock()
    }
    
    private static func detectCpuInfo() -> CpuInfo {
        let totalCores = ProcessInfo.processInfo.processorCount
        
        var systemInfo = utsname()
        uname(&systemInfo)
        let deviceModel = withUnsafePointer(to: &systemInfo.machine) {
            $0.withMemoryRebound(to: CChar.self, capacity: 1) {
                String(validatingUTF8: $0) ?? "Unknown"
            }
        }
        
        // Apple TV models typically have heterogeneous cores
        // Apple TV 4K (2021+) uses A12/A15 chips with performance + efficiency cores
        let isHeterogeneous = deviceModel.contains("AppleTV")
        
        var performanceCores = totalCores
        var efficiencyCores = 0
        
        if isHeterogeneous {
            // A12/A15 typically have 2 performance + 4 efficiency cores
            performanceCores = min(2, totalCores)
            efficiencyCores = totalCores - performanceCores
        }
        
        return CpuInfo(
            totalCores: totalCores,
            performanceCores: performanceCores,
            efficiencyCores: efficiencyCores,
            isHeterogeneous: isHeterogeneous,
            deviceModel: deviceModel
        )
    }
}
