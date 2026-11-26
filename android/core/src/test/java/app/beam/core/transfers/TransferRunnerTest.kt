package app.beam.core.transfers

import app.beam.core.BeamException
import app.beam.core.DeclineReason
import app.beam.core.NetworkFailure
import java.io.IOException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class TransferRunnerTest {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val registry = TransferRegistry()
    private var started = 0
    private val runner = TransferRunner(scope, registry) { started++ }

    @After
    fun stop() = scope.cancel()

    private fun ended(id: Long): TransferState = runBlocking {
        withTimeout(5_000) {
            registry.transfers.first { all -> all.any { it.id == id && !it.isRunning } }
                .first { it.id == id }.state
        }
    }

    @Test
    fun `a transfer reports progress and ends done`() = runBlocking {
        val halfway = CompletableDeferred<Unit>()
        val finish = CompletableDeferred<Unit>()
        val id = runner.start(Direction.SENDING, "photo.jpg", 100) { progress ->
            progress.onProgress(50u, 100u)
            halfway.complete(Unit)
            finish.await()
            TransferState.Done
        }
        halfway.await()
        val running = registry.transfers.value.single()
        assertEquals(
            TransferStatus(id, Direction.SENDING, "photo.jpg", 50, 100, TransferState.Running),
            running
        )
        assertTrue(registry.isBusy)
        assertEquals(1, started)

        finish.complete(Unit)
        assertEquals(TransferState.Done, ended(id))
        assertFalse(registry.isBusy)
    }

    @Test
    fun `a decline is how it ended, not a failure`() {
        val id = runner.start(Direction.SENDING, "x", 1) {
            TransferState.Declined(DeclineReason.USER_DECLINED)
        }
        assertEquals(TransferState.Declined(DeclineReason.USER_DECLINED), ended(id))
    }

    @Test
    fun `engine and storage errors end it as failed with the reason`() {
        val engine = runner.start(Direction.RECEIVING, "a", 1) {
            throw BeamException.Network("the link dropped", NetworkFailure.OTHER)
        }
        val storage = runner.start(Direction.RECEIVING, "b", 1) {
            throw IOException("disk full")
        }
        assertEquals(TransferState.Failed("the link dropped"), ended(engine))
        assertEquals(TransferState.Failed("disk full"), ended(storage))
    }

    @Test
    fun `cancelling ends it as cancelled and later progress is ignored`() = runBlocking {
        val running = CompletableDeferred<Unit>()
        val one = runner.start(Direction.RECEIVING, "a", 10) {
            running.complete(Unit)
            awaitCancellation()
        }
        val two = runner.start(Direction.SENDING, "b", 10) { awaitCancellation() }
        running.await()
        registry.cancel(one)
        assertEquals(TransferState.Cancelled, ended(one))
        registry.cancelAll()
        assertEquals(TransferState.Cancelled, ended(two))

        registry.progress(one, 5, 10)
        assertEquals(0L, registry.transfers.value.first { it.id == one }.bytesDone)
    }

    @Test
    fun `the work can tell whether the person or Android cancelled it`() = runBlocking {
        val causes = java.util.concurrent.ConcurrentLinkedQueue<Boolean>()
        fun cancellable(running: CompletableDeferred<Unit>) =
            runner.start(Direction.SENDING, "x", 1) {
                try {
                    running.complete(Unit)
                    awaitCancellation()
                } catch (e: kotlin.coroutines.cancellation.CancellationException) {
                    causes += e.isByPerson()
                    throw e
                }
            }
        val first = CompletableDeferred<Unit>()
        val byPerson = cancellable(first)
        first.await()
        registry.cancel(byPerson)
        assertEquals(TransferState.Cancelled, ended(byPerson))

        val second = CompletableDeferred<Unit>()
        val byAndroid = cancellable(second)
        second.await()
        registry.cancelAll(byPerson = false)
        assertEquals(TransferState.Cancelled, ended(byAndroid))
        assertEquals(listOf(true, false), causes.toList())
    }

    @Test
    fun `onEnded hears the ending and its cause before the registry shows it`() = runBlocking {
        val heard = java.util.concurrent.ConcurrentLinkedQueue<Pair<TransferState, Throwable?>>()
        fun hear(state: TransferState.Ended, cause: Throwable?) {
            val shownEnded = registry.transfers.value.any { !it.isRunning && it.title == "x" }
            assertFalse("recorded before it shows as ended", shownEnded)
            heard += state to cause
        }
        val network = BeamException.Network("gone", NetworkFailure.OTHER)
        val failed = runner.start(Direction.SENDING, "x", 1, ::hear) { throw network }
        assertEquals(TransferState.Failed("gone"), ended(failed))
        registry.clearFinished()
        val done = runner.start(Direction.SENDING, "x", 1, ::hear) { TransferState.Done }
        assertEquals(TransferState.Done, ended(done))
        assertEquals(
            listOf(TransferState.Failed("gone") to network, TransferState.Done to null),
            heard.toList()
        )
    }

    @Test
    fun `clearing forgets ended transfers only`() = runBlocking {
        val done = runner.start(Direction.SENDING, "a", 1) { TransferState.Done }
        ended(done)
        val still = runner.start(Direction.SENDING, "b", 1) { awaitCancellation() }
        registry.clearFinished()
        assertEquals(listOf(still), registry.transfers.value.map { it.id })
    }
}
