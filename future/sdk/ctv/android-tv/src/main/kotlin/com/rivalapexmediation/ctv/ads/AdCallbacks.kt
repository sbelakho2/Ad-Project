package com.rivalapexmediation.ctv.ads

interface AdLoadCallback {
    fun onLoaded()
    fun onError(code: String)
}

interface AdShowCallback {
    fun onShown()
    fun onClosed()
    fun onError(code: String)
}
