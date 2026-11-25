package app.beam.core.discovery

import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.os.Build
import androidx.annotation.RequiresApi
import java.net.InetAddress
import java.util.concurrent.Executor

/** A resolved service: where to connect, and its TXT attributes. */
internal data class Resolved(
    val addresses: List<InetAddress>,
    val port: Int,
    val attributes: Map<String, ByteArray?>
)

/** Turns found services into addresses. Callbacks run on the executor it was made with. */
internal interface ServiceResolver {
    /** Starts resolving [service]; [onResolved] may run again if its addresses change. */
    fun resolve(service: NsdServiceInfo, onResolved: (Resolved) -> Unit, onFailed: (Int) -> Unit)

    /** Stops resolving the service named [name]. Safe to call for one it never resolved. */
    fun cancel(name: String)

    /** Stops everything. */
    fun close()

    companion object {
        fun create(nsd: NsdManager, executor: Executor): ServiceResolver =
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
                WatchingResolver(nsd, executor)
            } else {
                QueuedResolver(nsd, executor)
            }
    }
}

/** Android 14+: watches each service and hears about address changes. */
@RequiresApi(Build.VERSION_CODES.UPSIDE_DOWN_CAKE)
private class WatchingResolver(private val nsd: NsdManager, private val executor: Executor) :
    ServiceResolver {
    private val watching = HashMap<String, NsdManager.ServiceInfoCallback>()

    @Synchronized
    override fun resolve(
        service: NsdServiceInfo,
        onResolved: (Resolved) -> Unit,
        onFailed: (Int) -> Unit
    ) {
        if (service.serviceName in watching) return
        val callback = object : NsdManager.ServiceInfoCallback {
            override fun onServiceUpdated(info: NsdServiceInfo) =
                onResolved(Resolved(info.hostAddresses, info.port, info.attributes))

            override fun onServiceInfoCallbackRegistrationFailed(errorCode: Int) {
                forget(service.serviceName)
                onFailed(errorCode)
            }

            override fun onServiceLost() = Unit // discovery reports the loss too
            override fun onServiceInfoCallbackUnregistered() = Unit
        }
        watching[service.serviceName] = callback
        nsd.registerServiceInfoCallback(service, executor, callback)
    }

    @Synchronized
    private fun forget(name: String) = watching.remove(name)

    @Synchronized
    override fun cancel(name: String) {
        watching.remove(name)?.let(nsd::unregisterServiceInfoCallback)
    }

    @Synchronized
    override fun close() {
        watching.values.forEach(nsd::unregisterServiceInfoCallback)
        watching.clear()
    }
}

/** Before Android 14 a device resolves one service at a time, so requests wait in line. */
private class QueuedResolver(private val nsd: NsdManager, private val executor: Executor) :
    ServiceResolver {
    private class Request(
        val service: NsdServiceInfo,
        val onResolved: (Resolved) -> Unit,
        val onFailed: (Int) -> Unit
    )

    private val waiting = ArrayDeque<Request>()
    private var active: Request? = null
    private var closed = false

    @Synchronized
    override fun resolve(
        service: NsdServiceInfo,
        onResolved: (Resolved) -> Unit,
        onFailed: (Int) -> Unit
    ) {
        if (closed) return
        waiting.addLast(Request(service, onResolved, onFailed))
        startNext()
    }

    @Synchronized
    override fun cancel(name: String) {
        waiting.removeAll { it.service.serviceName == name }
    }

    @Synchronized
    override fun close() {
        closed = true
        waiting.clear()
    }

    private fun startNext() {
        if (active != null || closed) return
        val request = waiting.removeFirstOrNull() ?: return
        active = request
        @Suppress("DEPRECATION") // the replacement needs Android 14
        nsd.resolveService(request.service, Listener(request))
    }

    @Synchronized
    private fun finished(request: Request): Boolean {
        val wanted = !closed
        active = null
        startNext()
        return wanted
    }

    private inner class Listener(private val request: Request) : NsdManager.ResolveListener {
        override fun onServiceResolved(info: NsdServiceInfo) {
            if (!finished(request)) return
            @Suppress("DEPRECATION") // hostAddresses needs Android 14
            val addresses = listOfNotNull(info.host)
            executor.execute {
                request.onResolved(Resolved(addresses, info.port, info.attributes))
            }
        }

        override fun onResolveFailed(info: NsdServiceInfo, errorCode: Int) {
            if (finished(request)) executor.execute { request.onFailed(errorCode) }
        }
    }
}
