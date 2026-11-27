package app.beam.core.network

import java.net.Inet6Address
import java.net.InetAddress
import java.net.NetworkInterface
import java.net.SocketException

/**
 * The addresses other devices on this phone's networks can reach it at, for pairing offers:
 * Wi-Fi and hotspot first, IPv4 before IPv6. Mobile data, VPN tunnels, loopback and IPv6
 * link-local addresses are left out (nobody nearby can use them).
 */
object PhoneAddresses {
    private val UNREACHABLE = listOf("rmnet", "ccmni", "tun", "dummy", "ipsec", "clat")

    fun find(): List<String> = try {
        NetworkInterface.getNetworkInterfaces().toList()
            .filter { it.isUp && !it.isLoopback && UNREACHABLE.none(it.name::startsWith) }
            .sortedByDescending { it.name.startsWith("wlan") || it.name.startsWith("swlan") }
            .flatMap { nic -> nic.inetAddresses.toList() }
            .filter(::usable)
            .sortedBy { it is Inet6Address }
            .mapNotNull { it.hostAddress?.substringBefore('%') }
            .distinct()
    } catch (e: SocketException) {
        emptyList()
    }

    private fun usable(address: InetAddress): Boolean =
        !address.isLoopbackAddress && !address.isLinkLocalAddress && !address.isAnyLocalAddress
}
