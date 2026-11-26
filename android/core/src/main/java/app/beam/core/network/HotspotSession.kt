package app.beam.core.network

import android.net.wifi.WifiManager
import java.io.Closeable
import java.net.Inet4Address
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/** Why the phone couldn't open (or keep) its hotspot. */
enum class HotspotProblem {
    /** People haven't allowed Beam to use nearby Wi-Fi (or, before Android 13, location). */
    NEEDS_PERMISSION,

    /** Before Android 13, location must be on for apps to open a hotspot. */
    LOCATION_OFF,

    /** No Wi-Fi channel is free for it. */
    NO_CHANNEL,

    /** Something else uses the Wi-Fi radio in a way that rules it out (often the phone's own hotspot). */
    INCOMPATIBLE_MODE,

    /** The phone's owner (a work profile, say) doesn't allow hotspots. */
    NOT_ALLOWED,

    /** Android failed to open it, without saying why. */
    FAILED
}

/** Opening the hotspot failed. */
class HotspotException(val problem: HotspotProblem, message: String) : Exception(message)

/**
 * The phone's open hotspot: what another device needs to join it, and the phone's address on
 * it. Closing it turns the hotspot off (unless another app is using it too).
 */
class HotspotSession internal constructor(
    private val reservation: WifiManager.LocalOnlyHotspotReservation,
    /** The network's name. */
    val ssid: String,
    /** Its password, or null for an open network. */
    val passphrase: String?,
    /** How it checks the password. */
    val security: WifiSecurity,
    /** The phone's address on it, or null if Android didn't show it in time. */
    val address: Inet4Address?,
    private val on: MutableStateFlow<Boolean>
) : Closeable {
    /** False once the hotspot is off: closed here, or stopped by Android. */
    val isOn: StateFlow<Boolean> = on.asStateFlow()

    /** The Wi-Fi QR code text that lets a camera join the hotspot. */
    val wifiQr: String get() = wifiQr(ssid, passphrase, security)

    override fun close() {
        on.value = false
        reservation.close()
    }
}

internal fun problemOf(reason: Int): HotspotProblem = when (reason) {
    WifiManager.LocalOnlyHotspotCallback.ERROR_NO_CHANNEL -> HotspotProblem.NO_CHANNEL
    WifiManager.LocalOnlyHotspotCallback.ERROR_INCOMPATIBLE_MODE -> HotspotProblem.INCOMPATIBLE_MODE
    WifiManager.LocalOnlyHotspotCallback.ERROR_TETHERING_DISALLOWED -> HotspotProblem.NOT_ALLOWED
    else -> HotspotProblem.FAILED
}
