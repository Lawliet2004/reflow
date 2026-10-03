package com.reflow.android.net

import com.reflow.android.data.HistoryItem
import com.reflow.android.data.ServerConnection
import com.reflow.android.data.StreamFinal
import com.reflow.android.data.StreamPartial
import kotlinx.coroutines.suspendCancellableCoroutine
import okhttp3.Call
import okhttp3.Callback
import okhttp3.HttpUrl
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okio.ByteString.Companion.toByteString
import org.json.JSONArray
import org.json.JSONObject
import java.io.IOException
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.coroutines.resumeWithException

class ReflowClient {
    private val http = OkHttpClient.Builder()
        .connectTimeout(8, TimeUnit.SECONDS)
        .readTimeout(15, TimeUnit.SECONDS)
        .writeTimeout(15, TimeUnit.SECONDS)
        .callTimeout(25, TimeUnit.SECONDS)
        .build()
    private data class Clients(val pin: String, val http: OkHttpClient, val streams: OkHttpClient)
    private var cached: Clients? = null

    @Synchronized
    private fun clients(pin: String): Clients {
        cached?.takeIf { it.pin == pin }?.let { return it }
        val secure = pinnedClient(http, pin)
        return Clients(pin, secure, secure.newBuilder()
            .readTimeout(0, TimeUnit.SECONDS)
            .callTimeout(0, TimeUnit.SECONDS)
            .pingInterval(15, TimeUnit.SECONDS)
            .build()).also { cached = it }
    }

    suspend fun pair(host: String, port: Int, code: String, deviceName: String, certificateSha256: String): ServerConnection {
        require(isAllowedLanHost(host)) { "Host must be localhost or a private LAN address" }
        require(code.matches(Regex("[0-9]{6}"))) { "Enter the six-digit pairing code" }
        val client = clients(certificateSha256).http
        val body = JSONObject()
            .put("code", code)
            .put("device_name", deviceName)
            .toString()
            .toRequestBody("application/json".toMediaType())
        val request = Request.Builder()
            .url(endpoint(host, port, "/v1/pair"))
            .post(body)
            .build()
        await(request, client).use { response ->
            val text = response.body?.string().orEmpty()
            if (!response.isSuccessful) {
                val message = runCatching { JSONObject(text).optString("message") }.getOrDefault(text)
                error(message.ifBlank { "Pairing failed (${response.code})" })
            }
            val json = JSONObject(text)
            val returnedPort = json.optInt("port", port)
            require(returnedPort in 1..65535) { "Desktop returned an invalid port" }
            return ServerConnection(
                host = host,
                port = returnedPort,
                token = json.getString("token"),
                serverName = json.optString("server_name", "Reflow"),
                certificateSha256 = certificateSha256,
            )
        }
    }

    suspend fun history(conn: ServerConnection): List<HistoryItem> {
        val request = authed(conn, "/v1/history?limit=50").get().build()
        await(request, clients(conn.certificateSha256).http).use { response ->
            val text = response.body?.string().orEmpty()
            if (!response.isSuccessful) error("History failed (${response.code})")
            val arr = JSONArray(text)
            return buildList {
                for (i in 0 until arr.length()) {
                    val o = arr.getJSONObject(i)
                    add(
                        HistoryItem(
                            id = o.getString("id"),
                            createdAt = o.optString("created_at"),
                            text = o.optString("final_transcript"),
                            language = o.optString("language"),
                        ),
                    )
                }
            }
        }
    }

    suspend fun inject(conn: ServerConnection, text: String) {
        val body = JSONObject().put("text", text).toString()
            .toRequestBody("application/json".toMediaType())
        val request = authed(conn, "/v1/inject").post(body).build()
        await(request, clients(conn.certificateSha256).http).use { response ->
            if (!response.isSuccessful) error("Inject failed (${response.code})")
            val result = JSONObject(response.body?.string().orEmpty().ifBlank { "{}" })
            if (!result.optBoolean("pasted", false)) {
                error("Desktop could not paste. Copy the transcript and paste it manually.")
            }
        }
    }

