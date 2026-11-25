package app.beam.core.discovery

import android.content.Context
import android.content.pm.PackageManager
import android.os.Build

/** What Beam announces on the local network (see docs/protocol.md, Discovery). */
internal object Mdns {
    /** The service type every Beam device announces. */
    const val SERVICE_TYPE = "_beam._tcp"

    /** TXT key holding the protocol major version. */
    const val VERSION_KEY = "v"

    /** Android 17 asks people before an app may use the local network. */
    private const val LOCAL_NETWORK_PERMISSION = "android.permission.ACCESS_LOCAL_NETWORK"
    private const val ANDROID_17 = 37

    /** Throws [DiscoveryException] if this app may not use the local network yet. */
    fun ensureLocalNetworkAllowed(context: Context) {
        if (Build.VERSION.SDK_INT >= ANDROID_17 &&
            context.checkSelfPermission(LOCAL_NETWORK_PERMISSION) !=
            PackageManager.PERMISSION_GRANTED
        ) {
            throw DiscoveryException("the local network permission hasn't been granted")
        }
    }
}

/** Discovery could not start or stopped working; [message] says why. */
class DiscoveryException(message: String) : Exception(message)
