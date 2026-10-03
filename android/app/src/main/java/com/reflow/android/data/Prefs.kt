package com.reflow.android.data

import android.content.Context
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey
import com.reflow.android.net.isSecureConnection

class Prefs(context: Context) {
    private val master = MasterKey.Builder(context)
        .setKeyScheme(MasterKey.KeyScheme.AES256_GCM)
        .build()

    private val prefs = EncryptedSharedPreferences.create(
        context,
        "reflow_secure",
        master,
        EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
        EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM,
    )

    var connection: ServerConnection?
        get() {
            val host = prefs.getString("host", null) ?: return null
            val token = prefs.getString("token", null) ?: return null
            val port = prefs.getInt("port", 7840)
            val name = prefs.getString("server", "Reflow") ?: "Reflow"
            val pin = prefs.getString("certificate_sha256", "") ?: ""
            return ServerConnection(host, port, token, name, pin).takeIf { isSecureConnection(it) }
        }
        set(value) {
            if (value == null) {
                prefs.edit().clear().apply()
            } else {
                require(isSecureConnection(value)) { "Secure pairing is required" }
                prefs.edit()
                    .putString("host", value.host)
                    .putInt("port", value.port)
                    .putString("token", value.token)
                    .putString("server", value.serverName)
                    .putString("certificate_sha256", value.certificateSha256)
                    .apply()
            }
        }

    val needsSecurePairing: Boolean
        get() = prefs.getString("token", null) != null && connection == null

    var injectOnDesktop: Boolean
        get() = prefs.getBoolean("inject", false)
        set(value) { prefs.edit().putBoolean("inject", value).apply() }

    var language: String
        get() = prefs.getString("language", "auto") ?: "auto"
        set(value) { prefs.edit().putString("language", value).apply() }
}
