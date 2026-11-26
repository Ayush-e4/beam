package app.beam.core.transfers

import android.content.ContentResolver
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.util.Log
import app.beam.core.BeamException
import app.beam.core.Engine
import app.beam.core.OutgoingTransfer
import app.beam.core.ProgressListener
import app.beam.core.SendResult
import app.beam.core.TransferSummary
import app.beam.core.discovery.NearbyDevice
import app.beam.core.files.Inbox
import app.beam.core.files.prepareSendUris
import app.beam.core.history.TransferHistory
import app.beam.core.network.NetworkFinder
import app.beam.core.trust.TrustedDevice
import java.io.IOException
import java.net.Inet4Address
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.TimeUnit
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.flow.Flow

/**
 * Sends to trusted devices and finishes what it started. Every send is remembered in
 * [pending] until it is delivered, declined, cancelled by the person or can't be sent any
 * more; the rest (the device went away, the app was closed, Android stopped it) resume on
 * their own when the device is nearby again, so the receiver continues where it stopped.
 * Each send goes into [history]: interrupted until it continues, and not delivered if it
 * never can. Connections go over the local network [networks] finds for each address, so a
 * VPN on the phone doesn't capture them.
 */
class Sending(
    private val context: Context,
    private val engine: Engine,
    private val runner: TransferRunner,
    private val pending: PendingSends,
    history: TransferHistory,
    private val networks: NetworkFinder,
    private val clock: () -> Long = System::currentTimeMillis
) {
    private val log = TransferLog(history, clock)

    /** Transfer IDs running in this process, and when each was last tried. */
    private val running = ConcurrentHashMap.newKeySet<String>()
    private val lastTried = ConcurrentHashMap<String, Long>()

    /**
     * Sends [sources] (paths or content URIs, by [kind]) to [nearby] and returns the
     * transfer's ID in the registry.
     *
     * @throws IOException if they can't be read or remembered.
     * @throws BeamException if they can't be sent as they are.
     */
    suspend fun send(nearby: NearbyDevice, kind: SourceKind, sources: List<String>): Long {
        if (kind == SourceKind.CONTENT) sources.forEach(::keepAccess)
        val outgoing = prepare(kind, sources, resumeId = null)
        val summary = outgoing.summary()
        val send = PendingSend(
            summary.transferId,
            nearby.device.deviceId,
            summary.title(context),
            kind,
            sources,
            clock()
        )
        pending.add(send)
        return start(send, nearby, outgoing)
    }

    /**
     * Resumes remembered sends to the devices in [nearby]. Sends older than
     * [KEEP_PENDING_DAYS] days, or to devices no longer trusted, are forgotten.
     */
    fun resume(nearby: List<NearbyDevice>, trusted: List<TrustedDevice>) {
        val sends = try {
            pending.all()
        } catch (e: IOException) {
            Log.e(TAG, "remembered sends are unreadable", e)
            return
        }
        val now = clock()
        for (send in sends) {
            val tooOld = now - send.startedAtMillis > TimeUnit.DAYS.toMillis(KEEP_PENDING_DAYS)
            if (tooOld || trusted.none { it.deviceId == send.deviceId }) {
                forget(send.transferId)
                log.notDelivered(send.transferId)
                continue
            }
            val device = nearby.firstOrNull { it.device.deviceId == send.deviceId && it.compatible }
            val recently = now - (lastTried[send.transferId] ?: 0L) < RETRY_AFTER_MS
            if (device != null && send.transferId !in running && !recently) {
                start(send, device, outgoing = null)
            }
        }
    }

    /** Resumes remembered sends whenever a trusted device shows up, until cancelled. */
    suspend fun resumeWhenNearby(
        nearby: Flow<List<NearbyDevice>>,
        trusted: () -> List<TrustedDevice>
    ) {
        nearby.collect { devices -> resume(devices, trusted()) }
    }

    private fun start(send: PendingSend, nearby: NearbyDevice, outgoing: OutgoingTransfer?): Long {
        running += send.transferId
        lastTried[send.transferId] = clock()
        val peer = Peer(nearby.device.deviceId, nearby.device.name)
        var summary: TransferSummary? = outgoing?.summary()
        val onEnded = { state: TransferState.Ended, cause: Throwable? ->
            val outcome = historyOutcome(state, cause)
            log.ended(Direction.SENDING, send.transferId, peer, send.title, summary, outcome)
        }
        return runner.start(Direction.SENDING, send.title, 0, onEnded) { progress ->
            try {
                val transfer = outgoing
                    ?: prepare(send.kind, send.sources, send.transferId).also {
                        summary = it.summary()
                    }
                val ended = transfer.use { deliver(it, nearby, progress) }
                forget(send.transferId)
                ended
            } catch (e: CancellationException) {
                if (e.isByPerson()) forget(send.transferId)
                throw e
            } catch (e: BeamException.Network) {
                throw e // the device went away; resumed when it is back
            } catch (e: BeamException) {
                forget(send.transferId)
                throw e
            } catch (e: IOException) {
                forget(send.transferId) // the files can't be read any more
                throw e
            } finally {
                running -= send.transferId
            }
        }
    }

    /** Tries each of the device's addresses, IPv4 first, until one answers. */
    private suspend fun deliver(
        transfer: OutgoingTransfer,
        nearby: NearbyDevice,
        progress: ProgressListener
    ): TransferState.Ended {
        val device = nearby.device
        var unreachable: BeamException.Network? = null
        for (address in nearby.addresses.sortedBy { if (it is Inet4Address) 0 else 1 }) {
            val result = try {
                transfer.sendTo(
                    address.hostAddress ?: continue,
                    nearby.port.toUShort(),
                    device.publicKey,
                    device.cipher,
                    progress,
                    networks.networkFor(address)?.toULong()
                )
            } catch (e: BeamException.Network) {
                Log.i(TAG, "${device.name} not reachable at $address: ${e.reason()}")
                unreachable = e
                continue
            }
            return when (result) {
                SendResult.Delivered -> TransferState.Done
                is SendResult.Declined -> TransferState.Declined(result.reason)
            }
        }
        if (unreachable != null && networks.behindVpn) {
            throw BeamException.Network(vpnBlocks(device.name, unreachable.reason()))
        }
        throw unreachable ?: BeamException.Network("${device.name} has no address")
    }

    private suspend fun prepare(
        kind: SourceKind,
        sources: List<String>,
        resumeId: String?
    ): OutgoingTransfer = when (kind) {
        SourceKind.PATHS ->
            if (resumeId ==
                null
            ) {
                engine.prepareSend(sources)
            } else {
                engine.resumeSend(sources, resumeId)
            }
        SourceKind.CONTENT -> engine.prepareSendUris(context, sources.map(Uri::parse), resumeId)
    }

    private fun forget(transferId: String) {
        lastTried -= transferId
        try {
            pending.remove(transferId)
        } catch (e: IOException) {
            Log.e(TAG, "could not forget a finished send", e)
        }
    }

    /** Keeps read access to [uri] after a restart, where its provider allows it. */
    private fun keepAccess(uri: String) {
        val parsed = Uri.parse(uri)
        if (parsed.scheme != ContentResolver.SCHEME_CONTENT) return
        try {
            context.contentResolver.takePersistableUriPermission(
                parsed,
                Intent.FLAG_GRANT_READ_URI_PERMISSION
            )
        } catch (e: SecurityException) {
            // Most share-sheet grants last only while the app runs: such a send resumes
            // while Beam stays open, and after a restart fails with a clear reason.
            Log.d(TAG, "access to $parsed lasts only while Beam runs", e)
        }
    }

    companion object {
        /** Why [name] couldn't be reached while a VPN is on, keeping the engine's [detail]. */
        internal fun vpnBlocks(name: String, detail: String) =
            "A VPN on this phone is blocking $name: allow local network access in the VPN " +
                "app, or pause it ($detail)"

        /** How long a send waits to be resumed; the receiver keeps its part as long. */
        const val KEEP_PENDING_DAYS = Inbox.KEEP_UNFINISHED_DAYS
        private const val RETRY_AFTER_MS = 30_000L
        private const val TAG = "BeamSending"
    }
}
