package app.beam.core.discovery

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.util.Log
import app.beam.core.BeamException
import app.beam.core.Engine
import app.beam.core.versionInfo
import kotlin.time.Duration
import kotlin.time.Duration.Companion.minutes
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch

/**
 * Announces this device on the local network under a beacon only trusted devices recognise,
 * and swaps it for a new one every [rotateEvery] and whenever the network changes.
 */
class BeaconAnnouncer(
    context: Context,
    private val engine: Engine,
    private val scope: CoroutineScope,
    private val rotateEvery: Duration = 15.minutes
) {
    /** What the announcer is doing. */
    sealed interface State {
        data object Stopped : State
        data class Announcing(val beacon: String) : State
        data class Failed(val reason: String) : State
    }

    private val context = context.applicationContext
    private val nsd = context.getSystemService(NsdManager::class.java)
    private val connectivity = context.getSystemService(ConnectivityManager::class.java)
    private val mutableState = MutableStateFlow<State>(State.Stopped)
    private var port = 0
    private var registration: NsdManager.RegistrationListener? = null
    private var rotation: Job? = null
    private var network: Network? = null
    private var networkCallback: ConnectivityManager.NetworkCallback? = null

    val state: StateFlow<State> = mutableState.asStateFlow()

    /** Starts announcing that this device listens on [port]. Throws [DiscoveryException]. */
    @Synchronized
    fun start(port: Int) {
        Mdns.ensureLocalNetworkAllowed(context)
        stop()
        this.port = port
        announce()
        rotation = scope.launch {
            while (isActive) {
                delay(rotateEvery)
                rotate()
            }
        }
        val callback = NetworkChanges()
        connectivity.registerDefaultNetworkCallback(callback)
        networkCallback = callback
    }

    /** Swaps the beacon for a new one now. */
    @Synchronized
    fun rotate() {
        if (registration == null && mutableState.value !is State.Failed) return
        withdraw()
        announce()
    }

    /** Stops announcing. */
    @Synchronized
    fun stop() {
        rotation?.cancel()
        rotation = null
        networkCallback?.let(connectivity::unregisterNetworkCallback)
        networkCallback = null
        network = null
        withdraw()
        mutableState.value = State.Stopped
    }

    private fun announce() {
        val beacon = try {
            engine.newBeacon()
        } catch (e: BeamException) {
            mutableState.value = State.Failed("could not make a beacon: ${e.message}")
            return
        }
        val info = NsdServiceInfo().apply {
            serviceName = beacon
            serviceType = Mdns.SERVICE_TYPE
            port = this@BeaconAnnouncer.port
            setAttribute(Mdns.VERSION_KEY, versionInfo().protocolMajor.toString())
        }
        val listener = Registration(beacon)
        registration = listener
        nsd.registerService(info, NsdManager.PROTOCOL_DNS_SD, listener)
    }

    private fun withdraw() {
        val listener = registration ?: return
        registration = null
        try {
            nsd.unregisterService(listener)
        } catch (e: IllegalArgumentException) {
            // Registration failed earlier, so the system already forgot this listener.
            Log.d(TAG, "beacon was not registered: ${e.message}")
        }
    }

    @Synchronized
    private fun registered(listener: Registration, name: String) {
        // NSD may rename on a clash; a renamed beacon is useless, so try a fresh one.
        if (registration !== listener) return
        if (name != listener.beacon) {
            rotate()
        } else {
            mutableState.value = State.Announcing(name)
        }
    }

    @Synchronized
    private fun failed(listener: Registration, errorCode: Int) {
        if (registration !== listener) return
        registration = null
        mutableState.value =
            State.Failed("could not announce on the network (NSD error $errorCode)")
    }

    private inner class Registration(val beacon: String) : NsdManager.RegistrationListener {
        override fun onServiceRegistered(info: NsdServiceInfo) = registered(this, info.serviceName)
        override fun onRegistrationFailed(info: NsdServiceInfo, errorCode: Int) =
            failed(this, errorCode)

        override fun onUnregistrationFailed(info: NsdServiceInfo, errorCode: Int) {
            Log.w(TAG, "could not withdraw a beacon: $errorCode")
        }

        override fun onServiceUnregistered(info: NsdServiceInfo) = Unit
    }

    /** A new default network means a new place: rotate so the beacon can't link the two. */
    private inner class NetworkChanges : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(available: Network) {
            synchronized(this@BeaconAnnouncer) {
                val previous = network
                network = available
                if (previous != null && previous != available) rotate()
            }
        }
    }

    private companion object {
        const val TAG = "BeamDiscovery"
    }
}
