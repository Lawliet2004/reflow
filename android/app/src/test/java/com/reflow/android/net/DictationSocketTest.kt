package com.reflow.android.net

import okhttp3.WebSocket
import okio.ByteString.Companion.toByteString
import org.junit.Assert.assertThrows
import org.junit.Test
import org.mockito.Mockito
import java.io.IOException

class DictationSocketTest {
    @Test fun rejectedPcmFrameSurfacesADisconnectedSocket() {
        val wire = Mockito.mock(WebSocket::class.java)
        Mockito.`when`(wire.send(byteArrayOf(1, 2).toByteString())).thenReturn(false)
        assertThrows(IOException::class.java) { DictationSocket(wire).sendPcm(byteArrayOf(1, 2)) }
    }

    @Test fun slowNetworkStopsAudioInsteadOfGrowingAnUnboundedQueue() {
        val wire = Mockito.mock(WebSocket::class.java)
        Mockito.`when`(wire.queueSize()).thenReturn(1024L * 1024)
        Mockito.`when`(wire.send(byteArrayOf(1, 2).toByteString())).thenReturn(true)
        assertThrows(IOException::class.java) { DictationSocket(wire).sendPcm(byteArrayOf(1, 2)) }
    }
}
