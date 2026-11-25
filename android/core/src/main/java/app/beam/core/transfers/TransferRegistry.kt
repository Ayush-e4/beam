package app.beam.core.transfers

import app.beam.core.DeclineReason
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update

enum class Direction { SENDING, RECEIVING }

/** Why a transfer was cancelled when the person did it, rather than Android or the app. */
class CancelledByPerson : CancellationException("cancelled by the person")

/** Whether this cancellation came from the person. */
internal fun Throwable.isByPerson(): Boolean =
    generateSequence(this) { it.cause }.any { it is CancelledByPerson }

/** Where a transfer is. Every transfer starts [Running] and ends in exactly one other state. */
sealed interface TransferState {
    data object Running : TransferState

    /** How a transfer ended. */
    sealed interface Ended : TransferState

    data object Done : Ended
    data class Declined(val reason: DeclineReason) : Ended
    data class Failed(val reason: String) : Ended

    /** Stopped on purpose (by the person, or by Android); the next attempt resumes. */
    data object Cancelled : Ended
}

/** One transfer, as the notification and the screens show it. */
data class TransferStatus(
    val id: Long,
    val direction: Direction,
    /** What it is about, such as "photo.jpg" or "12 files". */
    val title: String,
    val bytesDone: Long,
    val bytesTotal: Long,
    val state: TransferState
) {
    val isRunning: Boolean get() = state == TransferState.Running
}

/**
 * The transfers of this app process: the running ones, and ended ones until cleared. All
 * methods are thread-safe; [transfers] always holds a consistent list.
 */
class TransferRegistry {
    private val list = MutableStateFlow<List<TransferStatus>>(emptyList())
    private val jobs = HashMap<Long, Job>()
    private var nextId = 1L

    val transfers: StateFlow<List<TransferStatus>> = list.asStateFlow()

    /** Whether any transfer is still running. */
    val isBusy: Boolean get() = list.value.any { it.isRunning }

    /** The person stops [id]: it ends as [TransferState.Cancelled] and isn't resumed. */
    fun cancel(id: Long) {
        synchronized(jobs) { jobs[id] }?.cancel(CancelledByPerson())
    }

    /**
     * Stops every running transfer. [byPerson] is false when Android stops them: remembered
     * sends then resume later, while ones the person cancelled are forgotten.
     */
    fun cancelAll(byPerson: Boolean = true) {
        synchronized(jobs) { jobs.values.toList() }.forEach { job ->
            job.cancel(if (byPerson) CancelledByPerson() else null)
        }
    }

    /** Forgets ended transfers, once the person has seen them. */
    fun clearFinished() = list.update { all -> all.filter { it.isRunning } }

    internal fun add(direction: Direction, title: String, bytesTotal: Long, job: Job): Long {
        val id = synchronized(jobs) {
            val id = nextId++
            jobs[id] = job
            id
        }
        val status = TransferStatus(id, direction, title, 0, bytesTotal, TransferState.Running)
        list.update { it + status }
        return id
    }

    internal fun progress(id: Long, bytesDone: Long, bytesTotal: Long) = change(id) { status ->
        if (status.isRunning) {
            status.copy(
                bytesDone = bytesDone,
                bytesTotal = bytesTotal
            )
        } else {
            status
        }
    }

    internal fun end(id: Long, state: TransferState.Ended) {
        synchronized(jobs) { jobs.remove(id) }
        change(id) { status -> if (status.isRunning) status.copy(state = state) else status }
    }

    private fun change(id: Long, edit: (TransferStatus) -> TransferStatus) =
        list.update { all -> all.map { if (it.id == id) edit(it) else it } }
}
