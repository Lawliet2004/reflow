package com.reflow.android.audio

import android.app.Application
import android.os.Handler
import com.reflow.android.data.ServerConnection
import com.reflow.android.net.DictationSocket
import com.reflow.android.net.ReflowClient
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.setMain
import org.junit.Assert.*
import org.junit.Test
import org.mockito.ArgumentMatchers.any
import org.mockito.ArgumentMatchers.anyBoolean
import org.mockito.ArgumentMatchers.anyString
import org.mockito.Mockito
import java.util.concurrent.ConcurrentLinkedQueue
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference

@OptIn(ExperimentalCoroutinesApi::class)
class DictationViewModelLifecycleTest {
    private fun <T> matchAny(): T = Mockito.any<T>()
    @Test fun failedReleaseKeepsTheMicrophoneOwnerUntilCleanupSucceeds() =
        failedStopKeepsOwner { it.release() }

    @Test fun failedCancelKeepsTheMicrophoneOwnerUntilCleanupSucceeds() =
        failedStopKeepsOwner { it.cancel() }

    private fun failedStopKeepsOwner(stop: (DictationViewModel) -> Unit) {
        val dispatcher = StandardTestDispatcher()
        Dispatchers.setMain(dispatcher)
        val posted = ConcurrentLinkedQueue<Runnable>()
        val hardwareFree = CountDownLatch(1)
        val failed = CountDownLatch(1)
        val ready = AtomicReference<() -> Unit>()
        val application = Mockito.mock(Application::class.java)
        val client = Mockito.mock(ReflowClient::class.java)
        val socket = Mockito.mock(DictationSocket::class.java)
        val connection = ServerConnection("127.0.0.1", 7840, "test", "Desktop", "a".repeat(64))
        Mockito.doAnswer { call ->
            ready.set(call.getArgument(6))
            socket
        }.`when`(client).openStream(matchAny(), anyString(), anyBoolean(), matchAny(), matchAny(), matchAny(), matchAny())
        fun drain() {
            dispatcher.scheduler.runCurrent()
            while (true) (posted.poll() ?: break).run()
        }
        fun await(message: String, predicate: () -> Boolean) {
            val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(5)
            while (!predicate() && System.nanoTime() < deadline) {
                drain()
                Thread.sleep(5)
            }
            drain()
            assertTrue(message, predicate())
        }
        try {
            Mockito.mockConstruction(Handler::class.java) { handler, _ ->
                Mockito.`when`(handler.post(any(Runnable::class.java))).thenAnswer {
                    posted.add(it.getArgument(0))
                    true
                }
            }.use {
                Mockito.mockConstruction(PcmRecorder::class.java) { recorder, _ ->
                    Mockito.doAnswer {
                        if (hardwareFree.count > 0) {
                            failed.countDown()
                            throw IllegalStateException("Microphone capture is still stopping. Try again shortly.")
                        }
                        null
                    }.`when`(recorder).stop()
                }.use { recorders ->
                    val session = DictationViewModel(application)
                    assertTrue(session.start(client, connection, "auto", false))
                    ready.get().invoke()
                    drain()
                    assertEquals(DictationPhase.Listening, session.phase)
                    stop(session)
                    await("The failed stop must be reported") { failed.count == 0L && session.error != null }
                    assertEquals("Capture still owns the microphone", DictationPhase.Processing, session.phase)
                    assertFalse("A new session must not acquire another microphone", session.start(client, connection, "auto", false))
                    assertEquals(1, recorders.constructed().size)
                    hardwareFree.countDown()
                    await("Cleanup must eventually release the session owner") { session.phase == DictationPhase.Idle }
                    assertTrue(session.start(client, connection, "auto", false))
                    session.cancel()
                    drain()
                }
            }
        } finally {
            hardwareFree.countDown()
            drain()
            Dispatchers.resetMain()
        }
    }
}
