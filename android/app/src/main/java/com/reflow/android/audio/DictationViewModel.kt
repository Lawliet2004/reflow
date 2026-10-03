package com.reflow.android.audio

import android.app.Application
import android.content.Intent
import android.os.Handler
import android.os.Looper
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import com.reflow.android.data.ServerConnection
import com.reflow.android.net.DictationSocket
import com.reflow.android.net.ReflowClient
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

class DictationViewModel(application: Application) : AndroidViewModel(application) {
    private class Session(val id: Long) {
        var socket: DictationSocket? = null
        var recorder: PcmRecorder? = null
        var ready = false
        var finishing = false
        var timeout: Job? = null
    }

    private val lifecycle = DictationSessionState()
    private val main = Handler(Looper.getMainLooper())
    private var session: Session? = null
    var phase by mutableStateOf(DictationPhase.Idle)
        private set
    var transcript by mutableStateOf("")
        private set
    var error by mutableStateOf<String?>(null)
        private set

    fun permissionDenied() { error = "Microphone permission required. Enable it in Android settings." }

    fun start(client: ReflowClient, connection: ServerConnection, language: String, inject: Boolean): Boolean {
        val id = lifecycle.begin() ?: return false
        val owned = Session(id)
        session = owned
        phase = lifecycle.phase
        error = null
        transcript = ""
        try {
            getApplication<Application>().startForegroundService(serviceIntent())
            owned.socket = client.openStream(
                connection, language, inject,
                onPartial = { partial -> onMain(owned) { transcript = partial.fullText } },
                onFinal = { result -> onMain(owned) {
                    transcript = result.text.ifBlank { result.raw }
                    finish(owned, null, cancel = false)
                } },
                onError = { message -> onMain(owned) { finish(owned, message, cancel = true) } },
                onReady = { onMain(owned) {
                    owned.ready = true
                    lifecycle.ready(id)
                    phase = lifecycle.phase
                    if (phase == DictationPhase.Processing) {
                        runCatching { owned.socket?.stop() }.onFailure {
                            finish(owned, it.message ?: "Could not finish dictation", true)
                        }
                    } else {
                        val recorder = PcmRecorder(
                            onError = { message -> onMain(owned) { finish(owned, message, true) } },
                            onFrame = { bytes -> owned.socket?.sendPcm(bytes) },
                        )
                        owned.recorder = recorder
                        runCatching { recorder.start() }.onFailure {
                            finish(owned, it.message ?: "Microphone could not start", true)
                        }
                    }
                } },
            )
            armTimeout(owned, 20_000, "Desktop did not become ready. Check its model and phone connection.")
        } catch (failure: Exception) {
            finish(owned, failure.message ?: "Could not start dictation", true)
        }
        return lifecycle.isCurrent(id)
    }

    fun release() {
        val owned = session ?: return
        if (!lifecycle.release(owned.id)) return
        phase = lifecycle.phase
        armTimeout(owned, 90_000, "Desktop processing timed out. Your partial transcript is still available.")
        viewModelScope.launch {
            val result = withContext(Dispatchers.IO) { runCatching { owned.recorder?.stop() } }
            if (!lifecycle.isCurrent(owned.id)) return@launch
            result.onFailure { finish(owned, it.message ?: "Microphone could not stop", true) }
            if (result.isSuccess) {
                owned.recorder = null
                getApplication<Application>().stopService(serviceIntent())
            }
            if (result.isSuccess && owned.ready) {
                runCatching { owned.socket?.stop() }.onFailure { finish(owned, it.message, true) }
            }
        }
    }

    fun cancel() {
        session?.let { finish(it, null, true) }
    }

    private fun onMain(owned: Session, action: () -> Unit) {
        main.post { if (lifecycle.isCurrent(owned.id) && !owned.finishing) action() }
    }

    private fun armTimeout(owned: Session, milliseconds: Long, message: String) {
        owned.timeout?.cancel()
        owned.timeout = viewModelScope.launch {
            delay(milliseconds)
            if (lifecycle.isCurrent(owned.id) && phase != DictationPhase.Listening) finish(owned, message, true)
        }
    }

    private fun finish(owned: Session, message: String?, cancel: Boolean) {
        if (!lifecycle.isCurrent(owned.id) || owned.finishing) return
        owned.finishing = true
        lifecycle.release(owned.id)
        phase = lifecycle.phase
        owned.timeout?.cancel()
        if (cancel) owned.socket?.cancel() else owned.socket?.close()
        owned.socket = null
        val recorder = owned.recorder
        // Cleanup also runs after onCleared, when viewModelScope has already been cancelled.
        error = message
        val complete = {
            if (lifecycle.finish(owned.id)) {
                owned.recorder = null
                getApplication<Application>().stopService(serviceIntent())
                session = null
                phase = lifecycle.phase
            }
        }
        if (recorder != null) kotlin.concurrent.thread(name = "reflow-pcm-cleanup") {
            // A failed bounded stop still owns live hardware. Keep the owner
            // and retry cleanup before admitting another recorder.
            while (true) {
                val stopped = runCatching { recorder.stop() }
                if (stopped.isSuccess) {
                    main.post { complete() }
                    break
                }
                main.post {
                    if (lifecycle.isCurrent(owned.id) && error == null) {
                        error = stopped.exceptionOrNull()?.message ?: "Microphone could not stop"
                    }
                }
                Thread.sleep(100)
            }
        } else complete()
    }

    private fun serviceIntent() = Intent(getApplication<Application>(), DictationService::class.java)

    override fun onCleared() {
        cancel()
        super.onCleared()
    }
}
