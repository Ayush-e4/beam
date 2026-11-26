package app.beam.core.history

import app.beam.core.transfers.Direction

/** How a transfer in the history ended, or where it stands if it may still continue. */
enum class Outcome {
    DONE,
    DECLINED,
    FAILED,

    /** The person stopped it. */
    CANCELLED,

    /** Cut off (the device went away, or Android stopped it); it may still continue. */
    INTERRUPTED,

    /** An interrupted send that can't continue any more (too old, or the device untrusted). */
    NOT_DELIVERED
}

/** One file of a transfer: its path inside the transfer, such as "Holiday/beach.jpg". */
data class HistoryFile(val path: String, val size: Long)

/**
 * One transfer this device sent or received. A transfer has one entry, updated in place as
 * it is interrupted and continues, so [Outcome.INTERRUPTED] later becomes [Outcome.DONE].
 */
data class HistoryEntry(
    /** As [app.beam.core.TransferSummary.transferId]; with [direction], the entry's key. */
    val transferId: String,
    val direction: Direction,
    /** The other device. */
    val deviceId: String,
    /** Null when it isn't a trusted device, so its name isn't known. */
    val deviceName: String?,
    /** What it is about, such as "photo.jpg" or "12 files". */
    val title: String,
    /** The first [TransferHistory.KEPT_FILES] files; [fileCount] says how many there were. */
    val files: List<HistoryFile>,
    val fileCount: Int,
    val totalBytes: Long,
    val outcome: Outcome,
    /** Why it ended so, such as the error or the decline reason; null when there's no more. */
    val detail: String?,
    /** When it last changed. */
    val atMillis: Long,
    /** Where received files were saved (the first [TransferHistory.KEPT_FILES]). */
    val savedPaths: List<String> = emptyList()
) {
    /** Whether [other] is the same transfer, so it replaces this entry. */
    fun sameTransfer(other: HistoryEntry): Boolean =
        transferId == other.transferId && direction == other.direction
}
