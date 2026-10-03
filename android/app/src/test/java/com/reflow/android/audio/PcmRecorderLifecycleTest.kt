package com.reflow.android.audio

import android.media.AudioRecord
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assert.assertThrows
import org.junit.Test
import org.mockito.ArgumentMatchers.any
import org.mockito.ArgumentMatchers.anyInt
import org.mockito.Mockito
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference
import kotlin.concurrent.thread

/** Reproduces ownership bugs against the real PcmRecorder with only hardware mocked. */
class PcmRecorderLifecycleTest {
    @Test
    fun invalidMinimumBufferSizeRejectsRecording() {
        Mockito.mockStatic(AudioRecord::class.java).use { audio ->
            audio.`when`<Int> { AudioRecord.getMinBufferSize(16000, 16, 2) }
                .thenReturn(AudioRecord.ERROR_BAD_VALUE)
            Mockito.mockConstruction(AudioRecord::class.java).use {
                val recorder = PcmRecorder { }
                try {
                    assertThrows(IllegalStateException::class.java) { recorder.start() }
                } finally {
                    recorder.stop()
                }
            }
        }
    }

    @Test
    fun uninitializedMicrophoneRejectsRecordingAndReleasesHardware() {
        withHardware(configure = { hardware ->
            Mockito.`when`(hardware.state).thenReturn(AudioRecord.STATE_UNINITIALIZED)
        }) { released ->
            val recorder = PcmRecorder { }
            try {
                assertThrows(IllegalStateException::class.java) { recorder.start() }
                assertTrue("Failed initialization must release the microphone", released.await(1, TimeUnit.SECONDS))
            } finally {
                recorder.stop()
            }
        }
    }

    @Test
    fun deadMicrophoneTerminatesInsteadOfSpinningOnNegativeReads() {
        val readAttempted = CountDownLatch(1)
        val message = AtomicReference<String>()
        withHardware(configure = { hardware ->
            Mockito.`when`(hardware.read(any(ByteArray::class.java), anyInt(), anyInt()))
                .thenAnswer {
                    readAttempted.countDown()
                    Thread.sleep(1)
                    AudioRecord.ERROR_DEAD_OBJECT
                }
        }) { released ->
            val recorder = PcmRecorder(onError = { message.set(it) }, onFrame = {})
            try {
                recorder.start()
                assertTrue(readAttempted.await(1, TimeUnit.SECONDS))
                assertTrue("A terminal read failure must close the microphone", released.await(1, TimeUnit.SECONDS))
                assertTrue("A read failure must reach the UI error handler", message.get().contains("Microphone"))
            } finally {
                recorder.stop()
            }
        }
    }

    @Test
    fun startFailureReleasesHardwareAndDoesNotPreventRetry() {
        val first = AtomicBoolean(true)
        val delivered = CountDownLatch(1)
        withHardware(configure = { hardware ->
            if (first.getAndSet(false)) {
                Mockito.doThrow(SecurityException("Microphone permission revoked"))
                    .`when`(hardware).startRecording()
            } else {
                Mockito.`when`(hardware.read(any(ByteArray::class.java), anyInt(), anyInt())).thenReturn(2)
            }
        }) { released ->
            val recorder = PcmRecorder { delivered.countDown() }
            try {
                assertThrows(SecurityException::class.java) { recorder.start() }
                assertTrue(released.await(1, TimeUnit.SECONDS))
                recorder.start()
                assertTrue("A fresh attempt must acquire a fresh microphone", delivered.await(1, TimeUnit.SECONDS))
            } finally { recorder.stop() }
        }
    }

    @Test
    fun stopDoesNotReturnOrReleaseWhileAnOldFrameIsStillBeingDelivered() {
        val delivering = CountDownLatch(1)
        val finishDelivery = CountDownLatch(1)
        val stopFinished = CountDownLatch(1)
        val releaseDuringDelivery = AtomicBoolean(false)
        val firstRead = AtomicBoolean(true)
        withHardware(configure = { hardware ->
            Mockito.`when`(hardware.read(any(ByteArray::class.java), anyInt(), anyInt()))
                .thenAnswer { if (firstRead.getAndSet(false)) 2 else 0 }
            Mockito.doAnswer {
                releaseDuringDelivery.set(finishDelivery.count > 0)
                null
            }.`when`(hardware).release()
        }) { _ ->
            val recorder = PcmRecorder {
                delivering.countDown()
                check(finishDelivery.await(5, TimeUnit.SECONDS))
            }
            var stopping: Thread? = null
            try {
                recorder.start()
                assertTrue(delivering.await(1, TimeUnit.SECONDS))
                stopping = thread(name = "test-stop") {
                    recorder.stop()
                    stopFinished.countDown()
                }
                assertFalse("stop must drain the old session before a new one can start", stopFinished.await(100, TimeUnit.MILLISECONDS))
                assertFalse("Hardware must remain owned until frame delivery completes", releaseDuringDelivery.get())
                finishDelivery.countDown()
                assertTrue(stopFinished.await(1, TimeUnit.SECONDS))
            } finally {
                finishDelivery.countDown()
                stopping?.join(2000)
                recorder.stop()
            }
        }
    }

    private fun withHardware(configure: (AudioRecord) -> Unit = {}, test: (CountDownLatch) -> Unit) {
        Mockito.mockStatic(AudioRecord::class.java).use { audio ->
            audio.`when`<Int> { AudioRecord.getMinBufferSize(16000, 16, 2) }.thenReturn(3200)
            val released = CountDownLatch(1)
            Mockito.mockConstruction(AudioRecord::class.java) { hardware, _ ->
                Mockito.`when`(hardware.state).thenReturn(AudioRecord.STATE_INITIALIZED)
                Mockito.`when`(hardware.recordingState).thenReturn(AudioRecord.RECORDSTATE_RECORDING)
                Mockito.doAnswer { released.countDown(); null }.`when`(hardware).release()
                configure(hardware)
            }.use {
                test(released)
            }
        }
    }
}
