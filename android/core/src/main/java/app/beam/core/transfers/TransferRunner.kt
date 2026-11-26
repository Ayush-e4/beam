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
     * [TransferState.Cancelled]. [onEnded] hears how it ended, and the error or cancellation
     * behind it if any, just before [registry] shows it ended.
     */
    fun start(
        direction: Direction,
        title: String,
        bytesTotal: Long,
        onEnded: (TransferState.Ended, Throwable?) -> Unit = { _, _ -> },
        work: suspend (ProgressListener) -> TransferState.Ended
    ): Long {
        var id = 0L
        val job = scope.launch(start = CoroutineStart.LAZY) {
            val progress = object : ProgressListener {
                override fun onProgress(bytesDone: ULong, bytesTotal: ULong) =
                    registry.progress(id, bytesDone.toLong(), bytesTotal.toLong())
            }
            var cause: Throwable? = null
            val ended = try {
                work(progress)
            } catch (e: CancellationException) {
                onEnded(TransferState.Cancelled, e)
                registry.end(id, TransferState.Cancelled)
                throw e
            } catch (e: DeviceUnreachable) {
                cause = e
                TransferState.Failed(e.message.orEmpty(), e.problem)
            } catch (e: BeamException) {
                cause = e
                TransferState.Failed(e.reason())
            } catch (e: IOException) {
                cause = e
                TransferState.Failed(e.message ?: "could not read or save the files")
            }
            onEnded(ended, cause)
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
