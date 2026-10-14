import Foundation
import os.log

/// AdaptiveConcurrency - Adaptive dispatch queue and operation queue sizing based on device capabilities.
///
/// This class provides adaptive concurrency management that:
/// - Detects CPU core count and processor type
/// - Sizes operation queues appropriately for the device
/// - Provides separate queues for CPU-bound and I/O-bound operations
/// - Uses QoS (Quality of Service) for priority management
public final class AdaptiveConcurrency {
    
    /// Shared singleton instance
    public static let shared = AdaptiveConcurrency()
    
    // MARK: - Configuration Constants
    
    private static let minConcurrentOperations = 2
    private static let maxBackgroundOperations = 8
    private static let maxNetworkOperations = 16
    private static let maxComputeOperations = 4
    
    // MARK: - Logging Helpers
    
    private func logInfo(_ message: String) {
        if #available(iOS 14.0, macOS 11.0, tvOS 14.0, watchOS 7.0, *) {
            let logger = Logger(subsystem: "com.rivalapexmediation.sdk", category: "AdaptiveConcurrency")
            logger.info("\(message)")
        } else {
            os_log("%{public}@", log: OSLog(subsystem: "com.rivalapexmediation.sdk", category: "AdaptiveConcurrency"), type: .info, message)
        }
    }
    
    // MARK: - CPU Information
    
    /// Detected CPU information for the current device.
    public struct CpuInfo {
        public let totalCores: Int
        public let performanceCores: Int
        public let efficiencyCores: Int
        public let isHeterogeneous: Bool
        public let processorType: String
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
    
    /// Dispatch queue for background work
    public let backgroundDispatchQueue: DispatchQueue
    
    /// Dispatch queue for network I/O
    public let networkDispatchQueue: DispatchQueue
    
    /// Dispatch queue for compute-intensive work
    public let computeDispatchQueue: DispatchQueue
    
    // MARK: - Metrics
    
    private let metricsLock = NSLock()
    private var taskCount: Int = 0
    private var completedTaskCount: Int = 0
    
    // MARK: - Initialization
    
    private init() {
        let cpuInfo = AdaptiveConcurrency.detectCpuInfo()
        self.cpuInfo = cpuInfo
        
        // Create background queue
        let backgroundConfig = Self.getPoolConfig(for: .background, cpuInfo: cpuInfo)
        self.backgroundQueue = OperationQueue()
        backgroundQueue.name = "com.rivalapexmediation.sdk.background"
        backgroundQueue.maxConcurrentOperationCount = backgroundConfig.maxConcurrentOperations
        backgroundQueue.qualityOfService = backgroundConfig.qualityOfService
        
        // Create network queue
        let networkConfig = Self.getPoolConfig(for: .network, cpuInfo: cpuInfo)
        self.networkQueue = OperationQueue()
        networkQueue.name = "com.rivalapexmediation.sdk.network"
        networkQueue.maxConcurrentOperationCount = networkConfig.maxConcurrentOperations
        networkQueue.qualityOfService = networkConfig.qualityOfService
        
        // Create compute queue
        let computeConfig = Self.getPoolConfig(for: .compute, cpuInfo: cpuInfo)
        self.computeQueue = OperationQueue()
        computeQueue.name = "com.rivalapexmediation.sdk.compute"
        computeQueue.maxConcurrentOperationCount = computeConfig.maxConcurrentOperations
        computeQueue.qualityOfService = computeConfig.qualityOfService
        
        // Create dispatch queues with appropriate attributes
        self.backgroundDispatchQueue = DispatchQueue(
            label: "com.rivalapexmediation.sdk.background.dispatch",
            qos: .utility,
            attributes: .concurrent
        )
        
        self.networkDispatchQueue = DispatchQueue(
            label: "com.rivalapexmediation.sdk.network.dispatch",
            qos: .userInitiated,
            attributes: .concurrent
        )
        
        self.computeDispatchQueue = DispatchQueue(
            label: "com.rivalapexmediation.sdk.compute.dispatch",
            qos: .userInteractive,
            attributes: .concurrent
        )
        
        logInfo("AdaptiveConcurrency initialized: \(self.cpuInfo.totalCores) cores, heterogeneous: \(self.cpuInfo.isHeterogeneous)")
    }
    
    // MARK: - Pool Configuration
    
    private enum PoolType {
        case background
        case network
        case compute
    }
    
    private static func getPoolConfig(for poolType: PoolType, cpuInfo: CpuInfo) -> PoolConfig {
        switch poolType {
        case .background:
            let cores = cpuInfo.totalCores
            let maxOps = max(minConcurrentOperations, min(cores - 1, maxBackgroundOperations))
            return PoolConfig(maxConcurrentOperations: maxOps, qualityOfService: .utility)
        case .network:
            let cores = cpuInfo.totalCores
            let maxOps = max(minConcurrentOperations, min(cores * 4, maxNetworkOperations))
            return PoolConfig(maxConcurrentOperations: maxOps, qualityOfService: .userInitiated)
        case .compute:
            let cores = cpuInfo.isHeterogeneous ? cpuInfo.performanceCores : cpuInfo.totalCores
            let maxOps = max(minConcurrentOperations, min(cores, maxComputeOperations))
            return PoolConfig(maxConcurrentOperations: maxOps, qualityOfService: .userInteractive)
        }
    }
    
    /// Gets the recommended pool configuration for background work.
    public func getBackgroundPoolConfig() -> PoolConfig {
        Self.getPoolConfig(for: .background, cpuInfo: cpuInfo)
    }
    
    /// Gets the recommended pool configuration for network I/O.
    public func getNetworkPoolConfig() -> PoolConfig {
        Self.getPoolConfig(for: .network, cpuInfo: cpuInfo)
    }
    
    /// Gets the recommended pool configuration for compute-intensive work.
    public func getComputePoolConfig() -> PoolConfig {
        Self.getPoolConfig(for: .compute, cpuInfo: cpuInfo)
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
    @available(iOS 13.0, tvOS 13.0, *)
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
    @available(iOS 13.0, tvOS 13.0, *)
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
    
    /// Executes a compute task asynchronously.
    @available(iOS 13.0, tvOS 13.0, *)
    public func executeComputeAsync<T>(_ work: @escaping () throws -> T) async throws -> T {
        incrementTaskCount()
        return try await withCheckedThrowingContinuation { continuation in
            computeQueue.addOperation {
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
            "processorType": cpuInfo.processorType,
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
    }
    
    /// Waits until all operations are finished.
    public func waitUntilAllOperationsAreFinished() {
        backgroundQueue.waitUntilAllOperationsAreFinished()
        networkQueue.waitUntilAllOperationsAreFinished()
        computeQueue.waitUntilAllOperationsAreFinished()
    }
    
    /// Suspends all queues.
    public func suspendQueues() {
        backgroundQueue.isSuspended = true
        networkQueue.isSuspended = true
        computeQueue.isSuspended = true
    }
    
    /// Resumes all queues.
    public func resumeQueues() {
        backgroundQueue.isSuspended = false
        networkQueue.isSuspended = false
        computeQueue.isSuspended = false
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
        let activeProcessors = ProcessInfo.processInfo.activeProcessorCount
        
        // Detect processor type
        var systemInfo = utsname()
        uname(&systemInfo)
        let machine = withUnsafePointer(to: &systemInfo.machine) {
            $0.withMemoryRebound(to: CChar.self, capacity: 1) {
                String(validatingUTF8: $0) ?? "Unknown"
            }
        }
        
        // Check for Apple Silicon (ARM-based chips have heterogeneous cores)
        let isAppleSilicon = machine.contains("arm") || machine.hasPrefix("iPhone") || 
                            machine.hasPrefix("iPad") || machine.hasPrefix("AppleTV")
        
        // Apple Silicon Macs and iOS devices typically have heterogeneous cores
        // We estimate the split based on typical configurations
        var performanceCores = totalCores
        var efficiencyCores = 0
        var isHeterogeneous = false
        
        if isAppleSilicon {
            isHeterogeneous = true
            // Typical Apple Silicon configurations:
            // A14+: 2 performance + 4 efficiency
            // M1: 4 performance + 4 efficiency
            // M1 Pro/Max: 8 performance + 2 efficiency
            // We'll estimate performance cores as ~50% of total
            performanceCores = max(2, totalCores / 2)
            efficiencyCores = totalCores - performanceCores
        }
        
        return CpuInfo(
            totalCores: totalCores,
            performanceCores: performanceCores,
            efficiencyCores: efficiencyCores,
            isHeterogeneous: isHeterogeneous,
            processorType: machine
        )
    }
}

// MARK: - OperationQueue Extension for Convenience

extension OperationQueue {
    /// Creates an operation that will run the given work.
    public func addAsyncOperation(_ work: @escaping () async throws -> Void) {
        let operation = BlockOperation()
        operation.addExecutionBlock {
            let semaphore = DispatchSemaphore(value: 0)
            Task {
                do {
                    try await work()
                } catch {
                    // Log error if needed
                }
                semaphore.signal()
            }
            semaphore.wait()
        }
        addOperation(operation)
    }
}
