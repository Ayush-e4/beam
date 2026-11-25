package app.beam.core.discovery

import app.beam.core.trust.TrustedDevice
import java.net.InetAddress

/** A trusted device announcing itself on the local network right now. */
data class NearbyDevice(
    val device: TrustedDevice,
    /** Where it can be reached, as mDNS resolved them. */
    val addresses: List<InetAddress>,
    val port: Int,
    /** False when it speaks another protocol major version: one of the two needs updating. */
    val compatible: Boolean
)

/**
 * Which trusted devices are nearby, keyed by the mDNS service (beacon) they announce. A device
 * that picks a new beacon briefly shows under both; the newest wins, so it is listed once.
 * Not thread-safe: callers synchronise.
 */
internal class NearbyTracker {
    private val byService = LinkedHashMap<String, NearbyDevice>()

    /** Records that [service] resolved to [nearby]; returns the new list. */
    fun resolved(service: String, nearby: NearbyDevice): List<NearbyDevice> {
        byService.entries.removeAll { (name, known) ->
            name != service && known.device.deviceId == nearby.device.deviceId
        }
        byService[service] = nearby
        return snapshot()
    }

    /** Records that [service] went away; returns the new list, or null if nothing changed. */
    fun lost(service: String): List<NearbyDevice>? = byService.remove(service)?.let { snapshot() }

    fun snapshot(): List<NearbyDevice> = byService.values.toList()
}
