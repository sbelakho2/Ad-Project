package com.rivalapexmediation.ctv.ads

import java.util.ArrayDeque

internal class AdapterCircuitBreaker(
    private val failureThreshold: Int = 3,
    private val timeWindowMs: Long = 30_000,
    private val recoveryTimeMs: Long = 15_000,
    private val clock: Clock = SystemMonotonicClock
) {
    private enum class State { CLOSED, OPEN, HALF_OPEN }

    private val failures = ArrayDeque<Long>()
    private var state: State = State.CLOSED
    private var openedAtMs: Long = 0

    fun isOpen(): Boolean {
        val now = clock.now()
        if (state == State.OPEN && now - openedAtMs >= recoveryTimeMs) {
            state = State.HALF_OPEN
        }
        return state == State.OPEN
    }

    fun recordSuccess() {
        when (state) {
            State.HALF_OPEN -> {
                state = State.CLOSED
                failures.clear()
            }
            State.CLOSED -> failures.clear()
            State.OPEN -> {}
        }
    }

    fun recordFailure() {
        val now = clock.now()
        failures.addLast(now)
        while (failures.isNotEmpty() && now - failures.first > timeWindowMs) {
            failures.removeFirst()
        }

        if (state == State.HALF_OPEN) {
            state = State.OPEN
            openedAtMs = now
            return
        }

        if (state == State.CLOSED && failures.size >= failureThreshold) {
            state = State.OPEN
            openedAtMs = now
        }
    }
}
