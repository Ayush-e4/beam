package app.beam.core.network

import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class PhoneHotspotTest {
    private val details = HotspotDetails("AndroidShare_1234", "k9x2pq7m", WifiSecurity.WPA, null)
    private val running = MutableStateFlow(false)

    private class FakeHotspot(override val details: HotspotDetails) : OpenHotspot {
        override val isOn = MutableStateFlow(true)
        var closed = false

        override fun close() {
            closed = true
        }
    }

    private class FakeOpener(private val details: HotspotDetails) : HotspotOpener {
        val opened = mutableListOf<FakeHotspot>()
        var refuse: HotspotProblem? = null
        var gate: CompletableDeferred<Unit>? = null

        override suspend fun open(): OpenHotspot {
            gate?.await()
            refuse?.let { throw HotspotException(it, "refused") }
            return FakeHotspot(details).also { opened += it }
        }
    }

    private val opener = FakeOpener(details)

    private fun TestScope.phoneHotspot() = PhoneHotspot(opener, running, backgroundScope, IDLE)

    @Test
    fun theButtonOpensItAndClosesIt() = runTest {
        val hotspot = phoneHotspot()
        assertEquals(HotspotState.Off(null), hotspot.state.value)
        hotspot.open()
        assertEquals(HotspotState.Opening, hotspot.state.value)
        runCurrent()
        assertEquals(HotspotState.On(details), hotspot.state.value)

        hotspot.close()
        runCurrent()
        assertEquals(HotspotState.Off(HotspotOff.CLOSED), hotspot.state.value)
        assertTrue(opener.opened.single().closed)
    }

    @Test
    fun openingTwiceOpensOnce() = runTest {
        val hotspot = phoneHotspot()
        hotspot.open()
        hotspot.open()
        runCurrent()
        hotspot.open()
        runCurrent()
        assertEquals(1, opener.opened.size)
    }

    @Test
    fun itClosesWhenNoTransferHasRunForAWhile() = runTest {
        val hotspot = phoneHotspot()
        hotspot.open()
        runCurrent()
        advanceTimeBy(IDLE - 1)
        assertEquals(HotspotState.On(details), hotspot.state.value)
        advanceTimeBy(2)
        assertEquals(HotspotState.Off(HotspotOff.IDLE), hotspot.state.value)
        assertTrue(opener.opened.single().closed)
    }

    @Test
    fun aRunningTransferKeepsItOpenAndTheWaitStartsWhenItEnds() = runTest {
        val hotspot = phoneHotspot()
        hotspot.open()
        runCurrent()
        advanceTimeBy(IDLE / 2)
        running.value = true
        advanceTimeBy(IDLE * 3)
        assertEquals(HotspotState.On(details), hotspot.state.value)

        running.value = false
        advanceTimeBy(IDLE - 1)
        assertEquals(HotspotState.On(details), hotspot.state.value)
        advanceTimeBy(2)
        assertEquals(HotspotState.Off(HotspotOff.IDLE), hotspot.state.value)
    }

    @Test
    fun androidStoppingItIsReported() = runTest {
        val hotspot = phoneHotspot()
        hotspot.open()
        runCurrent()
        opener.opened.single().isOn.value = false
        runCurrent()
        assertEquals(HotspotState.Off(HotspotOff.STOPPED_BY_ANDROID), hotspot.state.value)
        assertTrue(opener.opened.single().closed)
    }

    @Test
    fun aRefusalIsReportedAndTheButtonCanTryAgain() = runTest {
        val hotspot = phoneHotspot()
        opener.refuse = HotspotProblem.INCOMPATIBLE_MODE
        hotspot.open()
        runCurrent()
        assertEquals(HotspotState.Failed(HotspotProblem.INCOMPATIBLE_MODE), hotspot.state.value)

        opener.refuse = null
        hotspot.open()
        runCurrent()
        assertEquals(HotspotState.On(details), hotspot.state.value)
    }

    @Test
    fun closingWhileOpeningThenReopeningKeepsTheNewOne() = runTest {
        val hotspot = phoneHotspot()
        val first = CompletableDeferred<Unit>()
        opener.gate = first
        hotspot.open()
        runCurrent()
        hotspot.close()
        assertEquals(HotspotState.Off(HotspotOff.CLOSED), hotspot.state.value)

        opener.gate = null
        hotspot.open()
        first.complete(Unit)
        runCurrent()
        assertEquals(HotspotState.On(details), hotspot.state.value)
        assertEquals(1, opener.opened.size)
        assertFalse(opener.opened.single().closed)
    }

    private companion object {
        const val IDLE = 60_000L
    }
}
