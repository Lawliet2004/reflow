package com.reflow.android.net

import com.reflow.android.data.HistoryItem
import com.reflow.android.data.ServerConnection
import com.reflow.android.data.StreamFinal
import com.reflow.android.data.StreamPartial
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okio.ByteString.Companion.toByteString
import org.json.JSONArray
import org.json.JSONObject
import java.util.concurrent.TimeUnit

class ReflowClient {
    private val http = OkHttpClient.Builder()
        .connectTimeout(8, TimeUnit.SECONDS)
        .readTimeout(0, TimeUnit.SECONDS)
        .build()

    fun pair(host: String, port: Int, code: String, deviceName: String): ServerConnection {
        require(isAllowedLanHost(host)) { "Host must be localhost or a private LAN address" }
        val body = JSONObject()
            .put("code", code)
            .put("device_name", deviceName)
            .toString()
            .toRequestBody("application/json".toMediaType())
        val request = Request.Builder()
            .url("http://$host:$port/v1/pair")
            .post(body)
            .build()
        http.newCall(request).execute().use { response ->
            val text = response.body?.string().orEmpty()
            if (!response.isSuccessful) {
                val message = runCatching { JSONObject(text).optString("message") }.getOrDefault(text)
                error(message.ifBlank { "Pairing failed (${response.code})" })
            }
            val json = JSONObject(text)
            return ServerConnection(
                host = host,
                port = json.optInt("port", port),
                token = json.getString("token"),
                serverName = json.optString("server_name", "Reflow"),
            )
        }
    }

    fun history(conn: ServerConnection): List<HistoryItem> {
        val request = authed(conn, "/v1/history?limit=50").get().build()
        http.newCall(request).execute().use { response ->
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

    fun inject(conn: ServerConnection, text: String) {
        val body = JSONObject().put("text", text).toString()
            .toRequestBody("application/json".toMediaType())
        val request = authed(conn, "/v1/inject").post(body).build()
        http.newCall(request).execute().use { response ->
            if (!response.isSuccessful) error("Inject failed (${response.code})")
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
        val request = Request.Builder()
            .url("ws://${conn.host}:${conn.port}/v1/stream")
            .header("Authorization", "Bearer ${conn.token}")
            .build()
        val ws = http.newWebSocket(request, object : WebSocketListener() {
            override fun onOpen(webSocket: WebSocket, response: okhttp3.Response) {
                val start = JSONObject()
                    .put("type", "start")
                    .put("language", language)
                    .put("format", "pcm_s16le")
                    .put("sample_rate", 16000)
                    .put("inject", inject)
                webSocket.send(start.toString())
            }

            override fun onMessage(webSocket: WebSocket, text: String) {
                val json = runCatching { JSONObject(text) }.getOrElse { return }
                when (json.optString("type")) {
                    "ready" -> onReady()
                    "partial" -> onPartial(
                        StreamPartial(
                            fullText = json.optString("full_text"),
                            language = json.optString("language"),
                            audioLevel = json.optDouble("audio_level", 0.0).toFloat(),
                        ),
                    )
                    "final" -> onFinal(
                        StreamFinal(
                            text = json.optString("text"),
                            raw = json.optString("raw"),
                            language = json.optString("language"),
                        ),
                    )
                    "error" -> onError(json.optString("message", "Stream error"))
                }
            }

            override fun onFailure(webSocket: WebSocket, t: Throwable, response: okhttp3.Response?) {
                onError(t.message ?: "WebSocket failed")
            }
        })
        return DictationSocket(ws)
    }

    private fun authed(conn: ServerConnection, path: String): Request.Builder {
        require(isAllowedLanHost(conn.host)) { "Host must be localhost or a private LAN address" }
        return Request.Builder()
            .url("http://${conn.host}:${conn.port}$path")
            .header("Authorization", "Bearer ${conn.token}")
    }
}

class DictationSocket(private val ws: WebSocket) {
    fun sendPcm(bytes: ByteArray) {
        ws.send(bytes.toByteString())
    }

    fun stop() {
        ws.send(JSONObject().put("type", "stop").toString())
    }

    fun cancel() {
        ws.send(JSONObject().put("type", "cancel").toString())
        ws.close(1000, "cancel")
    }

    fun close() {
        ws.close(1000, "done")
    }
}
