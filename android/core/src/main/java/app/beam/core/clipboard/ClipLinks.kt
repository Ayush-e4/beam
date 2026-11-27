package app.beam.core.clipboard

import android.content.Context
import android.util.Log
import app.beam.core.BeamException
import app.beam.core.ClipLinkInterface
import app.beam.core.ReceivedClip
import java.io.IOException
import java.util.concurrent.atomic.AtomicLong
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

/** A device whose clipboard is linked with this phone's. */
data class LinkedDevice(val id: Long, val name: String, val publicKey: ByteArray) {
    override fun equals(other: Any?): Boolean = other is LinkedDevice && other.id == id

    override fun hashCode(): Int = id.hashCode()
}

/** An item [device] copied, now on this phone's clipboard. */
data class Arrival(val device: LinkedDevice, val clip: ReceivedClip)

/**
 * The clipboard links this phone has open, one per device, outliving every screen. What a
 * linked device copies goes straight onto the phone's clipboard ([put]); [send] sends what is
 * on it to every linked device. A link that ends is dropped from [linked].
 */
class ClipLinks internal constructor(
    private val scope: CoroutineScope,
    private val put: (ReceivedClip) -> Unit,
    private val onLinked: () -> Unit,
    private val warn: (String, Throwable) -> Unit = { message, error -> Log.w(TAG, message, error) }
) {
    private class Entry(val device: LinkedDevice, val link: ClipLinkInterface)

    private val ids = AtomicLong()
    private val entries = MutableStateFlow<List<Entry>>(emptyList())
    private val devices = MutableStateFlow<List<LinkedDevice>>(emptyList())

    private val arrived = MutableSharedFlow<Arrival>(extraBufferCapacity = ARRIVALS_KEPT)

    /** The devices linked right now. */
    val linked: StateFlow<List<LinkedDevice>> = devices.asStateFlow()

    /** Each item a linked device copied, once it is on the clipboard, for showing it. */
    val arrivals: SharedFlow<Arrival> = arrived.asSharedFlow()

    /** Starts using [link] with the device called [name]: its copies land on the clipboard. */
    fun add(name: String, link: ClipLinkInterface): LinkedDevice {
        val device = LinkedDevice(ids.incrementAndGet(), name, link.peerKey())
        change { it + Entry(device, link) }
        scope.launch { receive(device, link) }
        onLinked()
        return device
    }

    /** Sends [clip] to every linked device. */
    suspend fun send(clip: OutgoingClip): SendResult {
        val now = entries.value
        if (now.isEmpty()) return SendResult.NOBODY_LINKED
        var sent = false
        var failed = false
        for (entry in now) {
            try {
                sent = sendTo(entry.link, clip) || sent
            } catch (e: BeamException) {
                warn("the link with ${entry.device.name} failed while sending", e)
                failed = true
            }
        }
        return when {
            sent -> SendResult.SENT
            failed -> SendResult.FAILED
            else -> SendResult.ALREADY_THERE
        }
    }

    /** Sends what [read] found, or says why there is nothing to send. */
    suspend fun send(read: ClipboardRead): SendResult = when (read) {
        is ClipboardRead.Found -> send(read.clip)
        ClipboardRead.Empty -> SendResult.NOTHING_COPIED
        ClipboardRead.TooLarge -> SendResult.TOO_LARGE
        ClipboardRead.Unsupported -> SendResult.UNSUPPORTED
    }

    /** Ends the link with [device]; it sees the link close. */
    fun disconnect(device: LinkedDevice) {
        val entry = entries.value.firstOrNull { it.device == device } ?: return
        entry.link.stop()
    }

    /** Ends every link. */
    fun disconnectAll() = entries.value.forEach { it.link.stop() }

    private suspend fun sendTo(link: ClipLinkInterface, clip: OutgoingClip): Boolean = when (clip) {
        is OutgoingClip.Text -> link.sendText(clip.text, clip.sensitive)
        is OutgoingClip.Image -> link.sendImage(clip.mime, clip.bytes, clip.sensitive)
    }

    /** Puts each item [device] copies on the clipboard until the link ends. */
    private suspend fun receive(device: LinkedDevice, link: ClipLinkInterface) {
        try {
            while (true) {
                val clip = link.nextClip() ?: break
                try {
                    put(clip)
                    arrived.tryEmit(Arrival(device, clip))
                } catch (e: IOException) {
                    warn("could not keep the image ${device.name} copied", e)
                }
            }
        } catch (e: BeamException) {
            warn("the clipboard link with ${device.name} ended", e)
        } finally {
            change { all -> all.filterNot { it.device == device } }
            (link as? AutoCloseable)?.close()
        }
    }

    private fun change(transform: (List<Entry>) -> List<Entry>) {
        entries.update(transform)
        devices.value = entries.value.map { it.device }
    }

    companion object {
        private const val TAG = "BeamClipLinks"
        private const val ARRIVALS_KEPT = 16

        @Volatile
        private var instance: ClipLinks? = null

        /** This process's clipboard links. */
        fun get(context: Context): ClipLinks = instance ?: synchronized(this) {
            instance ?: create(context.applicationContext).also { instance = it }
        }

        private fun create(context: Context): ClipLinks {
            val clipboard = PhoneClipboard(context)
            return ClipLinks(
                CoroutineScope(SupervisorJob() + Dispatchers.Default),
                put = clipboard::put,
                onLinked = { ClipLinkService.start(context) }
            )
        }
    }
}
