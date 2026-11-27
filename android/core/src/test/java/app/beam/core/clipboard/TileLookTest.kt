package app.beam.core.clipboard

import org.junit.Assert.assertEquals
import org.junit.Test

class TileLookTest {
    private fun device(id: Long, name: String) = LinkedDevice(id, name, ByteArray(32))

    @Test
    fun `the tile is off with nobody linked and names who is`() {
        assertEquals(TileLook(active = false, subtitle = null), TileLook.of(emptyList()))
        assertEquals(
            TileLook(true, "Chrome on Mac"),
            TileLook.of(listOf(device(1, "Chrome on Mac")))
        )
        assertEquals(
            TileLook(true, "Chrome on Mac +2"),
            TileLook.of(listOf(device(1, "Chrome on Mac"), device(2, "iPad"), device(3, "Work PC")))
        )
    }
}
