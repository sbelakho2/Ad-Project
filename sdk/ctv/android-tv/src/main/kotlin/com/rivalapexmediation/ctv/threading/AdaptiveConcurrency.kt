package com.rivalapexmediation.ctv.threading

import java.io.BufferedReader
import java.io.File
import java.io.FileReader
import java.util.concurrent.*
import java.util.concurrent.atomic.AtomicInteger
import kotlin.math.max
import kotlin.math.min
import kotlinx.coroutines.*

/**
 * AdaptiveConcurrency - Adaptive thread pool sizing for Android TV devices.
 * 
 * Optimized for Android TV's hardware characteristics:
 * - Wide range of device capabilities (low-end to high-end)
 * - Consistent AC power without battery concerns
 * - Media playback focus with hardware video decoding
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
        
        /** Maximum threads for background work (conservative for TV) */
        private const val MAX_BACKGROUND_THREADS = 4
        
        /** Maximum threads for network I/O */
        private const val MAX_NETWORK_THREADS = 8
        
        /** Maximum threads for compute-intensive work */
        private const val MAX_COMPUTE_THREADS = 2
        
        /** Maximum threads for media operations */
        private const val MAX_MEDIA_THREADS = 2
    }
    
    /**
     * Detected CPU information for the Android TV device.
     */
    data class CpuInfo(
        val totalCores: Int,
        val performanceCores: Int,
        val efficiencyCores: Int,
        val isBigLittle: Boolean,
        val maxFrequencyMHz: Int,
        val deviceType: DeviceType
    )
    
    /**
     * Android TV device capability tier.
     */
    enum class DeviceType {
        LOW_END,      // Older/budget devices (2 cores, low RAM)
        MID_RANGE,    // Average devices (4 cores)
        HIGH_END      // Premium devices (6+ cores, high performance)
    }
    
    /**
     * Pool configuration for different workload types.
     */
    data class PoolConfig(
        val coreSize: Int,
        val maxSize: Int,
        val keepAliveSeconds: Long,
        val queueCapacity: Int
    )
    
    private val _cpuInfo: CpuInfo by lazy { detectCpuInfo() }
    
    // Adaptive thread pools
    private val backgroundPool: ThreadPoolExecutor by lazy { createBackgroundPool() }
    private val networkPool: ThreadPoolExecutor by lazy { createNetworkPool() }
    private val computePool: ThreadPoolExecutor by lazy { createComputePool() }
    private val mediaPool: ThreadPoolExecutor by lazy { createMediaPool() }
    
    // Coroutine dispatchers
    val backgroundDispatcher: CoroutineDispatcher by lazy { backgroundPool.asCoroutineDispatcher() }
    val networkDispatcher: CoroutineDispatcher by lazy { networkPool.asCoroutineDispatcher() }
    val computeDispatcher: CoroutineDispatcher by lazy { computePool.asCoroutineDispatcher() }
    val mediaDispatcher: CoroutineDispatcher by lazy { mediaPool.asCoroutineDispatcher() }
    
    // Metrics
    private val taskCount = AtomicInteger(0)
    private val completedTaskCount = AtomicInteger(0)
    private val rejectedTaskCount = AtomicInteger(0)
    
    /**
     * Gets the detected CPU information.
     */
    fun getCpuInfo(): CpuInfo = _cpuInfo
    
    /**
     * Gets the recommended pool configuration for background work.
     */
    fun getBackgroundPoolConfig(): PoolConfig {
        val cores = _cpuInfo.totalCores
        val (coreSize, maxSize) = when (_cpuInfo.deviceType) {
            DeviceType.LOW_END -> Pair(1, 2)
            DeviceType.MID_RANGE -> Pair(2, 3)
            DeviceType.HIGH_END -> Pair(2, MAX_BACKGROUND_THREADS)
        }
        return PoolConfig(
            coreSize = max(1, coreSize),
            maxSize = min(cores, maxSize),
            keepAliveSeconds = 60,
            queueCapacity = 50
        )
    }
    
    /**
     * Gets the recommended pool configuration for network I/O.
     */
    fun getNetworkPoolConfig(): PoolConfig {
        val cores = _cpuInfo.totalCores
        val (coreSize, maxSize) = when (_cpuInfo.deviceType) {
            DeviceType.LOW_END -> Pair(2, 4)
            DeviceType.MID_RANGE -> Pair(3, 6)
            DeviceType.HIGH_END -> Pair(4, MAX_NETWORK_THREADS)
        }
        return PoolConfig(
            coreSize = max(MIN_THREADS, coreSize),
            maxSize = min(cores * 2, maxSize),
            keepAliveSeconds = 30,
            queueCapacity = 100
        )
    }
    
    /**
     * Gets the recommended pool configuration for compute-intensive work.
     */
    fun getComputePoolConfig(): PoolConfig {
        val cores = if (_cpuInfo.isBigLittle) _cpuInfo.performanceCores else _cpuInfo.totalCores
        val poolSize = when (_cpuInfo.deviceType) {
            DeviceType.LOW_END -> 1
            DeviceType.MID_RANGE -> min(2, cores)
            DeviceType.HIGH_END -> min(MAX_COMPUTE_THREADS, cores)
        }
        return PoolConfig(
            coreSize = poolSize,
            maxSize = poolSize,
            keepAliveSeconds = 120,
            queueCapacity = 20
        )
    }
    
    /**
     * Gets the recommended pool configuration for media operations.
     */
    fun getMediaPoolConfig(): PoolConfig {
        // Media pool is conservative - hardware decoding handles most work
        return PoolConfig(
            coreSize = 1,
            maxSize = MAX_MEDIA_THREADS,
            keepAliveSeconds = 60,
            queueCapacity = 10
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
     * Executes a compute-intensive task.
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
     * Executes a media processing task.
     */
    fun executeMedia(task: Runnable) {
        taskCount.incrementAndGet()
        try {
            mediaPool.execute {
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
     * Submits a background task as a coroutine.
     */
    suspend fun <T> submitBackgroundAsync(block: suspend () -> T): T = withContext(backgroundDispatcher) {
        taskCount.incrementAndGet()
        try {
            block()
        } finally {
            completedTaskCount.incrementAndGet()
        }
    }
    
    /**
     * Submits a network task as a coroutine.
     */
    suspend fun <T> submitNetworkAsync(block: suspend () -> T): T = withContext(networkDispatcher) {
        taskCount.incrementAndGet()
        try {
            block()
        } finally {
            completedTaskCount.incrementAndGet()
        }
    }
    
    /**
     * Submits a compute task as a coroutine.
     */
    suspend fun <T> submitComputeAsync(block: suspend () -> T): T = withContext(computeDispatcher) {
        taskCount.incrementAndGet()
        try {
            block()
        } finally {
            completedTaskCount.incrementAndGet()
        }
    }
    
    /**
     * Gets current pool statistics.
     */
    fun getStats(): Map<String, Any> {
        return mapOf(
            "cpuCores" to _cpuInfo.totalCores,
            "isBigLittle" to _cpuInfo.isBigLittle,
            "deviceType" to _cpuInfo.deviceType.name,
            "backgroundPool" to getPoolStats(backgroundPool),
            "networkPool" to getPoolStats(networkPool),
            "computePool" to getPoolStats(computePool),
            "mediaPool" to getPoolStats(mediaPool),
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
        mediaPool.shutdown()
    }
    
    /**
     * Shuts down all thread pools immediately.
     */
    fun shutdownNow(): List<Runnable> {
        val pending = mutableListOf<Runnable>()
        pending.addAll(backgroundPool.shutdownNow())
        pending.addAll(networkPool.shutdownNow())
        pending.addAll(computePool.shutdownNow())
        pending.addAll(mediaPool.shutdownNow())
        return pending
    }
    
    private fun createBackgroundPool(): ThreadPoolExecutor {
        val config = getBackgroundPoolConfig()
        return ThreadPoolExecutor(
            config.coreSize,
            config.maxSize,
            config.keepAliveSeconds,
            TimeUnit.SECONDS,
            LinkedBlockingQueue(config.queueCapacity),
            createThreadFactory("CTV-Background")
        ).apply { allowCoreThreadTimeOut(true) }
    }
    
    private fun createNetworkPool(): ThreadPoolExecutor {
        val config = getNetworkPoolConfig()
        return ThreadPoolExecutor(
            config.coreSize,
            config.maxSize,
            config.keepAliveSeconds,
            TimeUnit.SECONDS,
            LinkedBlockingQueue(config.queueCapacity),
            createThreadFactory("CTV-Network")
        ).apply { allowCoreThreadTimeOut(true) }
    }
    
    private fun createComputePool(): ThreadPoolExecutor {
        val config = getComputePoolConfig()
        return ThreadPoolExecutor(
            config.coreSize,
            config.maxSize,
            config.keepAliveSeconds,
            TimeUnit.SECONDS,
            LinkedBlockingQueue(config.queueCapacity),
            createThreadFactory("CTV-Compute")
        )
    }
    
    private fun createMediaPool(): ThreadPoolExecutor {
        val config = getMediaPoolConfig()
        return ThreadPoolExecutor(
            config.coreSize,
            config.maxSize,
            config.keepAliveSeconds,
            TimeUnit.SECONDS,
            LinkedBlockingQueue(config.queueCapacity),
            createThreadFactory("CTV-Media")
        )
    }
    
    private fun createThreadFactory(prefix: String): ThreadFactory {
        val counter = AtomicInteger(0)
        return ThreadFactory { runnable ->
            Thread(runnable, "RivalApex-$prefix-${counter.incrementAndGet()}").apply {
                isDaemon = true
                priority = when {
                    prefix.contains("Compute") -> Thread.NORM_PRIORITY + 1
                    prefix.contains("Media") -> Thread.NORM_PRIORITY + 1
                    prefix.contains("Network") -> Thread.NORM_PRIORITY
                    else -> Thread.NORM_PRIORITY - 1
                }
            }
        }
    }
    
    private fun detectCpuInfo(): CpuInfo {
        val totalCores = Runtime.getRuntime().availableProcessors()
        
        // Try to detect big.LITTLE architecture
        val coreFrequencies = try { getCoreFrequencies() } catch (e: Exception) { emptyList() }
        
        val hasBigLittle = coreFrequencies.isNotEmpty() && hasBigLittleArchitecture(coreFrequencies)
        
        val maxFreq = coreFrequencies.maxOrNull() ?: 0
        val performanceCores: Int
        val efficiencyCores: Int
        
        if (hasBigLittle) {
            val threshold = maxFreq * 0.75
            performanceCores = coreFrequencies.count { it >= threshold }
            efficiencyCores = totalCores - performanceCores
        } else {
            performanceCores = totalCores
            efficiencyCores = 0
        }
        
        // Determine device tier
        val deviceType = when {
            totalCores <= 2 -> DeviceType.LOW_END
            totalCores <= 4 -> DeviceType.MID_RANGE
            else -> DeviceType.HIGH_END
        }
        
        return CpuInfo(
            totalCores = totalCores,
            performanceCores = performanceCores,
            efficiencyCores = efficiencyCores,
            isBigLittle = hasBigLittle,
            maxFrequencyMHz = maxFreq / 1000,
            deviceType = deviceType
        )
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
                            reader.readLine()?.toIntOrNull()?.let { frequencies.add(it) }
                        }
                    } catch (e: Exception) { /* ignore */ }
                }
            }
        }
        
        return frequencies
    }
    
    private fun hasBigLittleArchitecture(frequencies: List<Int>): Boolean {
        if (frequencies.size < 2) return false
        val sorted = frequencies.sorted()
        return sorted.last() >= sorted.first() * 1.5
    }
}
