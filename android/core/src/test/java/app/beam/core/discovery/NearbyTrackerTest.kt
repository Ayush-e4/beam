package app.beam.core.discovery

import app.beam.core.SessionCipher
import app.beam.core.trust.TrustedDevice
import java.net.InetAddress
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class NearbyTrackerTest {
    private fun nearby(n: Int, port: Int = 4000) = NearbyDevice(
        TrustedDevice(
            "id-$n",
            ByteArray(32) { n.toByte() },
            "Phone $n",
            SessionCipher.AES_GCM,
            ByteArray(32),
            0
        ),
        listOf(InetAddress.getByAddress(byteArrayOf(192.toByte(), 168.toByte(), 1, n.toByte()))),
        port,
        compatible = true
    )

    @Test
    fun `devices come and go by service`() {
        val tracker = NearbyTracker()
        tracker.resolved("A", nearby(1))
        assertEquals(listOf(nearby(1), nearby(2)), tracker.resolved("B", nearby(2)))
        assertEquals(listOf(nearby(2)), tracker.lost("A"))
        assertNull("losing an unknown service changes nothing", tracker.lost("A"))
    }

    @Test
    fun `a device with a new beacon is listed once`() {
        val tracker = NearbyTracker()
        tracker.resolved("OLD", nearby(1))
        assertEquals(listOf(nearby(1, port = 5000)), tracker.resolved("NEW", nearby(1, 5000)))
        assertNull("the old beacon's late goodbye changes nothing", tracker.lost("OLD"))
        assertEquals(listOf(nearby(1, port = 5000)), tracker.snapshot())
    }

    @Test
    fun `a device that resolves again is updated in place`() {
        val tracker = NearbyTracker()
        tracker.resolved("A", nearby(1))
        assertEquals(listOf(nearby(1, port = 6000)), tracker.resolved("A", nearby(1, 6000)))
    }
}
