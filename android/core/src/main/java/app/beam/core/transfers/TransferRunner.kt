package app.beam.core.transfers

import app.beam.core.BeamException
import app.beam.core.ProgressListener
import java.io.IOException
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.launch

/**
 * Runs transfers in [scope], which should live as long as the app process (not a screen), and
 * keeps [registry] up to date. [onStarted] runs whenever a transfer starts: the hook that keeps
 * the process alive while transfers run.
 */
class TransferRunner(
    private val scope: CoroutineScope,
    private val registry: TransferRegistry,
    private val onStarted: () -> Unit
) {
    /**
     * Starts a transfer and returns its ID. [work] does it, reporting to the listener it is
     * given, and returns how it ended ([TransferState.Done] or [TransferState.Declined]).
     * Engine and storage errors end it as [TransferState.Failed]; cancelling ends it as
     * [TransferState.Cancelled].
     */
    fun start(
        direction: Direction,
        title: String,
        bytesTotal: Long,
        work: suspend (ProgressListener) -> TransferState.Ended
    ): Long {
        var id = 0L
        val job = scope.launch(start = CoroutineStart.LAZY) {
            val progress = object : ProgressListener {
                override fun onProgress(bytesDone: ULong, bytesTotal: ULong) =
                    registry.progress(id, bytesDone.toLong(), bytesTotal.toLong())
            }
            val ended = try {
                work(progress)
            } catch (e: CancellationException) {
                registry.end(id, TransferState.Cancelled)
                throw e
            } catch (e: BeamException) {
                TransferState.Failed(e.reason())
            } catch (e: IOException) {
                TransferState.Failed(e.message ?: "could not read or save the files")
            }
            registry.end(id, ended)
        }
        id = registry.add(direction, title, bytesTotal, job)
        onStarted()
        job.start()
        return id
    }
}

/** The engine's explanation, without UniFFI's "detail=" wrapping. */
internal fun BeamException.reason(): String = when (this) {
    is BeamException.Network -> detail
    is BeamException.Storage -> detail
    is BeamException.Permission -> detail
    is BeamException.Protocol -> detail
    is BeamException.Crypto -> detail
    is BeamException.Cancelled -> detail
    is BeamException.Internal -> detail
}
