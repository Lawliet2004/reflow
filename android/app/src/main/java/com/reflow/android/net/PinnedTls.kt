package com.reflow.android.net

import com.reflow.android.data.ServerConnection
import okhttp3.ConnectionSpec
import okhttp3.OkHttpClient
import java.security.MessageDigest
import java.security.cert.CertificateException
import java.security.cert.X509Certificate
import javax.net.ssl.SSLContext
import javax.net.ssl.X509TrustManager

fun isCertificatePin(value: String) = value.matches(Regex("[a-f0-9]{64}"))

fun certificateSha256(certificate: X509Certificate): String =
    MessageDigest.getInstance("SHA-256").digest(certificate.encoded)
        .joinToString("") { "%02x".format(it.toInt() and 0xff) }

fun isSecureConnection(connection: ServerConnection) =
    isAllowedLanHost(connection.host) && connection.port in 1..65535 &&
        connection.token.isNotBlank() && isCertificatePin(connection.certificateSha256)

/** A pairing pin, copied from the desktop, is the sole accepted desktop identity. */
class PinnedCertificateTrustManager(pin: String) : X509TrustManager {
    init { require(isCertificatePin(pin)) { "Copy a valid desktop certificate fingerprint and pair again" } }
    private val expected = pin.chunked(2).map { it.toInt(16).toByte() }.toByteArray()

    override fun checkServerTrusted(chain: Array<out X509Certificate>, authType: String) {
        val leaf = chain.firstOrNull() ?: throw CertificateException("Desktop certificate is missing")
        leaf.checkValidity()
        val observed = MessageDigest.getInstance("SHA-256").digest(leaf.encoded)
        if (!MessageDigest.isEqual(expected, observed)) {
            throw CertificateException("Desktop identity changed. Copy a new pairing link from the trusted desktop.")
        }
    }

    override fun checkClientTrusted(chain: Array<out X509Certificate>, authType: String) {
        throw CertificateException("Client certificate authentication is not supported")
    }

    override fun getAcceptedIssuers(): Array<X509Certificate> = emptyArray()
}

fun pinnedClient(base: OkHttpClient, pin: String): OkHttpClient {
    val trust = PinnedCertificateTrustManager(pin)
    val context = SSLContext.getInstance("TLS")
    context.init(null, arrayOf(trust), null)
    return base.newBuilder()
        .sslSocketFactory(context.socketFactory, trust)
        .connectionSpecs(listOf(ConnectionSpec.MODERN_TLS))
        .hostnameVerifier { _, session ->
            runCatching {
                val leaf = session.peerCertificates.firstOrNull() as? X509Certificate
                    ?: throw CertificateException("Desktop did not supply a certificate")
                trust.checkServerTrusted(arrayOf(leaf), "TLS")
                true
            }.getOrDefault(false)
        }
        .followRedirects(false)
        .followSslRedirects(false)
        .build()
}
