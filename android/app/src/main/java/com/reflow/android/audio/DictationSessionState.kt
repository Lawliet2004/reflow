package com.reflow.android.audio

enum class DictationPhase { Idle, Connecting, Listening, Processing }

/** Main-thread state ownership prevents a completed socket from changing a later session. */
class DictationSessionState {
    var phase = DictationPhase.Idle
        private set
    private var serial = 0L
    private var active: Long? = null

    fun begin(): Long? {
        if (active != null) return null
        active = ++serial
        phase = DictationPhase.Connecting
        return serial
    }

    fun isCurrent(id: Long) = active == id

    fun ready(id: Long): Boolean {
        if (!isCurrent(id)) return false
        if (phase == DictationPhase.Connecting) phase = DictationPhase.Listening
        return true
    }

    fun release(id: Long): Boolean {
        if (!isCurrent(id) || phase == DictationPhase.Processing) return false
        phase = DictationPhase.Processing
        return true
    }

    fun finish(id: Long): Boolean {
        if (!isCurrent(id)) return false
        active = null
        phase = DictationPhase.Idle
        return true
    }
}
