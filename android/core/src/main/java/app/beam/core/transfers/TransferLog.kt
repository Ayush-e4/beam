package app.beam.core.transfers

import android.util.Log
import app.beam.core.BeamException
import app.beam.core.TransferSummary
import app.beam.core.history.HistoryEntry
import app.beam.core.history.HistoryFile
import app.beam.core.history.Outcome
import app.beam.core.history.TransferHistory
import java.io.IOException

/** The other side of a transfer, as the history names it. */
internal data class Peer(val deviceId: String, val name: String?)

/**
 * What the history says about a transfer that ended in [state] because of [cause]: a link that
 * dropped, or Android stopping it, is [Outcome.INTERRUPTED] (it may still continue), while the
 * person stopping it is [Outcome.CANCELLED]. Returns the outcome and its detail.
 */
internal fun historyOutcome(state: TransferState.Ended, cause: Throwable?): Pair<Outcome, String?> =
    when (state) {
        TransferState.Done -> Outcome.DONE to null
        is TransferState.Declined -> Outcome.DECLINED to state.reason.name
        TransferState.Cancelled ->
            if (cause?.isByPerson() == true) {
                Outcome.CANCELLED to null
            } else {
                Outcome.INTERRUPTED to null
            }
        is TransferState.Failed ->
            if (cause is BeamException.Network) {
                Outcome.INTERRUPTED to state.reason
            } else {
                Outcome.FAILED to state.reason
            }
    }

/**
 * Writes transfers into [history]. The history is a record, not part of the transfer: if it
 * can't be saved that is logged, and the transfer is unaffected.
 */
internal class TransferLog(
    private val history: TransferHistory,
    private val clock: () -> Long = System::currentTimeMillis
) {
    /**
     * Records how a transfer ended. [summary] is null when the files weren't read (the earlier
     * entry keeps them); [selection] lists the item IDs taken, empty for all.
     */
    fun ended(
        direction: Direction,
        transferId: String,
        peer: Peer,
        title: String,
        summary: TransferSummary?,
        outcome: Pair<Outcome, String?>,
        selection: List<UInt> = emptyList(),
        saved: List<String> = emptyList()
    ) {
        val items = summary?.items.orEmpty()
            .filter { selection.isEmpty() || it.itemId in selection }
        val entry = HistoryEntry(
            transferId = transferId,
            direction = direction,
            deviceId = peer.deviceId,
            deviceName = peer.name,
            title = title,
            files = items.take(TransferHistory.KEPT_FILES)
                .map { HistoryFile(it.path, it.size.toLong()) },
            fileCount = items.size,
            totalBytes = items.sumOf { it.size.toLong() },
            outcome = outcome.first,
            detail = outcome.second,
            atMillis = clock(),
            savedPaths = saved
        )
        write("record a transfer") { history.record(entry) }
    }

    /** A send that will never continue. */
    fun notDelivered(transferId: String) {
        write("mark a send not delivered") { history.notDelivered(transferId, clock()) }
    }

    private fun write(what: String, change: () -> Unit) {
        try {
            change()
        } catch (e: IOException) {
            Log.e(TAG, "could not $what in the history", e)
        }
    }

    private companion object {
        const val TAG = "BeamHistory"
    }
}
