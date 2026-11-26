package app.beam.core.history

import app.beam.core.transfers.Direction
import java.io.File
import java.io.IOException
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

class TransferHistoryTest {
    @get:Rule
    val folder = TemporaryFolder()

    private val file get() = File(folder.root, "transfer-history.bin")

    private fun entry(
        id: String,
        at: Long,
        outcome: Outcome = Outcome.DONE,
        direction: Direction = Direction.RECEIVING
    ) = HistoryEntry(
        transferId = id,
        direction = direction,
        deviceId = "device-$id",
        deviceName = "Pixel",
        title = "Holiday",
        files = listOf(HistoryFile("Holiday/a.jpg", 10), HistoryFile("Holiday/b.jpg", 20)),
        fileCount = 2,
        totalBytes = 30,
        outcome = outcome,
        detail = if (outcome == Outcome.FAILED) "disk full" else null,
        atMillis = at,
        savedPaths = listOf("/storage/emulated/0/Download/Beam/Holiday")
    )

    @Test
    fun `entries survive a restart, newest first`() {
        val failed = entry("2", 20, Outcome.FAILED, Direction.SENDING).copy(savedPaths = listOf())
        TransferHistory(file).apply {
            record(entry("1", 10))
            record(failed)
        }
        assertEquals(listOf(failed, entry("1", 10)), TransferHistory(file).all())
    }

    @Test
    fun `a transfer has one entry, updated in place and moved to the top`() {
        val history = TransferHistory(file)
        history.record(entry("1", 10, Outcome.INTERRUPTED))
        history.record(entry("2", 20))
        history.record(entry("1", 30, Outcome.DONE))
        val all = TransferHistory(file).all()
        assertEquals(listOf("1", "2"), all.map { it.transferId })
        assertEquals(Outcome.DONE, all.first().outcome)
    }

    @Test
    fun `the same ID in the other direction is another transfer`() {
        val history = TransferHistory(file)
        history.record(entry("1", 10))
        history.record(entry("1", 20, direction = Direction.SENDING))
        assertEquals(2, history.all().size)
    }

    @Test
    fun `an update without files keeps the files it had`() {
        val history = TransferHistory(file)
        history.record(entry("1", 10, Outcome.INTERRUPTED))
        val unread = entry("1", 20, Outcome.FAILED)
            .copy(files = listOf(), fileCount = 0, totalBytes = 0, savedPaths = listOf())
        history.record(unread)
        val kept = history.all().single()
        assertEquals(Outcome.FAILED, kept.outcome)
        assertEquals(entry("1", 0).files, kept.files)
        assertEquals(30L, kept.totalBytes)
        assertEquals(entry("1", 0).savedPaths, kept.savedPaths)
    }

    @Test
    fun `only the newest are kept, with a bounded file list`() {
        val history = TransferHistory(file)
        val many = List(TransferHistory.KEPT_FILES + 50) { HistoryFile("f$it", 1) }
        repeat(TransferHistory.KEPT + 3) { history.record(entry("$it", it.toLong())) }
        history.record(entry("big", 9_999).copy(files = many, fileCount = many.size))
        val all = TransferHistory(file).all()
        assertEquals(TransferHistory.KEPT, all.size)
        assertEquals("3 oldest and 1 more dropped", "4", all.last().transferId)
        assertEquals(TransferHistory.KEPT_FILES, all.first().files.size)
        assertEquals(many.size, all.first().fileCount)
    }

    @Test
    fun `only an interrupted send becomes not delivered`() {
        val history = TransferHistory(file)
        history.record(entry("1", 10, Outcome.INTERRUPTED, Direction.SENDING))
        history.record(entry("2", 20, Outcome.DONE, Direction.SENDING))
        history.record(entry("3", 30, Outcome.INTERRUPTED, Direction.RECEIVING))
        assertTrue(history.notDelivered("1", 40))
        assertFalse(history.notDelivered("2", 40))
        assertFalse(history.notDelivered("3", 40))
        val outcomes = TransferHistory(file).all().associate { it.transferId to it.outcome }
        assertEquals(
            mapOf("1" to Outcome.NOT_DELIVERED, "2" to Outcome.DONE, "3" to Outcome.INTERRUPTED),
            outcomes
        )
    }

    @Test
    fun `entries can be removed one by one or all at once`() {
        val history = TransferHistory(file)
        history.record(entry("1", 10))
        history.record(entry("2", 20))
        assertTrue(history.remove(Direction.RECEIVING, "1"))
        assertFalse(history.remove(Direction.SENDING, "2"))
        assertEquals(listOf("2"), TransferHistory(file).all().map { it.transferId })
        history.clear()
        assertEquals(emptyList<HistoryEntry>(), TransferHistory(file).all())
    }

    @Test
    fun `a damaged file is an error, and clearing starts over`() {
        TransferHistory(file).record(entry("1", 10))
        file.writeBytes(file.readBytes().copyOf(20))
        val damaged = TransferHistory(file)
        assertThrows(IOException::class.java) { damaged.all() }
        damaged.clear()
        assertEquals(emptyList<HistoryEntry>(), TransferHistory(file).all())
    }

    @Test
    fun `an unknown format is refused`() {
        file.writeBytes(byteArrayOf(9, 0, 0, 0, 0))
        assertThrows(IOException::class.java) { TransferHistory(file).all() }
    }
}
