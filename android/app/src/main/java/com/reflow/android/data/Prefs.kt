package com.reflow.android.data

import android.content.Context
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey

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
            return ServerConnection(host, port, token, name)
        }
        set(value) {
            if (value == null) {
                prefs.edit().clear().apply()
            } else {
                prefs.edit()
                    .putString("host", value.host)
                    .putInt("port", value.port)
                    .putString("token", value.token)
                    .putString("server", value.serverName)
                    .apply()
            }
        }

    var injectOnDesktop: Boolean
        get() = prefs.getBoolean("inject", false)
        set(value) { prefs.edit().putBoolean("inject", value).apply() }

    var language: String
        get() = prefs.getString("language", "auto") ?: "auto"
        set(value) { prefs.edit().putString("language", value).apply() }
}
