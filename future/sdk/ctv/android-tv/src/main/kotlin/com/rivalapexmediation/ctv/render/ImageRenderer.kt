package com.rivalapexmediation.ctv.render

import android.graphics.BitmapFactory
import android.widget.ImageView
import com.rivalapexmediation.ctv.util.Logger
import okhttp3.OkHttpClient
import okhttp3.Request
import java.util.concurrent.TimeUnit

internal object ImageRenderer {
    private val client: OkHttpClient = OkHttpClient.Builder()
        .connectTimeout(5000, TimeUnit.MILLISECONDS)
        .readTimeout(5000, TimeUnit.MILLISECONDS)
        .build()

    fun load(url: String, target: ImageView, onReady: (() -> Unit)? = null, onError: ((String) -> Unit)? = null) {
        try {
            val req = Request.Builder().url(url).get().build()
            client.newCall(req).enqueue(object: okhttp3.Callback {
                override fun onFailure(call: okhttp3.Call, e: java.io.IOException) {
                    onError?.invoke("image_network_error")
                }
                override fun onResponse(call: okhttp3.Call, response: okhttp3.Response) {
                    response.use { res ->
                        if (!res.isSuccessful) { onError?.invoke("image_http_${res.code}"); return }
                        val bytes = res.body?.bytes()
                        if (bytes == null) { onError?.invoke("image_empty"); return }
                        val bmp = BitmapFactory.decodeByteArray(bytes, 0, bytes.size)
                        if (bmp == null) { onError?.invoke("image_decode"); return }
                        target.post {
                            target.setImageBitmap(bmp)
                            try { onReady?.invoke() } catch (e: Exception) { Logger.w("image ready callback error", e) }
                        }
                    }
                }
            })
        } catch (e: Exception) {
            Logger.w("ImageRenderer error: ${e.message}", e)
            onError?.invoke("image_exception")
        }
    }
}
