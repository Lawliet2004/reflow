package com.reflow.android.audio

import android.annotation.SuppressLint
import android.media.AudioFormat
import android.media.AudioRecord
import android.media.MediaRecorder
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.concurrent.thread

class PcmRecorder(
    private val onError: (String) -> Unit = {},
    private val onFrame: (ByteArray) -> Unit,
) {
    private class Capture(val record: AudioRecord) {
        val running = AtomicBoolean(true)
        val released = AtomicBoolean(false)
        var worker: Thread? = null

        fun release() {
            if (released.compareAndSet(false, true)) {
                runCatching { record.stop() }
                record.release()
            }
        }
    }

    private var capture: Capture? = null

    @Synchronized
    @SuppressLint("MissingPermission")
    fun start() {
        if (capture?.running?.get() == true) return
        stop()
        val minimum = AudioRecord.getMinBufferSize(
            16000, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT,
        )
        check(minimum > 0) { "This microphone does not support 16 kHz audio ($minimum)" }
        val record = AudioRecord(
            MediaRecorder.AudioSource.VOICE_RECOGNITION, 16000,
            AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT,
            minimum.coerceAtLeast(3200),
        )
        val session = Capture(record)
        try {
            check(record.state == AudioRecord.STATE_INITIALIZED) { "Microphone initialization failed" }
            record.startRecording()
            check(record.recordingState == AudioRecord.RECORDSTATE_RECORDING) { "Microphone could not start" }
            capture = session
            session.worker = thread(name = "reflow-pcm") {
                try {
                    val buffer = ByteArray(3200)
                    while (session.running.get()) {
                        val size = record.read(buffer, 0, buffer.size)
                        if (!session.running.get()) break
                        check(size >= 0) { "Microphone disconnected or failed ($size). Try reconnecting it." }
                        if (size > 0) onFrame(buffer.copyOf(size)) else Thread.sleep(5)
                    }
                } catch (failure: Exception) {
                    if (session.running.getAndSet(false)) {
                        onError(failure.message ?: "Microphone capture failed")
                    }
                } finally {
                    session.running.set(false)
                    session.release()
                }
            }
        } catch (failure: Exception) {
            session.running.set(false)
            session.release()
            capture = null
            throw failure
        }
    }

    @Synchronized
    fun stop() {
        val session = capture ?: return
        session.running.set(false)
        // Stop unblocks AudioRecord.read; the worker releases after its last callback.
        runCatching { session.record.stop() }
        val worker = session.worker
        if (worker != null && worker !== Thread.currentThread()) {
            worker.join(2000)
            check(!worker.isAlive) { "Microphone capture is still stopping. Try again shortly." }
        }
        if (worker == null) session.release()
        capture = null
    }
}
