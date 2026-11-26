package app.beam.core.network

import org.junit.Assert.assertEquals
import org.junit.Test

class WifiQrTest {
    @Test
    fun aPasswordNetworkNamesItsSecurityAndPassword() {
        assertEquals(
            "WIFI:T:WPA;S:AndroidShare_1234;P:k9x2pq7m;;",
            wifiQr("AndroidShare_1234", "k9x2pq7m", WifiSecurity.WPA)
        )
    }

    @Test
    fun wpa3OnlyIsSae() {
        assertEquals("WIFI:T:SAE;S:Beam;P:pw;;", wifiQr("Beam", "pw", WifiSecurity.SAE))
    }

    @Test
    fun anOpenNetworkHasNoPassword() {
        assertEquals("WIFI:T:nopass;S:Beam;;", wifiQr("Beam", "ignored", WifiSecurity.OPEN))
    }

    @Test
    fun separatorsInTheNameAndPasswordAreEscaped() {
        assertEquals(
            """WIFI:T:WPA;S:a\;b\,c\:d;P:q\"\\;;""",
            wifiQr("a;b,c:d", """q"\""", WifiSecurity.WPA)
        )
    }
}
