package com.rivalapexmediation.ctv.network

sealed interface LoadError {
    object NoFill : LoadError
    object Timeout : LoadError
    object Network : LoadError
    data class Status(val label: String, val code: Int) : LoadError
    data class Generic(val reason: String) : LoadError
}

fun LoadError.reason(): String = when (this) {
    LoadError.NoFill -> "no_fill"
    LoadError.Timeout -> "timeout"
    LoadError.Network -> "network_error"
    is LoadError.Status -> label
    is LoadError.Generic -> reason
}
