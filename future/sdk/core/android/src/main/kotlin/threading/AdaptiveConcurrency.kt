package com.rivalapexmediation.sdk.threading

import android.os.Build
import java.io.BufferedReader
import java.io.File
import java.io.FileReader
import java.util.concurrent.*
import java.util.concurrent.atomic.AtomicInteger
import kotlin.math.max
import kotlin.math.min

/**
 * AdaptiveConcurrency - Adaptive thread pool sizing based on device capabilities.
 * 
 * This class provides adaptive concurrency management that:
 * - Detects CPU core count and big.LITTLE architecture
 * - Sizes thread pools appropriately for the device
 * - Provides separate pools for CPU-bound and I/O-bound operations
 * - Monitors and adjusts pool sizes based on load
 */
class AdaptiveConcurrency private constructor() {
    
    companion object {
        @Volatile
        private var instance: AdaptiveConcurrency? = null
        
        fun getInstance(): AdaptiveConcurrency {
            return instance ?: synchronized(this) {
                instance ?: AdaptiveConcurrency().also { instance = it }
            }
        }
        
        /** Minimum threads for any pool */
        private const val MIN_THREADS = 2
        
        /** Maximum threads for background work */
        private const val MAX_BACKGROUND_THREADS = 8
        
        /** Maximum threads for network I/O */
        private const val MAX_NETWORK_THREADS = 16
        
        /** Maximum threads for compute-intensive work */
        private const val MAX_COMPUTE_THREADS = 4
    }
    
    /**
     * Detected CPU information for the current device.
     */
    data class CpuInfo(
        val totalCores: Int,
        val performanceCores: Int,
        val efficiencyCores: Int,
        val isBigLittle: Boolean,
        val maxFrequencyMHz: Int
    )
    
    /**
     * Pool configuration for different workload types.
     */
    data class PoolConfig(
        val coreSize: Int,
        val maxSize: Int,
        val keepAliveSeconds: Long,
        val queueCapacity: Int
    )
    
    private val cpuInfo: CpuInfo by lazy { detectCpuInfo() }
    
    // Adaptive thread pools
    private val backgroundPool: ThreadPoolExecutor by lazy { createBackgroundPool() }
    private val networkPool: ThreadPoolExecutor by lazy { createNetworkPool() }
    private val computePool: ThreadPoolExecutor by lazy { createComputePool() }
    
    // Metrics
    private val taskCount = AtomicInteger(0)
    private val completedTaskCount = AtomicInteger(0)
    private val rejectedTaskCount = AtomicInteger(0)
    
    /**
     * Gets the detected CPU information.
     */
    val cpuInformation: CpuInfo
        get() = cpuInfo

    /**
     * Gets the recommended pool configuration for background work.
     */
    fun getBackgroundPoolConfig(): PoolConfig {
        val cores = cpuInfo.totalCores
        val coreSize = max(MIN_THREADS, min(cores - 1, MAX_BACKGROUND_THREADS / 2))
        val maxSize = min(cores * 2, MAX_BACKGROUND_THREADS)
        return PoolConfig(
            coreSize = coreSize,
            maxSize = maxSize,
            keepAliveSeconds = 60,
            queueCapacity = 100
        )
    }
    
    /**
     * Gets the recommended pool configuration for network I/O.
     */
    fun getNetworkPoolConfig(): PoolConfig {
        // Network I/O is mostly waiting, so we can have more threads
        val cores = cpuInfo.totalCores
        val coreSize = max(MIN_THREADS, cores)
        val maxSize = min(cores * 4, MAX_NETWORK_THREADS)
        return PoolConfig(
            coreSize = coreSize,
            maxSize = maxSize,
            keepAliveSeconds = 30,
            queueCapacity = 200
        )
    }
    
    /**
     * Gets the recommended pool configuration for compute-intensive work.
     */
    fun getComputePoolConfig(): PoolConfig {
        // Compute work should use performance cores preferentially
        val cores = if (cpuInfo.isBigLittle) {
            cpuInfo.performanceCores
        } else {
            cpuInfo.totalCores
        }
        val poolSize = max(MIN_THREADS, min(cores, MAX_COMPUTE_THREADS))
        return PoolConfig(
            coreSize = poolSize,
            maxSize = poolSize, // Fixed size for compute
            keepAliveSeconds = 120,
            queueCapacity = 50
        )
    }
    
