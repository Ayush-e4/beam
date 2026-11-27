package app.beam.core.clipboard

import android.util.Log
import app.beam.core.BeamException
import app.beam.core.ClipLinkInterface
import app.beam.core.Engine
import app.beam.core.network.PhoneAddresses
import app.beam.core.trust.TrustedDevice
import app.beam.core.trust.TrustedDevices
import java.io.IOException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch

/**
 * Meets this phone's trusted browsers again without a pairing code (`protocol.md`, Meeting
 * again). While held, it waits for each trusted browser in their signaling room and links the
 * clipboard of any that comes, again after each link ends. Beam holds it while it is open
 * ([hold]) and for a while after the tile is tapped ([holdFor]).
 */
class BrowserMeetings internal constructor(
    private val trusted: TrustedDevices,
    private val links: ClipLinks,
    private val scope: CoroutineScope,
    /** Waits in the room for [TrustedDevice] and returns its link. */
    private val meet: suspend (TrustedDevice) -> ClipLinkInterface,
    private val retryMillis: Long = RETRY_MS,
    private val warn: (String, Throwable) -> Unit = { message, error -> Log.w(TAG, message, error) }
) {
    /**
     * Meets trusted browsers through the signaling service at [signaling] (`wss://…`, or
     * `ws://…` for a local one), offering the phone's current [addresses].
     */
    constructor(
        engine: Engine,
        trusted: TrustedDevices,
        links: ClipLinks,
        signaling: String,
        scope: CoroutineScope,
        addresses: () -> List<String> = PhoneAddresses::find
    ) : this(trusted, links, scope, meet = { browser ->
        engine.meetBrowser(browser.publicKey, browser.cipher, signaling, addresses())
            .use { it.meet() }
    })

    private var holders = 0
    private var job: Job? = null
    private val active = MutableStateFlow(false)

    /** Whether the phone is waiting for its trusted browsers now. */
    val waiting: StateFlow<Boolean> = active.asStateFlow()

    /** Keeps waiting until the returned handle is closed (and nothing else holds it). */
    @Synchronized
    fun hold(): AutoCloseable {
        holders++
        if (job == null) {
            job = scope.launch { meetAll() }
            active.value = true
        }
        var closed = false
        return AutoCloseable {
            synchronized(this) {
                if (!closed) {
                    closed = true
                    release()
                }
            }
        }
    }

    /** Keeps waiting for [millis], as after a tap on the tile. */
    fun holdFor(millis: Long) {
        val held = hold()
        scope.launch {
            try {
                delay(millis)
            } finally {
                held.close()
            }
        }
    }

    @Synchronized
    private fun release() {
        holders--
        if (holders == 0) {
            job?.cancel()
            job = null
            active.value = false
        }
    }

    private suspend fun meetAll() {
        val browsers = try {
            trusted.browsers()
        } catch (e: IOException) {
            warn("trusted devices unreadable; not waiting for browsers", e)
            return
        }
        coroutineScope { browsers.forEach { launch { keepMeeting(it) } } }
    }

    private suspend fun keepMeeting(browser: TrustedDevice) {
        while (true) {
            val isThis = { device: LinkedDevice ->
                device.publicKey.contentEquals(browser.publicKey)
            }
            links.linked.first { devices -> devices.none(isThis) } // one link per browser
            try {
                links.add(browser.name, meet(browser))
            } catch (e: BeamException) {
                warn("couldn't wait for ${browser.name}; trying again", e)
                delay(retryMillis)
            }
        }
    }

    private companion object {
        const val TAG = "BeamMeetings"
        const val RETRY_MS = 5_000L
    }
}
