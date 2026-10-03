package com.reflow.android.audio

import org.junit.Assert.*
import org.junit.Test

class DictationSessionStateTest {
    @Test fun releaseDuringConnectionRemainsProcessingWhenReadyArrives() {
        val state = DictationSessionState()
        val id = state.begin()!!
        assertTrue(state.release(id))
        assertTrue(state.ready(id))
        assertEquals(DictationPhase.Processing, state.phase)
    }

    @Test fun newPressCannotReplaceAnUnfinishedSession() {
        val state = DictationSessionState()
        val id = state.begin()!!
        state.ready(id)
        state.release(id)
        assertNull(state.begin())
        assertTrue(state.finish(id))
        assertNotNull(state.begin())
    }

    @Test fun delayedOldCallbackCannotFinishOrRestartTheNewSession() {
        val state = DictationSessionState()
        val old = state.begin()!!
        state.finish(old)
        val current = state.begin()!!
        assertFalse(state.ready(old))
        assertFalse(state.finish(old))
        assertTrue(state.isCurrent(current))
        assertEquals(DictationPhase.Connecting, state.phase)
    }
}