    /**
     * Executes a task on the background thread pool.
     */
    fun executeBackground(task: Runnable) {
        taskCount.incrementAndGet()
        try {
            backgroundPool.execute {
                try {
                    task.run()
                } finally {
                    completedTaskCount.incrementAndGet()
                }
            }
        } catch (e: RejectedExecutionException) {
            rejectedTaskCount.incrementAndGet()
            throw e
        }
    }
    
    /**
     * Executes a task on the network I/O thread pool.
     */
    fun executeNetwork(task: Runnable) {
        taskCount.incrementAndGet()
        try {
            networkPool.execute {
                try {
                    task.run()
                } finally {
                    completedTaskCount.incrementAndGet()
                }
            }
        } catch (e: RejectedExecutionException) {
            rejectedTaskCount.incrementAndGet()
            throw e
        }
    }
    
    /**
     * Executes a compute-intensive task on the compute thread pool.
     */
    fun executeCompute(task: Runnable) {
        taskCount.incrementAndGet()
        try {
            computePool.execute {
                try {
                    task.run()
                } finally {
                    completedTaskCount.incrementAndGet()
                }
            }
        } catch (e: RejectedExecutionException) {
            rejectedTaskCount.incrementAndGet()
            throw e
        }
    }
    
    /**
     * Submits a callable to the background pool and returns a Future.
     */
    fun <T> submitBackground(task: Callable<T>): Future<T> {
        taskCount.incrementAndGet()
        return backgroundPool.submit(Callable {
            try {
                task.call()
            } finally {
                completedTaskCount.incrementAndGet()
            }
        })
    }
    
    /**
     * Submits a callable to the network pool and returns a Future.
     */
    fun <T> submitNetwork(task: Callable<T>): Future<T> {
        taskCount.incrementAndGet()
        return networkPool.submit(Callable {
            try {
                task.call()
            } finally {
                completedTaskCount.incrementAndGet()
            }
        })
    }
    
    /**
     * Submits a callable to the compute pool and returns a Future.
     */
    fun <T> submitCompute(task: Callable<T>): Future<T> {
        taskCount.incrementAndGet()
        return computePool.submit(Callable {
            try {
                task.call()
            } finally {
                completedTaskCount.incrementAndGet()
            }
        })
    }
    
    /**
     * Gets current pool statistics.
     */
    fun getStats(): Map<String, Any> {
        return mapOf(
            "cpuCores" to cpuInfo.totalCores,
            "isBigLittle" to cpuInfo.isBigLittle,
            "backgroundPool" to getPoolStats(backgroundPool),
            "networkPool" to getPoolStats(networkPool),
            "computePool" to getPoolStats(computePool),
            "totalTasksSubmitted" to taskCount.get(),
            "totalTasksCompleted" to completedTaskCount.get(),
            "totalTasksRejected" to rejectedTaskCount.get()
        )
    }
    
    private fun getPoolStats(pool: ThreadPoolExecutor): Map<String, Any> {
        return mapOf(
            "corePoolSize" to pool.corePoolSize,
            "maxPoolSize" to pool.maximumPoolSize,
            "activeCount" to pool.activeCount,
            "poolSize" to pool.poolSize,
            "largestPoolSize" to pool.largestPoolSize,
            "taskCount" to pool.taskCount,
            "completedTaskCount" to pool.completedTaskCount,
            "queueSize" to pool.queue.size
        )
    }
    
    /**
     * Shuts down all thread pools gracefully.
     */
    fun shutdown() {
        backgroundPool.shutdown()
        networkPool.shutdown()
        computePool.shutdown()
    }
    
    /**
     * Shuts down all thread pools immediately.
     */
    fun shutdownNow(): List<Runnable> {
        val pending = mutableListOf<Runnable>()
        pending.addAll(backgroundPool.shutdownNow())
        pending.addAll(networkPool.shutdownNow())
        pending.addAll(computePool.shutdownNow())
        return pending
    }
    
    /**
     * Awaits termination of all pools.
     */
    fun awaitTermination(timeout: Long, unit: TimeUnit): Boolean {
        val deadline = System.nanoTime() + unit.toNanos(timeout)
        
        var remaining = deadline - System.nanoTime()
        if (!backgroundPool.awaitTermination(remaining, TimeUnit.NANOSECONDS)) {
            return false
        }
        
        remaining = deadline - System.nanoTime()
        if (!networkPool.awaitTermination(remaining, TimeUnit.NANOSECONDS)) {
            return false
        }
        
        remaining = deadline - System.nanoTime()
        return computePool.awaitTermination(remaining, TimeUnit.NANOSECONDS)
    }
    
