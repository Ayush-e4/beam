package app.beam.util

import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Test

class ByteFormatTest {
    @Test
    fun `small values stay in bytes`() {
        assertEquals("0 B", formatBytes(0))
        assertEquals("1023 B", formatBytes(1023))
    }

    @Test
    fun `larger values scale to the right unit`() {
        assertEquals("1.5 KB", formatBytes(1536))
        assertEquals("5.0 MB", formatBytes(5L * 1024 * 1024))
        assertEquals("2.0 GB", formatBytes(2L * 1024 * 1024 * 1024))
    }

    @Test
    fun `negative values are rejected`() {
        assertThrows(IllegalArgumentException::class.java) { formatBytes(-1) }
    }
}
