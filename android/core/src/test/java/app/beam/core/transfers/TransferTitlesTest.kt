package app.beam.core.transfers

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class TransferTitlesTest {
    @Test
    fun `one file goes by its name`() = assertEquals("photo.jpg", singleName(listOf("photo.jpg")))

    @Test
    fun `files in one folder go by the folder`() =
        assertEquals("Holiday", singleName(listOf("Holiday/a.jpg", "Holiday/b/c.jpg")))

    @Test
    fun `separate files have no single name`() {
        assertNull(singleName(listOf("a.jpg", "b.jpg")))
        assertNull(singleName(emptyList()))
    }
}
