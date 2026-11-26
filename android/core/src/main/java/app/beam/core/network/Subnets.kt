package app.beam.core.network

import java.net.Inet6Address
import java.net.InetAddress

/** One address of a local network interface, as Android's `LinkAddress` gives it. */
internal data class LocalAddress(val address: InetAddress, val prefixLength: Int)

/**
 * Whether [peer] is on the same subnet as [local]: the same address family, and the same
 * first [LocalAddress.prefixLength] bits. An IPv6 link-local peer (fe80::/10) only counts
 * when it is scoped to [interfaceName], the interface [local] belongs to.
 */
internal fun sameSubnet(peer: InetAddress, local: LocalAddress, interfaceName: String?): Boolean {
    val ours = local.address.address
    val theirs = peer.address
    if (ours.size != theirs.size) return false
    if (peer is Inet6Address && peer.isLinkLocalAddress) {
        val scope = peer.scopedInterface?.name
        return local.address.isLinkLocalAddress && scope != null && scope == interfaceName
    }
    val bits = local.prefixLength.coerceIn(0, ours.size * Byte.SIZE_BITS)
    val whole = bits / Byte.SIZE_BITS
    for (i in 0 until whole) {
        if (ours[i] != theirs[i]) return false
    }
    val rest = bits % Byte.SIZE_BITS
    if (rest == 0) return true
    val mask = (0xFF shl (Byte.SIZE_BITS - rest)) and 0xFF
    return (ours[whole].toInt() and mask) == (theirs[whole].toInt() and mask)
}
