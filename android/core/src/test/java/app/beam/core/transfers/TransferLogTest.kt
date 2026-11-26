package app.beam.core.transfers

import app.beam.core.BeamException
import app.beam.core.DeclineReason
import app.beam.core.NetworkFailure
import app.beam.core.OfferedItem
import app.beam.core.TransferSummary
import app.beam.core.history.HistoryFile
import app.beam.core.history.Outcome
import app.beam.core.history.TransferHistory
import java.io.File
import java.io.IOException
import kotlin.coroutines.cancellation.CancellationException
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

class TransferLogTest {
    @get:Rule
    val folder = TemporaryFolder()

    @Test
    fun `endings map to what the history says`() {
        val failed = TransferState.Failed("gone")
        assertEquals(Outcome.DONE to null, historyOutcome(TransferState.Done, null))
        assertEquals(
            Outcome.DECLINED to "USER_DECLINED",
            historyOutcome(TransferState.Declined(DeclineReason.USER_DECLINED), null)
        )
        assertEquals(
            Outcome.CANCELLED to null,
            historyOutcome(TransferState.Cancelled, CancelledByPerson())
        )
        assertEquals(
            "Android stopped it",
            Outcome.INTERRUPTED to null,
            historyOutcome(TransferState.Cancelled, CancellationException("stopped"))
        )
        assertEquals(
            "the link dropped",
            Outcome.INTERRUPTED to "gone",
            historyOutcome(failed, BeamException.Network("gone", NetworkFailure.NO_ANSWER))
        )
        assertEquals(
            Outcome.FAILED to "gone",
            historyOutcome(failed, BeamException.Storage("gone"))
        )
        assertEquals(Outcome.FAILED to "gone", historyOutcome(failed, IOException("gone")))
    }

    @Test
    fun `an entry lists the files taken, and a later one without files keeps them`() {
        val history = TransferHistory(File(folder.root, "history.bin"))
        val log = TransferLog(history) { 42 }
        val summary = TransferSummary(
            "ab",
            listOf(item(1u, "Trip/a.jpg", 10u), item(2u, "Trip/b.jpg", 20u)),
            30u
        )
        val peer = Peer("device", "Pixel")
        log.ended(
            Direction.RECEIVING,
            "ab",
            peer,
            "Trip",
            summary,
            Outcome.INTERRUPTED to null,
            selection = listOf(2u)
        )
        val entry = history.all().single()
        assertEquals(listOf(HistoryFile("Trip/b.jpg", 20)), entry.files)
        assertEquals(1 to 20L, entry.fileCount to entry.totalBytes)
        assertEquals("Pixel" to 42L, entry.deviceName to entry.atMillis)

        log.ended(Direction.RECEIVING, "ab", peer, "Trip", null, Outcome.FAILED to "disk full")
        val updated = history.all().single()
        assertEquals(Outcome.FAILED to "disk full", updated.outcome to updated.detail)
        assertEquals(entry.files, updated.files)
    }

    private fun item(id: UInt, path: String, size: ULong) =
        OfferedItem(id, path, size, "image/jpeg")
}
