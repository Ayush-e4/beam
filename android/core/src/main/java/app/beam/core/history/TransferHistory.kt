package app.beam.core.history

import android.content.Context
import app.beam.core.storage.writeAtomically
import app.beam.core.transfers.Direction
import java.io.File
import java.io.IOException

/**
 * What this device sent and received, newest first, kept in [file] (not backed up). It holds
 * the newest [KEPT] transfers, one entry each, and every change is written atomically. An
 * unreadable file throws rather than silently showing an empty history; [clear] still works.
 */
class TransferHistory(private val file: File) {
    private var entries: List<HistoryEntry>? = null

    /** Every entry, newest first. */
    @Synchronized
    @Throws(IOException::class)
    fun all(): List<HistoryEntry> = loaded()

    /**
     * Records [entry] as the newest, replacing the same transfer's earlier entry. Files and
     * saved paths it doesn't know (a send that failed before reading its files) are kept from
     * the earlier entry.
     */
    @Synchronized
    @Throws(IOException::class)
    fun record(entry: HistoryEntry) {
        val all = loaded()
        val earlier = all.firstOrNull { it.sameTransfer(entry) }
        val merged = if (earlier != null && entry.fileCount == 0) {
            entry.copy(
                files = earlier.files,
                fileCount = earlier.fileCount,
                totalBytes = earlier.totalBytes,
                savedPaths = entry.savedPaths.ifEmpty { earlier.savedPaths }
            )
        } else {
            entry
        }
        save((listOf(merged.trimmed()) + all.filterNot { it.sameTransfer(entry) }).take(KEPT))
    }

    /**
     * Marks an interrupted send that will never continue as [Outcome.NOT_DELIVERED]. Returns
     * whether it changed anything: other entries are left as they are.
     */
    @Synchronized
    @Throws(IOException::class)
    fun notDelivered(transferId: String, atMillis: Long): Boolean {
        val all = loaded()
        val index = all.indexOfFirst {
            it.direction == Direction.SENDING &&
                it.transferId == transferId &&
                it.outcome == Outcome.INTERRUPTED
        }
        if (index < 0) return false
        val updated = all[index].copy(outcome = Outcome.NOT_DELIVERED, atMillis = atMillis)
        save(all.toMutableList().apply { this[index] = updated })
        return true
    }

    /** Forgets one transfer's entry; returns whether there was one. */
    @Synchronized
    @Throws(IOException::class)
    fun remove(direction: Direction, transferId: String): Boolean {
        val before = loaded()
        val after = before.filterNot { it.direction == direction && it.transferId == transferId }
        if (after.size != before.size) save(after)
        return after.size != before.size
    }

    /** Forgets every entry, even when the file couldn't be read. */
    @Synchronized
    @Throws(IOException::class)
    fun clear() = save(emptyList())

    private fun loaded(): List<HistoryEntry> = entries ?: read().also { entries = it }

    private fun read(): List<HistoryEntry> =
        if (file.exists()) HistoryFormat.decode(file.readBytes()) else emptyList()

    private fun save(updated: List<HistoryEntry>) {
        writeAtomically(file, HistoryFormat.encode(updated))
        entries = updated
    }

    /** Keeps the entry small, and within what the file format can hold. */
    private fun HistoryEntry.trimmed() = copy(
        files = files.take(KEPT_FILES),
        savedPaths = savedPaths.take(KEPT_FILES),
        detail = detail?.take(MAX_DETAIL)
    )

    companion object {
        /** How many transfers the history holds. */
        const val KEPT = 500

        /** How many files (and saved paths) each entry lists. */
        const val KEPT_FILES = 200

        private const val MAX_DETAIL = 500

        /** The app's history. */
        fun forApp(context: Context) =
            TransferHistory(File(context.noBackupFilesDir, "transfer-history.bin"))
    }
}
