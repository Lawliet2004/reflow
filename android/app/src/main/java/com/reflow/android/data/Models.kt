package com.reflow.android.data

data class ServerConnection(
    val host: String,
    val port: Int,
    val token: String,
    val serverName: String,
)

data class HistoryItem(
    val id: String,
    val createdAt: String,
    val text: String,
    val language: String,
)

data class StreamPartial(
    val fullText: String,
    val language: String,
    val audioLevel: Float,
)

data class StreamFinal(
    val text: String,
    val raw: String,
    val language: String,
)
