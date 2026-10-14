package com.rivalapexmediation.sdk.adapters.ironsource

import com.google.gson.Gson
import com.rivalapexmediation.sdk.contract.AdapterError
import com.rivalapexmediation.sdk.contract.ErrorCode
import okhttp3.HttpUrl
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import java.io.IOException
import java.util.concurrent.TimeUnit

internal class IronSourceApiClient(
    private val baseUrl: HttpUrl,
    private val secret: String,
    private val gson: Gson = Gson(),
    private val httpClient: OkHttpClient = OkHttpClient.Builder()
        .retryOnConnectionFailure(false)
        .build()
) {
    fun loadBid(request: IronSourceBidRequest, timeoutMs: Int): IronSourceBidResponse {
        val boundedClient = httpClient.newBuilder()
            .callTimeout(timeoutMs.toLong(), TimeUnit.MILLISECONDS)
            .connectTimeout(timeoutMs.toLong(), TimeUnit.MILLISECONDS)
            .readTimeout(timeoutMs.toLong(), TimeUnit.MILLISECONDS)
            .writeTimeout(timeoutMs.toLong(), TimeUnit.MILLISECONDS)
            .build()

        val json = gson.toJson(request)
        val httpRequest = Request.Builder()
            .url(baseUrl)
            .post(json.toRequestBody("application/json".toMediaType()))
            .addHeader("Content-Type", "application/json")
            .addHeader("Authorization", "Bearer $secret")
            .build()

        try {
            boundedClient.newCall(httpRequest).execute().use { response ->
                if (!response.isSuccessful) {
                    throw mapHttpError(response.code)
                }
                val body = response.body?.string().orEmpty()
                if (body.isBlank()) {
                    throw AdapterError.Recoverable(ErrorCode.NO_FILL, "ironSource empty response")
                }
                val parsed = gson.fromJson(body, IronSourceBidResponse::class.java)
                if (parsed.providerName.isNullOrBlank() || parsed.adMarkup.isNullOrBlank()) {
                    throw AdapterError.Recoverable(ErrorCode.NO_FILL, "ironSource no fill")
                }
                return parsed
            }
        } catch (error: AdapterError) {
            throw error
        } catch (io: IOException) {
            throw AdapterError.Recoverable(ErrorCode.NETWORK_ERROR, io.message ?: "network_error")
        } catch (t: Throwable) {
            throw AdapterError.Recoverable(ErrorCode.ERROR, t.message ?: "unknown_error")
        }
    }

    private fun mapHttpError(code: Int): AdapterError {
        return when {
            code == 204 -> AdapterError.Recoverable(ErrorCode.NO_FILL, "ironSource 204 no fill")
            code == 408 -> AdapterError.Recoverable(ErrorCode.TIMEOUT, "ironSource timeout")
            code in 500..599 -> AdapterError.Recoverable(ErrorCode.STATUS_5XX, "ironSource $code")
            code == 401 || code == 403 -> AdapterError.Fatal(ErrorCode.STATUS_4XX, "ironSource auth failed $code")
            code in 400..499 -> AdapterError.Fatal(ErrorCode.STATUS_4XX, "ironSource HTTP $code")
            else -> AdapterError.Recoverable(ErrorCode.ERROR, "ironSource HTTP $code")
        }
    }
}
