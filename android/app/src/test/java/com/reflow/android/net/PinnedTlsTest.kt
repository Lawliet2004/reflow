package com.reflow.android.net

import com.reflow.android.data.ServerConnection
import okhttp3.tls.HeldCertificate
import org.junit.Assert.*
import org.junit.Test
import java.security.cert.CertificateException

class PinnedTlsTest {
    @Test fun acceptsTheExactTrustedLeafCertificate() {
        val certificate = HeldCertificate.Builder().commonName("Reflow desktop").build().certificate
        PinnedCertificateTrustManager(certificateSha256(certificate))
            .checkServerTrusted(arrayOf(certificate), "RSA")
    }

    @Test fun rejectsADifferentCertificateEvenWithTheSameDesktopName() {
        val trusted = HeldCertificate.Builder().commonName("Reflow desktop").build().certificate
        val stranger = HeldCertificate.Builder().commonName("Reflow desktop").build().certificate
        assertThrows(CertificateException::class.java) {
            PinnedCertificateTrustManager(certificateSha256(trusted))
                .checkServerTrusted(arrayOf(stranger), "RSA")
        }
    }

    @Test fun rejectsAnExpiredPinnedCertificate() {
        val expired = HeldCertificate.Builder().validityInterval(1, 2).build().certificate
        assertThrows(CertificateException::class.java) {
            PinnedCertificateTrustManager(certificateSha256(expired))
                .checkServerTrusted(arrayOf(expired), "RSA")
        }
    }

    @Test fun rejectsMalformedAndMissingPinsBeforeConnecting() {
        listOf("", "a".repeat(63), "G".repeat(64), "A".repeat(64), "00:11").forEach {
            assertThrows(IllegalArgumentException::class.java) { PinnedCertificateTrustManager(it) }
        }
    }

    @Test fun pinlessStoredConnectionsMustBePairedAgain() {
        assertFalse(isSecureConnection(ServerConnection("192.168.1.2", 7840, "token", "Desktop")))
        assertTrue(isSecureConnection(ServerConnection("192.168.1.2", 7840, "token", "Desktop", "a".repeat(64))))
    }

    @Test fun legacyPairLinksHaveNoTrustedIdentity() {
        assertNull(parsePairUri("reflow://pair?host=192.168.1.2&port=7840&code=123456"))
        assertNull(parsePairUri("reflow://pair?v=1&host=192.168.1.2&port=7840&code=123456&cert_sha256=${"a".repeat(64)}"))
    }
}
