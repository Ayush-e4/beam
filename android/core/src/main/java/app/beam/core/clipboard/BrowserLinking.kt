package app.beam.core.clipboard

import app.beam.core.BrowserPairing
import app.beam.core.Engine
import app.beam.core.network.PhoneAddresses
import app.beam.core.trust.DeviceKind
import app.beam.core.trust.TrustedDevice
import app.beam.core.trust.TrustedDevices

/**
 * Links this phone's clipboard with the browser whose pairing code was scanned. The browser
 * can't accept connections, so the phone serves and tells it where to connect: [start] posts
 * a sealed offer in the code's room on the [signaling] service, then [PendingLink.await]
 * pairs, keeps the browser as a trusted device and opens the clipboard link.
 */
class BrowserLinking(
    private val engine: Engine,
    private val links: ClipLinks,
    private val trusted: TrustedDevices,
    /** The signaling service: `wss://…`, or `ws://…` for a local one. */
    private val signaling: String,
    private val addresses: () -> List<String> = PhoneAddresses::find
) {
    /**
     * Starts serving for the browser showing [code] and tells it where to connect.
     *
     * @throws app.beam.core.BeamException.Protocol if it isn't a Beam code, or the phone has
     * no address on a network the browser could share.
     * @throws app.beam.core.BeamException.Network if the signaling service can't be reached
     * or turned the phone away.
     */
    suspend fun start(code: String): PendingLink {
        val pairing = engine.pairWithBrowser(code)
        return try {
            pairing.announce(signaling, addresses())
            PendingLink(pairing, links, trusted)
        } catch (e: Exception) {
            pairing.stop()
            pairing.close()
            throw e
        }
    }
}

/** A browser link waiting for the browser: [await] it, or [close] to give up. */
class PendingLink internal constructor(
    private val pairing: BrowserPairing,
    private val links: ClipLinks,
    private val trusted: TrustedDevices
) : AutoCloseable {
    @Volatile
    private var done = false

    /**
     * Waits for the browser to connect and pair, introducing this phone as [phoneName], and
     * links its clipboard. Browsers holding another code are ignored.
     */
    suspend fun await(phoneName: String): LinkedDevice {
        val browser = pairing.waitForBrowser(phoneName)
        trusted.add(TrustedDevice.from(browser, System.currentTimeMillis(), DeviceKind.BROWSER))
        val link = pairing.link()
        done = true
        pairing.close() // the link keeps the server it needs
        return links.add(browser.name, link)
    }

    /** Stops waiting. Once linked, it leaves the link alone. */
    override fun close() {
        if (done) return
        done = true
        pairing.stop()
        pairing.close()
    }
}
