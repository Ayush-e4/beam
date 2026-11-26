package app.beam.core.network

import android.content.Context
import android.net.ConnectivityManager
import android.net.LinkProperties
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import java.net.InetAddress

/** Finds the network a nearby device's address is on, as a `Network.getNetworkHandle()`. */
fun interface NetworkFinder {
    /** The handle of the local network [address] is on, or null to use the default route. */
    fun networkFor(address: InetAddress): Long?

    companion object {
        /** Always the default route (tests, and devices reached by loopback). */
        val DEFAULT = NetworkFinder { null }
    }
}

/**
 * The phone's local networks (Wi-Fi, Ethernet, a hotspot it hosts; never a VPN), followed
 * while [start]ed. A connection to a nearby device should go over the one whose subnet holds
 * its address: a VPN as the default network would otherwise capture it and couldn't reach
 * the device. Networks without internet count too, since nearby devices don't need it.
 */
class LocalNetworks(context: Context) : NetworkFinder {
    private val connectivity = context.getSystemService(ConnectivityManager::class.java)
    private val links = HashMap<Network, LinkProperties>()
    private var callback: ConnectivityManager.NetworkCallback? = null

    /** Starts following the networks; call [stop] when nearby devices no longer matter. */
    @Synchronized
    fun start() {
        if (callback != null) return
        val request = NetworkRequest.Builder()
            .removeCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
            .addCapability(NetworkCapabilities.NET_CAPABILITY_NOT_VPN)
            .build()
        val follow = Follow()
        connectivity.registerNetworkCallback(request, follow)
        callback = follow
    }

    @Synchronized
    fun stop() {
        callback?.let(connectivity::unregisterNetworkCallback)
        callback = null
        links.clear()
    }

    @Synchronized
    override fun networkFor(address: InetAddress): Long? {
        if (address.isLoopbackAddress) return null
        return links.entries.firstOrNull { (_, link) ->
            link.linkAddresses.any { linkAddress ->
                val local = LocalAddress(linkAddress.address, linkAddress.prefixLength)
                sameSubnet(address, local, link.interfaceName)
            }
        }?.key?.networkHandle
    }

    private inner class Follow : ConnectivityManager.NetworkCallback() {
        override fun onLinkPropertiesChanged(network: Network, properties: LinkProperties) {
            synchronized(this@LocalNetworks) { links[network] = properties }
        }

        override fun onLost(network: Network) {
            synchronized(this@LocalNetworks) { links.remove(network) }
        }
    }
}
