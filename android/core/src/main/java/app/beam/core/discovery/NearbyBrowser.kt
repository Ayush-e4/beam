package app.beam.core.discovery

import android.content.Context
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.net.wifi.WifiManager
import android.util.Log
import app.beam.core.recogniseBeacon
import app.beam.core.trust.TrustedDevice
import app.beam.core.versionInfo
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.ThreadPoolExecutor
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.callbackFlow
import kotlinx.coroutines.flow.conflate

/**
 * Finds trusted devices on the local network. Only services whose beacon matches a trusted
 * device are resolved, so this phone never contacts unknown devices just by browsing.
 */
class NearbyBrowser(
    context: Context,
    /** The trusted devices to look for, read whenever a service appears. */
    private val trusted: () -> List<TrustedDevice>
) {
    private val context = context.applicationContext
    private val nsd = context.getSystemService(NsdManager::class.java)
    private val wifi = context.getSystemService(WifiManager::class.java)

    /**
     * The trusted devices nearby, updated as they come, go and change address. Browsing runs
     * while this is collected. Fails with [DiscoveryException] if it can't start.
     */
    fun nearby(): Flow<List<NearbyDevice>> = callbackFlow {
        Mdns.ensureLocalNetworkAllowed(context)
        val executor = callbackExecutor()
        val tracker = NearbyTracker()
        val resolver = ServiceResolver.create(nsd, executor)
        val multicast = wifi.createMulticastLock("beam-discovery").apply {
            setReferenceCounted(false)
            acquire()
        }

        fun publish(list: List<NearbyDevice>?) {
            if (list != null) trySend(list)
        }

        fun found(service: NsdServiceInfo) {
            val devices = trusted()
            val index = recogniseBeacon(service.serviceName, devices.map { it.discoveryKey })
                ?: return
            val device = devices[index.toInt()]
            resolver.resolve(
                service,
                onResolved = { resolved ->
                    val nearby = NearbyDevice(
                        device,
                        resolved.addresses,
                        resolved.port,
                        compatible = speaksOurProtocol(resolved.attributes)
                    )
                    publish(synchronized(tracker) { tracker.resolved(service.serviceName, nearby) })
                },
                onFailed = { code -> Log.w(TAG, "could not resolve a trusted device: $code") }
            )
        }

        val listener = object : NsdManager.DiscoveryListener {
            override fun onServiceFound(service: NsdServiceInfo) =
                executor.execute { found(service) }

            override fun onServiceLost(service: NsdServiceInfo) = executor.execute {
                resolver.cancel(service.serviceName)
                publish(synchronized(tracker) { tracker.lost(service.serviceName) })
            }

            override fun onStartDiscoveryFailed(serviceType: String, errorCode: Int) {
                close(DiscoveryException("could not start discovery (NSD error $errorCode)"))
            }

            override fun onStopDiscoveryFailed(serviceType: String, errorCode: Int) {
                Log.w(TAG, "could not stop discovery: $errorCode")
            }

            override fun onDiscoveryStarted(serviceType: String) = Unit
            override fun onDiscoveryStopped(serviceType: String) = Unit
        }

        trySend(emptyList())
        nsd.discoverServices(Mdns.SERVICE_TYPE, NsdManager.PROTOCOL_DNS_SD, listener)
        awaitClose {
            stopDiscovery(listener)
            resolver.close()
            executor.shutdown()
            multicast.release()
        }
    }.conflate()

    private fun stopDiscovery(listener: NsdManager.DiscoveryListener) {
        try {
            nsd.stopServiceDiscovery(listener)
        } catch (e: IllegalArgumentException) {
            // Discovery never started (it failed and closed the flow), so there is nothing to stop.
            Log.d(TAG, "discovery was not running: ${e.message}")
        }
    }

    /**
     * One thread for every callback. NSD still delivers a few (unregistered, stopped) after
     * browsing ends and the executor shuts down; they have nothing left to do, so they're dropped.
     */
    private fun callbackExecutor() = ThreadPoolExecutor(
        1,
        1,
        0L,
        TimeUnit.MILLISECONDS,
        LinkedBlockingQueue(),
        { runnable -> Thread(runnable, "beam-discovery") },
        ThreadPoolExecutor.DiscardPolicy()
    )

    private fun speaksOurProtocol(attributes: Map<String, ByteArray?>): Boolean {
        val major = attributes[Mdns.VERSION_KEY]?.decodeToString()?.toIntOrNull()
        return major == versionInfo().protocolMajor.toInt()
    }

    private companion object {
        const val TAG = "BeamDiscovery"
    }
}