    fun openStream(
        conn: ServerConnection,
        language: String,
        inject: Boolean,
        onPartial: (StreamPartial) -> Unit,
        onFinal: (StreamFinal) -> Unit,
        onError: (String) -> Unit,
        onReady: () -> Unit,
    ): DictationSocket {
        validateConnection(conn)
        val request = Request.Builder()
            .url(endpoint(conn.host, conn.port, "/v1/stream"))
            .header("Authorization", "Bearer ${conn.token}")
            .build()
        val terminal = AtomicBoolean(false)
        val ws = clients(conn.certificateSha256).streams.newWebSocket(request, object : WebSocketListener() {
            private fun fail(socket: WebSocket, message: String) {
                if (terminal.compareAndSet(false, true)) onError(message)
                socket.cancel()
            }

            override fun onOpen(webSocket: WebSocket, response: okhttp3.Response) {
                val start = JSONObject()
                    .put("type", "start")
                    .put("language", language)
                    .put("format", "pcm_s16le")
                    .put("sample_rate", 16000)
                    .put("inject", inject)
                if (!webSocket.send(start.toString())) fail(webSocket, "Could not start the desktop session")
            }

            override fun onMessage(webSocket: WebSocket, text: String) {
                if (terminal.get()) return
                val json = runCatching { JSONObject(text) }.getOrElse {
                    fail(webSocket, "Desktop returned an invalid stream message")
                    return
                }
                when (json.optString("type")) {
                    "ready" -> onReady()
                    "partial" -> onPartial(
                        StreamPartial(
                            fullText = json.optString("full_text"),
                            language = json.optString("language"),
                            audioLevel = json.optDouble("audio_level", 0.0).toFloat(),
                        ),
                    )
                    "final" -> {
                        if (terminal.compareAndSet(false, true)) onFinal(StreamFinal(
                            text = json.optString("text"),
                            raw = json.optString("raw"),
                            language = json.optString("language"),
                        ))
                        webSocket.close(1000, "done")
                    }
                    "error" -> fail(webSocket, json.optString("message", "Stream error"))
                }
            }

            override fun onFailure(webSocket: WebSocket, t: Throwable, response: okhttp3.Response?) {
                fail(webSocket, t.message ?: "WebSocket failed")
            }

            override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
                webSocket.close(code, reason)
            }

            override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
                if (terminal.compareAndSet(false, true)) onError("Desktop connection closed. Try again.")
            }
        })
        return DictationSocket(ws)
    }

    private fun authed(conn: ServerConnection, path: String): Request.Builder {
        validateConnection(conn)
        return Request.Builder()
            .url(endpoint(conn.host, conn.port, path))
            .header("Authorization", "Bearer ${conn.token}")
    }

    private fun endpoint(host: String, port: Int, path: String): HttpUrl {
        require(isAllowedLanHost(host)) { "Use localhost or a literal private LAN IP address" }
        require(port in 1..65535) { "Port must be between 1 and 65535" }
        val address = HttpUrl.Builder().scheme("https").host(host).port(port).build()
        return requireNotNull(address.resolve(path))
    }

    private fun validateConnection(conn: ServerConnection) {
        require(isSecureConnection(conn)) { "This connection needs secure pairing. Copy a new pairing link from the desktop." }
    }

    private suspend fun await(request: Request, client: OkHttpClient): Response = suspendCancellableCoroutine { continuation ->
        val call = client.newCall(request)
        continuation.invokeOnCancellation { call.cancel() }
        call.enqueue(object : Callback {
            override fun onFailure(call: Call, e: IOException) {
                if (continuation.isActive) continuation.resumeWithException(e)
            }

            override fun onResponse(call: Call, response: Response) {
                continuation.resume(response) { _, value, _ -> value.close() }
            }
        })
    }
}

class DictationSocket(private val ws: WebSocket) {
    private val closed = AtomicBoolean(false)
    private val stopped = AtomicBoolean(false)
    fun sendPcm(bytes: ByteArray) {
        if (closed.get() || stopped.get() || ws.queueSize() > 512 * 1024 || !ws.send(bytes.toByteString())) {
            throw IOException("Phone connection is too slow or disconnected. Reconnect and try again.")
        }
    }

    fun stop() {
        if (closed.get() || !stopped.compareAndSet(false, true)) return
        if (!ws.send(JSONObject().put("type", "stop").toString())) throw IOException("Could not finish the desktop session")
    }

    fun cancel() {
        if (closed.compareAndSet(false, true)) {
            ws.send(JSONObject().put("type", "cancel").toString())
            ws.cancel()
        }
    }

    fun close() {
        if (closed.compareAndSet(false, true)) ws.close(1000, "done")
    }
}
