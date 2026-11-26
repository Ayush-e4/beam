package app.beam.core.network

import android.Manifest
import android.annotation.SuppressLint
import android.content.Context
import android.content.pm.PackageManager
import android.location.LocationManager
import android.net.wifi.SoftApConfiguration
import android.net.wifi.WifiManager
import android.os.Build
import android.os.Handler
import android.os.Looper
import java.net.Inet4Address
import java.net.NetworkInterface
import kotlin.coroutines.resumeWithException
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withTimeoutOrNull

/**
 * A private Wi-Fi network the phone opens (Android's local-only hotspot), so a laptop or tablet
 * can reach it with no shared Wi-Fi. It shares no internet and uses no mobile data; Android
 * picks its name and a random password each time.
 */
class Hotspot(context: Context) : HotspotOpener {
    private val context = context.applicationContext
    private val wifi = context.getSystemService(WifiManager::class.java)

    /** The permission people still have to allow before [start], or null. */
    fun missingPermission(): String? {
        val needed = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            Manifest.permission.NEARBY_WIFI_DEVICES
        } else {
            Manifest.permission.ACCESS_FINE_LOCATION
        }
        val granted = context.checkSelfPermission(needed) == PackageManager.PERMISSION_GRANTED
        return if (granted) null else needed
    }

    override suspend fun open(): OpenHotspot = start()

    /**
     * Opens the hotspot. Close the session to turn it off.
     *
     * @throws HotspotException if it can't be opened.
     */
    suspend fun start(): HotspotSession {
        missingPermission()?.let {
            throw HotspotException(HotspotProblem.NEEDS_PERMISSION, "not allowed: $it")
        }
        if (!locationAllowsIt()) {
            throw HotspotException(HotspotProblem.LOCATION_OFF, "location is off")
        }
        val before = interfaceAddresses()
        val on = MutableStateFlow(true)
        val reservation = reserve(on)
        try {
            val details = details(reservation)
            val address = withTimeoutOrNull(ADDRESS_WAIT_MS) { newAddress(before) }
            return HotspotSession(
                reservation,
                details.ssid,
                details.passphrase,
                details.security,
                address,
                on
            )
        } catch (failure: Throwable) {
            reservation.close()
            throw failure
        }
    }

    // The permissions are checked in start(); lint can't follow the version-dependent one.
    @SuppressLint("MissingPermission")
    private suspend fun reserve(on: MutableStateFlow<Boolean>) =
        suspendCancellableCoroutine { continuation ->
            val callback = object : WifiManager.LocalOnlyHotspotCallback() {
                override fun onStarted(reservation: WifiManager.LocalOnlyHotspotReservation) {
                    continuation.resume(reservation) { _, opened, _ -> opened.close() }
                }

                override fun onStopped() {
                    on.value = false
                }

                override fun onFailed(reason: Int) {
                    on.value = false
                    if (continuation.isActive) {
                        val problem = problemOf(reason)
                        continuation.resumeWithException(
                            HotspotException(problem, "Android refused the hotspot: $problem")
                        )
                    }
                }
            }
            try {
                wifi.startLocalOnlyHotspot(callback, Handler(Looper.getMainLooper()))
            } catch (refused: SecurityException) {
                continuation.resumeWithException(
                    HotspotException(HotspotProblem.NEEDS_PERMISSION, "${refused.message}")
                )
            } catch (busy: IllegalStateException) {
                // This app already has a hotspot request open.
                continuation.resumeWithException(
                    HotspotException(HotspotProblem.FAILED, "${busy.message}")
                )
            }
        }

    private fun locationAllowsIt(): Boolean {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) return true
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.P) return true // Android says so itself.
        return context.getSystemService(LocationManager::class.java).isLocationEnabled
    }

    private class Details(val ssid: String, val passphrase: String?, val security: WifiSecurity)

    private fun details(reservation: WifiManager.LocalOnlyHotspotReservation): Details {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.R) {
            @Suppress("DEPRECATION")
            val config = reservation.wifiConfiguration
            val ssid = config?.SSID?.removeSurrounding("\"")
                ?: throw HotspotException(HotspotProblem.FAILED, "the hotspot has no name")
            return Details(ssid, config.preSharedKey?.removeSurrounding("\""), WifiSecurity.WPA)
        }
        val config = reservation.softApConfiguration
        val ssid = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            config.wifiSsid?.bytes?.decodeToString()
        } else {
            @Suppress("DEPRECATION")
            config.ssid
        } ?: throw HotspotException(HotspotProblem.FAILED, "the hotspot has no name")
        val security = when (config.securityType) {
            SoftApConfiguration.SECURITY_TYPE_OPEN -> WifiSecurity.OPEN
            SoftApConfiguration.SECURITY_TYPE_WPA3_SAE -> WifiSecurity.SAE
            else -> WifiSecurity.WPA
        }
        return Details(ssid, config.passphrase, security)
    }

    private companion object {
        const val ADDRESS_WAIT_MS = 5_000L
        const val ADDRESS_POLL_MS = 100L

        fun interfaceAddresses(): Set<Inet4Address> =
            NetworkInterface.getNetworkInterfaces().toList()
                .filter { it.isUp && !it.isLoopback }
                .flatMap { it.inetAddresses.toList() }
                .filterIsInstance<Inet4Address>()
                .toSet()

        /** The hotspot's interface gets its address just after Android reports it open. */
        suspend fun newAddress(before: Set<Inet4Address>): Inet4Address {
            while (true) {
                val fresh = (interfaceAddresses() - before).firstOrNull { it.isSiteLocalAddress }
                if (fresh != null) return fresh
                delay(ADDRESS_POLL_MS)
            }
        }
    }
}
