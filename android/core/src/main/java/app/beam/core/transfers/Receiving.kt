package app.beam.core.transfers

import android.content.Context
import android.util.Log
import app.beam.core.BeamException
import app.beam.core.DeclineReason
import app.beam.core.Engine
import app.beam.core.IncomingOffer
import app.beam.core.SessionCipher
import app.beam.core.TransferSummary
import app.beam.core.TrustedKeys
import app.beam.core.files.Inbox
import app.beam.core.files.acceptIntoInbox
import app.beam.core.files.continuesInInbox
import app.beam.core.history.Outcome
import app.beam.core.history.TransferHistory
import java.io.File
import java.util.concurrent.atomic.AtomicLong
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.getAndUpdate
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull

/** An offer waiting for the person to say yes or no. */
class PendingOffer internal constructor(val id: Long, internal val offer: IncomingOffer) {
    val summary: TransferSummary = offer.summary()
    val fromTrustedDevice: Boolean = offer.fromTrustedDevice()
    val peerPublicKey: ByteArray = offer.peerPublicKey()
}

/**
 * Receives while the app is open. [run] listens; transfers the person already accepted, cut
 * off earlier, continue without asking; every other offer waits in [offers] for [accept] or
 * [decline]. Accepted transfers run through [runner] into Download/Beam. Received transfers
 * are listed in [finishedList], so a sender that asks again after a restart (it missed the
 * last word) is told they arrived instead of the person being asked twice. Each transfer, and
 * each offer the person declines, goes into [history]; [deviceName] names trusted senders by
 * their public key.
 */
class Receiving(
    private val context: Context,
    private val engine: Engine,
    private val trusted: TrustedKeys,
    private val runner: TransferRunner,
    private val cipher: SessionCipher,
    history: TransferHistory,
    private val deviceName: (publicKey: ByteArray) -> String? = { null },
    private val finishedList: File = File(context.noBackupFilesDir, FINISHED_FILE)
) {
    private val log = TransferLog(history)
    private val waiting = MutableStateFlow<List<PendingOffer>>(emptyList())
    private val listeningOn = MutableStateFlow<Int?>(null)
    private val nextId = AtomicLong(1)

    /** Offers waiting for an answer, oldest first. */
    val offers: StateFlow<List<PendingOffer>> = waiting.asStateFlow()

    /** The port while listening (to announce for discovery), otherwise null. */
    val port: StateFlow<Int?> = listeningOn.asStateFlow()

    /**
     * Listens on [port] (0 picks a free one) until cancelled. Run it while the app is open;
     * offers still waiting when it stops are declined as busy, so their senders try later.
     *
     * @throws BeamException.Network if the port can't be opened.
     */
    suspend fun run(port: UShort = 0u) {
        rememberFinished()
        removeStale()
        engine.listen(port, cipher, trusted).use { listener ->
            listeningOn.value = listener.port().toInt()
            try {
                while (true) handle(listener.nextOffer())
            } finally {
                listeningOn.value = null
                withContext(NonCancellable) { declineAll(DeclineReason.BUSY) }
            }
        }
    }

    /**
     * Receives the waiting offer [id] ([selection] lists item IDs, empty for all) and returns
     * the transfer's ID, or null if it is no longer waiting.
     */
    fun accept(id: Long, selection: List<UInt> = emptyList()): Long? =
        take(id)?.let { receive(it.offer, selection) }

    /** Tells the sender of waiting offer [id] no. Returns false if it is no longer waiting. */
    suspend fun decline(id: Long, reason: DeclineReason = DeclineReason.USER_DECLINED): Boolean {
        val pending = take(id) ?: return false
        val peer = peerOf(pending.offer)
        answer(pending.offer, reason)
        if (reason == DeclineReason.USER_DECLINED) {
            val summary = pending.summary
            val outcome = Outcome.DECLINED to reason.name
            log.ended(
                Direction.RECEIVING,
                summary.transferId,
                peer,
                summary.title(context),
                summary,
                outcome
            )
        }
        return true
    }

    private suspend fun handle(offer: IncomingOffer) {
        val continues = try {
            offer.continuesInInbox(context)
        } catch (e: BeamException) {
            Log.w(TAG, "could not check for an earlier part: ${e.reason()}")
            false
        }
        if (continues) {
            receive(offer, emptyList())
        } else {
            waiting.update { it + PendingOffer(nextId.getAndIncrement(), offer) }
        }
    }

    private fun receive(offer: IncomingOffer, selection: List<UInt>): Long {
        val summary = offer.summary()
        val title = summary.title(context)
        val peer = peerOf(offer)
        var saved = emptyList<String>()
        return runner.start(
            Direction.RECEIVING,
            title,
            summary.totalBytes.toLong(),
            onEnded = { state, cause ->
                val outcome = historyOutcome(state, cause)
                log.ended(
                    Direction.RECEIVING,
                    summary.transferId,
                    peer,
                    title,
                    summary,
                    outcome,
                    selection,
                    saved
                )
            }
        ) { progress ->
            saved = offer.use { it.acceptIntoInbox(context, progress, selection) }
            TransferState.Done
        }
    }

    private fun peerOf(offer: IncomingOffer) =
        Peer(offer.peerDeviceId(), deviceName(offer.peerPublicKey()))

    private fun take(id: Long): PendingOffer? {
        val before = waiting.getAndUpdate { all -> all.filterNot { it.id == id } }
        return before.firstOrNull { it.id == id }
    }

    private suspend fun declineAll(reason: DeclineReason) {
        waiting.getAndUpdate { emptyList() }.forEach { answer(it.offer, reason) }
    }

    /** Declines [offer] and lets it go; a sender that already left needs no answer. */
    private suspend fun answer(offer: IncomingOffer, reason: DeclineReason) {
        offer.use {
            try {
                withTimeoutOrNull(ANSWER_WAIT_MS) { it.decline(reason) }
            } catch (e: BeamException) {
                Log.i(TAG, "the sender left before the answer: ${e.reason()}")
            }
        }
    }

    /**
     * Keeps the engine's list of received transfers in [finishedList]. A damaged list is
     * dropped and started again: at worst the person is asked once more about a transfer
     * that already arrived.
     */
    private suspend fun rememberFinished() {
        try {
            engine.rememberFinishedIn(finishedList.path)
        } catch (e: BeamException.Storage) {
            Log.w(TAG, "starting a new list of received transfers: ${e.reason()}")
            try {
                if (finishedList.exists() && !finishedList.delete()) {
                    Log.e(TAG, "could not remove the damaged list")
                    return
                }
                engine.rememberFinishedIn(finishedList.path)
            } catch (again: BeamException) {
                Log.e(TAG, "received transfers are remembered until Beam closes: ${again.reason()}")
            }
        }
    }

    private suspend fun removeStale() {
        try {
            Inbox.removeStale(engine, context)
        } catch (e: BeamException) {
            Log.w(TAG, "could not remove unfinished transfers: ${e.reason()}")
        }
    }

    private companion object {
        const val TAG = "BeamReceiving"
        const val ANSWER_WAIT_MS = 5_000L
        const val FINISHED_FILE = "finished-transfers.bin"
    }
}
