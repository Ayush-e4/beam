package app.beam.core.network

import java.net.Inet6Address
import java.net.InetAddress
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class SubnetsTest {
    private fun ip(text: String): InetAddress = InetAddress.getByName(text)

    private fun local(text: String, prefix: Int) = LocalAddress(ip(text), prefix)

    @Test
    fun `an IPv4 address in the same subnet matches`() {
        val wifi = local("192.168.1.23", 24)
        assertTrue(sameSubnet(ip("192.168.1.200"), wifi, "wlan0"))
        assertFalse(sameSubnet(ip("192.168.2.200"), wifi, "wlan0"))
        assertFalse(sameSubnet(ip("8.8.8.8"), wifi, "wlan0"))
    }

    @Test
    fun `a prefix that splits a byte compares only its bits`() {
        val office = local("10.0.18.5", 20) // 10.0.16.0 - 10.0.31.255
        assertTrue(sameSubnet(ip("10.0.31.254"), office, "wlan0"))
        assertTrue(sameSubnet(ip("10.0.16.1"), office, "wlan0"))
        assertFalse(sameSubnet(ip("10.0.32.1"), office, "wlan0"))
        assertFalse(sameSubnet(ip("10.0.15.255"), office, "wlan0"))
    }

    @Test
    fun `IPv6 matches by prefix and never across families`() {
        val wifi = local("2001:db8:aa:1::5", 64)
        assertTrue(sameSubnet(ip("2001:db8:aa:1:abcd::9"), wifi, "wlan0"))
        assertFalse(sameSubnet(ip("2001:db8:aa:2::9"), wifi, "wlan0"))
        assertFalse(sameSubnet(ip("192.168.1.9"), wifi, "wlan0"))
        assertFalse(sameSubnet(ip("2001:db8:aa:1::9"), local("192.168.1.1", 24), "wlan0"))
    }

    @Test
    fun `a link-local peer without its interface matches nothing`() {
        val unscoped = Inet6Address.getByAddress(null, ip("fe80::1").address, 0)
        assertFalse(sameSubnet(unscoped, local("fe80::2", 64), "wlan0"))
    }
}
