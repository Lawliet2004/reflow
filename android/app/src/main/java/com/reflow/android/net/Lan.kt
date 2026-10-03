package com.reflow.android.net

import java.net.URLDecoder

fun isAllowedLanHost(host: String): Boolean {
    if (host == "localhost" || host == "127.0.0.1" || host == "::1") return true
    // Literal addresses avoid validating one DNS answer and connecting to another.
    val parts = host.split('.')
    if (parts.size != 4 || parts.any { !it.matches(Regex("0|[1-9][0-9]{0,2}")) }) return false
    val bytes = parts.map { it.toIntOrNull() ?: return false }
    if (bytes.any { it !in 0..255 }) return false
    return bytes[0] == 10 || (bytes[0] == 192 && bytes[1] == 168) ||
        (bytes[0] == 172 && bytes[1] in 16..31)
}

data class PairOffer(val host: String, val port: Int, val code: String, val certificateSha256: String)

fun parsePairUri(uri: String): PairOffer? {
    val trimmed = uri.trim()
    val query = when {
        trimmed.startsWith("reflow://pair?") -> trimmed.removePrefix("reflow://pair?")
        else -> return null
    }
    val map = query.split("&").mapNotNull {
        val parts = it.split("=", limit = 2)
        if (parts.size == 2) runCatching {
            URLDecoder.decode(parts[0], "UTF-8") to URLDecoder.decode(parts[1], "UTF-8")
        }.getOrNull() else null
    }.toMap()
    val host = map["host"] ?: return null
    val port = if (map.containsKey("port")) map["port"]?.toIntOrNull() ?: return null else 7840
    val code = map["code"] ?: return null
    val pin = map["cert_sha256"] ?: return null
    if (map["v"] != "2" || !isCertificatePin(pin)) return null
    if (!isAllowedLanHost(host) || port !in 1..65535 || !code.matches(Regex("[0-9]{6}"))) return null
    return PairOffer(host, port, code, pin)
}
