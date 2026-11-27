package app.beam.core.clipboard

import app.beam.core.BrowserPairing
import app.beam.core.Engine
import app.beam.core.network.PhoneAddresses
import app.beam.core.trust.TrustedDevice
import app.beam.core.trust.TrustedDevices

/**
 * Links this phone's clipboard with the browser whose pairing code was scanned. The browser
 * can't accept connections, so the phone serves and tells it where to connect: [start] gives
 * the sealed offer to post in the code's signaling room, then [PendingLink.await] pairs,
 * keeps the browser as a trusted device and opens the clipboard link.
 */
class BrowserLinking(
    private val engine: Engine,
    private val links: ClipLinks,
    private val trusted: TrustedDevices,
    private val addresses: () -> List<String> = PhoneAddresses::find
) {
    /**
     * Starts serving for the browser showing [code].
     *
     * @throws app.beam.core.BeamException.Protocol if it isn't a Beam code, or the phone has
     * no address on a network the browser could share.
     */
    suspend fun start(code: String): PendingLink {
        val pairing = engine.pairWithBrowser(code)
        return try {
            PendingLink(pairing.room(), pairing.offer(addresses()), pairing, links, trusted)
        } catch (e: Exception) {
            pairing.stop()
            pairing.close()
            throw e
        }
    }
}

/**
 * A browser link waiting for the browser. Post [offer] in the signaling [room], then call
 * [await]; [close] gives up.
 */
class PendingLink internal constructor(
    /** The signaling room, as `/rooms/{room}` takes it. */
    val room: String,
    /** The sealed offer to post there. */
    val offer: ByteArray,
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
        trusted.add(TrustedDevice.from(browser, System.currentTimeMillis()))
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
