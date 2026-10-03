package com.reflow.android.net

import com.reflow.android.data.ServerConnection
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import okhttp3.mockwebserver.SocketPolicy
import okhttp3.tls.HandshakeCertificates
import okhttp3.tls.HeldCertificate
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okio.ByteString
import org.json.JSONObject
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertThrows
import org.junit.Test
import java.util.concurrent.TimeUnit
import java.util.concurrent.CountDownLatch
import java.util.concurrent.atomic.AtomicReference

class ReflowClientTest {
    @Test fun cancellingPairingDoesNotWaitForASilentDesktop() = runBlocking {
        val (server, pin) = secureDesktop()
        server.use { desktop ->
            desktop.enqueue(MockResponse().setSocketPolicy(SocketPolicy.NO_RESPONSE))
            val request = launch(Dispatchers.IO) {
                ReflowClient().pair("127.0.0.1", desktop.port, "123456", "Test", pin)
            }
            try {
                assertNotNull(desktop.takeRequest(2, TimeUnit.SECONDS))
                withTimeout(2000) { request.cancelAndJoin() }
            } finally {
                request.cancel()
            }
        }
    }

    @Test fun pairingRejectsAnInvalidReturnedPort() {
        val (server, pin) = secureDesktop()
        server.use { desktop ->
            desktop.enqueue(MockResponse().setBody("""{"token":"test","port":99999}"""))
            assertThrows(IllegalArgumentException::class.java) {
                runBlocking { ReflowClient().pair("127.0.0.1", desktop.port, "123456", "Test", pin) }
            }
        }
    }

    @Test fun unsuccessfulDesktopPasteIsSurfacedForTranscriptRecovery() {
        val (server, pin) = secureDesktop()
        server.use { desktop ->
            desktop.enqueue(MockResponse().setBody("""{"pasted":false,"fallback_copy":true,"paste_chord":"Ctrl+V"}"""))
            val connection = ServerConnection("127.0.0.1", desktop.port, "test", "Desktop", pin)
            assertThrows(IllegalStateException::class.java) {
                runBlocking { ReflowClient().inject(connection, "Keep this transcript") }
            }
        }
    }

    @Test fun trustedDesktopUsesHttpsAndKeepsThePinInThePairedIdentity() = runBlocking {
        val (server, pin) = secureDesktop()
        server.use { desktop ->
            desktop.enqueue(MockResponse().setBody("""{"token":"test","server_name":"Desktop"}"""))
            val paired = ReflowClient().pair("127.0.0.1", desktop.port, "123456", "Test", pin)
            org.junit.Assert.assertEquals(pin, paired.certificateSha256)
            val request = desktop.takeRequest(2, TimeUnit.SECONDS)!!
            org.junit.Assert.assertNotNull(request.handshake)
            org.junit.Assert.assertEquals("/v1/pair", request.path)
        }
    }

    @Test fun successfulDesktopPasteUsesTheActualEndpointResponse() = runBlocking {
        val (server, pin) = secureDesktop()
        server.use { desktop ->
            desktop.enqueue(MockResponse().setBody("""{"pasted":true,"fallback_copy":false,"paste_chord":"Ctrl+V"}"""))
            ReflowClient().inject(ServerConnection("127.0.0.1", desktop.port, "test", "Desktop", pin), "Paste this transcript")
        }
    }

    @Test fun certificateMismatchRejectsBeforeSendingPairingCredentials() {
        val (server, _) = secureDesktop()
        server.use { desktop ->
            assertThrows(java.io.IOException::class.java) {
                runBlocking { ReflowClient().pair("127.0.0.1", desktop.port, "123456", "Test", "a".repeat(64)) }
            }
            org.junit.Assert.assertEquals(0, desktop.requestCount)
        }
    }

    @Test fun encryptedAudioStreamStartsFinishesAndClosesWithThePinnedIdentity() {
        val (server, pin) = secureDesktop()
        server.use { desktop ->
            val ready = CountDownLatch(1)
            val audio = CountDownLatch(1)
            val final = CountDownLatch(1)
            val error = AtomicReference<String>()
            desktop.enqueue(MockResponse().withWebSocketUpgrade(object : WebSocketListener() {
                override fun onMessage(webSocket: WebSocket, text: String) {
                    when (JSONObject(text).getString("type")) {
                        "start" -> webSocket.send("""{"type":"ready"}""")
                        "stop" -> webSocket.send("""{"type":"final","text":"Retained transcript","raw":"Retained transcript","language":"en"}""")
                    }
                }

                override fun onMessage(webSocket: WebSocket, bytes: ByteString) { audio.countDown() }
                override fun onClosing(webSocket: WebSocket, code: Int, reason: String) { webSocket.close(code, reason) }
            }))
            val socket = ReflowClient().openStream(
                ServerConnection("127.0.0.1", desktop.port, "test", "Desktop", pin), "auto", false,
                onPartial = {}, onFinal = { final.countDown() }, onError = { error.set(it) }, onReady = { ready.countDown() },
            )
            try {
                org.junit.Assert.assertTrue(ready.await(2, TimeUnit.SECONDS))
                socket.sendPcm(byteArrayOf(1, 2))
                org.junit.Assert.assertTrue(audio.await(2, TimeUnit.SECONDS))
                socket.stop()
                org.junit.Assert.assertTrue(final.await(2, TimeUnit.SECONDS))
                org.junit.Assert.assertNull(error.get())
                org.junit.Assert.assertNotNull(desktop.takeRequest(2, TimeUnit.SECONDS)!!.handshake)
            } finally { socket.close() }
        }
    }

    @Test fun pairingDoesNotFollowARedirectToCleartext() {
        val (server, pin) = secureDesktop()
        server.use { desktop ->
            MockWebServer().use { cleartext ->
                cleartext.start()
                desktop.enqueue(MockResponse().setResponseCode(302).setHeader("Location", cleartext.url("/v1/pair")))
                assertThrows(IllegalStateException::class.java) {
                    runBlocking { ReflowClient().pair("127.0.0.1", desktop.port, "123456", "Test", pin) }
                }
                org.junit.Assert.assertEquals(0, cleartext.requestCount)
            }
        }
    }

    private fun secureDesktop(): Pair<MockWebServer, String> {
        // No IP subjectAltName: the pairing pin, rather than a changing LAN IP, is the identity.
        val certificate = HeldCertificate.Builder().commonName("Reflow desktop").build()
        val tls = HandshakeCertificates.Builder().heldCertificate(certificate).build()
        val desktop = MockWebServer()
        desktop.useHttps(tls.sslSocketFactory(), false)
        desktop.start()
        return desktop to certificateSha256(certificate.certificate)
    }
}