    private fun createBackgroundPool(): ThreadPoolExecutor {
        val config = getBackgroundPoolConfig()
        return ThreadPoolExecutor(
            config.coreSize,
            config.maxSize,
            config.keepAliveSeconds,
            TimeUnit.SECONDS,
            LinkedBlockingQueue(config.queueCapacity),
            createThreadFactory("Background")
        ).apply {
            allowCoreThreadTimeOut(true)
        }
    }
    
    private fun createNetworkPool(): ThreadPoolExecutor {
        val config = getNetworkPoolConfig()
        return ThreadPoolExecutor(
            config.coreSize,
            config.maxSize,
            config.keepAliveSeconds,
            TimeUnit.SECONDS,
            LinkedBlockingQueue(config.queueCapacity),
            createThreadFactory("Network")
        ).apply {
            allowCoreThreadTimeOut(true)
        }
    }
    
    private fun createComputePool(): ThreadPoolExecutor {
        val config = getComputePoolConfig()
        return ThreadPoolExecutor(
            config.coreSize,
            config.maxSize,
            config.keepAliveSeconds,
            TimeUnit.SECONDS,
            LinkedBlockingQueue(config.queueCapacity),
            createThreadFactory("Compute")
        ).apply {
            // Keep compute threads alive for longer
            allowCoreThreadTimeOut(false)
        }
    }
    
    private fun createThreadFactory(prefix: String): ThreadFactory {
        val counter = AtomicInteger(0)
        return ThreadFactory { runnable ->
            Thread(runnable, "RivalApex-$prefix-${counter.incrementAndGet()}").apply {
                isDaemon = true
                priority = when (prefix) {
                    "Compute" -> Thread.NORM_PRIORITY + 1
                    "Network" -> Thread.NORM_PRIORITY
                    else -> Thread.NORM_PRIORITY - 1
                }
            }
        }
    }
    
    private fun detectCpuInfo(): CpuInfo {
        val totalCores = Runtime.getRuntime().availableProcessors()
        
        // Try to detect big.LITTLE architecture
        val coreFrequencies = try {
            getCoreFrequencies()
        } catch (e: Exception) {
            emptyList()
        }
        
        return if (coreFrequencies.isNotEmpty() && hasBigLittleArchitecture(coreFrequencies)) {
            val maxFreq = coreFrequencies.maxOrNull() ?: 0
            val threshold = maxFreq * 0.75
            val performanceCores = coreFrequencies.count { it >= threshold }
            val efficiencyCores = totalCores - performanceCores
            
            CpuInfo(
                totalCores = totalCores,
                performanceCores = performanceCores,
                efficiencyCores = efficiencyCores,
                isBigLittle = true,
                maxFrequencyMHz = maxFreq / 1000
            )
        } else {
            CpuInfo(
                totalCores = totalCores,
                performanceCores = totalCores,
                efficiencyCores = 0,
                isBigLittle = false,
                maxFrequencyMHz = coreFrequencies.maxOrNull()?.div(1000) ?: 0
            )
        }
    }
    
    private fun getCoreFrequencies(): List<Int> {
        val frequencies = mutableListOf<Int>()
        val cpuDir = File("/sys/devices/system/cpu")
        
        if (cpuDir.exists() && cpuDir.isDirectory) {
            cpuDir.listFiles()?.filter { 
                it.name.startsWith("cpu") && it.name.substring(3).toIntOrNull() != null 
            }?.forEach { cpuCore ->
                val maxFreqFile = File(cpuCore, "cpufreq/cpuinfo_max_freq")
                if (maxFreqFile.exists()) {
                    try {
                        BufferedReader(FileReader(maxFreqFile)).use { reader ->
                            reader.readLine()?.toIntOrNull()?.let { freq ->
                                frequencies.add(freq)
                            }
                        }
                    } catch (e: Exception) {
                        // Ignore read errors
                    }
                }
            }
        }
        
        return frequencies
    }
    
    private fun hasBigLittleArchitecture(frequencies: List<Int>): Boolean {
        if (frequencies.size < 2) return false
        
        val sorted = frequencies.sorted()
        val min = sorted.first()
        val max = sorted.last()
        
        // Consider it big.LITTLE if max is at least 1.5x min
        return max >= min * 1.5
    }
}
