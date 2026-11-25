package app.beam.core.transfers

import java.io.File
import java.io.IOException
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

class PendingSendsTest {
    @get:Rule
    val folder = TemporaryFolder()

    private val file get() = File(folder.root, "pending-sends.bin")

    private fun send(id: String, startedAt: Long, kind: SourceKind = SourceKind.PATHS) =
        PendingSend(id, "device-$id", "photo.jpg", kind, listOf("/a/b.jpg", "/a/c"), startedAt)

    @Test
    fun `sends survive a restart, oldest first`() {
        val content = send("2", 20, SourceKind.CONTENT)
            .copy(sources = listOf("content://media/external/images/media/42"))
        PendingSends(file).apply {
            add(send("1", 30))
            add(content)
        }
        assertEquals(listOf(content, send("1", 30)), PendingSends(file).all())
    }

    @Test
    fun `adding a transfer again replaces it and removing forgets it`() {
        val sends = PendingSends(file)
        sends.add(send("1", 10))
        sends.add(send("1", 10).copy(title = "2 files"))
        assertEquals(listOf("2 files"), PendingSends(file).all().map { it.title })
        assertTrue(sends.remove("1"))
        assertFalse(sends.remove("1"))
        assertEquals(emptyList<PendingSend>(), PendingSends(file).all())
    }

    @Test(expected = IOException::class)
    fun `a damaged file is an error, not an empty list`() {
        PendingSends(file).add(send("1", 10))
        file.writeBytes(file.readBytes().copyOf(12))
        PendingSends(file).all()
    }
}
