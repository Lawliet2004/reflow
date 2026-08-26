package com.reflow.android.audio

import android.annotation.SuppressLint
import android.media.AudioFormat
import android.media.AudioRecord
import android.media.MediaRecorder
import kotlin.concurrent.thread

class PcmRecorder(
    private val onFrame: (ByteArray) -> Unit,
) {
    @Volatile private var running = false
    private var record: AudioRecord? = null
    private var worker: Thread? = null

    @SuppressLint("MissingPermission")
    fun start() {
        if (running) return
        val sampleRate = 16000
        val min = AudioRecord.getMinBufferSize(
            sampleRate,
            AudioFormat.CHANNEL_IN_MONO,
            AudioFormat.ENCODING_PCM_16BIT,
        )
        val recorder = AudioRecord(
            MediaRecorder.AudioSource.VOICE_RECOGNITION,
            sampleRate,
            AudioFormat.CHANNEL_IN_MONO,
            AudioFormat.ENCODING_PCM_16BIT,
            min.coerceAtLeast(3200),
        )
        record = recorder
        running = true
        recorder.startRecording()
        worker = thread(name = "reflow-pcm") {
            val buf = ByteArray(3200)
            while (running) {
                val n = recorder.read(buf, 0, buf.size)
                if (n > 0) onFrame(buf.copyOf(n))
            }
        }
    }

    fun stop() {
        running = false
        try {
            record?.stop()
        } catch (_: Exception) {
        }
        record?.release()
        record = null
        worker = null
    }
}
