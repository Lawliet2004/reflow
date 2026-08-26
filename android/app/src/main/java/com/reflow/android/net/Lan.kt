package com.reflow.android.net

import java.net.InetAddress

fun isAllowedLanHost(host: String): Boolean {
    if (host == "localhost" || host == "127.0.0.1" || host == "::1") return true
    return try {
        val addr = InetAddress.getByName(host)
        val b = addr.address
        if (b.size != 4) return false
        val a0 = b[0].toInt() and 0xff
        val a1 = b[1].toInt() and 0xff
        a0 == 10 || (a0 == 192 && a1 == 168) || (a0 == 172 && a1 in 16..31)
    } catch (_: Exception) {
        false
    }
}

fun parsePairUri(uri: String): Triple<String, Int, String>? {
    val trimmed = uri.trim()
    val query = when {
        trimmed.startsWith("reflow://pair?") -> trimmed.removePrefix("reflow://pair?")
        trimmed.startsWith("http") && trimmed.contains("?") -> trimmed.substringAfter("?")
        else -> return null
    }
    val map = query.split("&").mapNotNull {
        val parts = it.split("=", limit = 2)
        if (parts.size == 2) parts[0] to parts[1] else null
    }.toMap()
    val host = map["host"] ?: return null
    val port = map["port"]?.toIntOrNull() ?: 7840
    val code = map["code"] ?: return null
    return Triple(host, port, code)
}
