package com.rivalapexmediation.sdk.config

import android.content.Context
import androidx.test.core.app.ApplicationProvider
import com.rivalapexmediation.sdk.SDKConfig
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Protocol
import okhttp3.Response
import okhttp3.ResponseBody.Companion.toResponseBody
import org.junit.Test

class ConfigManagerSigningTest {

    @Test(expected = IllegalStateException::class)
    fun failsWhenPublicKeyMissingInProduction() {
        val context = ApplicationProvider.getApplicationContext<Context>()
        val cfg = SDKConfig(
            appId = "app",
            configEndpoint = "https://example.com",
            auctionEndpoint = "https://example.com",
            telemetryEnabled = true,
            logLevel = com.rivalapexmediation.sdk.LogLevel.INFO,
            testMode = false
        )

        val fakeJson = """
            {
              "configId": "cfg",
              "version": 1,
              "placements": {},
              "adapters": {},
              "features": {"telemetryEnabled": true},
              "signature": "",
              "timestamp": 1
            }
        """.trimIndent()

        val client = OkHttpClient.Builder()
            .addInterceptor { chain ->
                Response.Builder()
                    .request(chain.request())
                    .protocol(Protocol.HTTP_1_1)
                    .code(200)
                    .message("OK")
                    .body(fakeJson.toResponseBody("application/json".toMediaType()))
                    .build()
            }
            .build()

        val manager = ConfigManager(
            context = context,
            sdkConfig = cfg,
            client = client,
            configPublicKey = null
        )

        manager.loadConfig()
    }
}
