package app.beam.core.transfers

import android.content.Context
import android.net.wifi.WifiManager
import android.os.Build
import android.os.PowerManager

/**
 * Keeps the phone working on transfers with the screen off. A foreground service keeps the
 * process alive but lets the CPU sleep, which stalls a transfer; this holds a partial wake
 * lock while transfers run. Each [hold] lasts [HOLD_MS], so a transfer that stops reporting
 * progress can't keep the phone awake for long.
 *
 * Below Android 14 it also holds a high-performance Wi-Fi lock, so Wi-Fi doesn't drop into
 * power saving mid-transfer. Android 14+ turns that lock into a low-latency one, which only
 * works with the screen on and may lower throughput, so none is taken there.
 */
internal class StayAwake(context: Context) {
    private val wake = context.getSystemService(PowerManager::class.java)
        .newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, TAG)
        .apply { setReferenceCounted(false) }

    private val wifi: WifiManager.WifiLock? =
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            context.applicationContext.getSystemService(WifiManager::class.java)
                ?.createWifiLock(highPerformance(), TAG)
                ?.apply { setReferenceCounted(false) }
        } else {
            null
        }

    /** Whether the wake lock is held now. */
    val isHeld: Boolean get() = wake.isHeld

    /** Holds the locks for the next [HOLD_MS]; call again while transfers still run. */
    fun hold() {
        wake.acquire(HOLD_MS)
        wifi?.takeUnless { it.isHeld }?.acquire()
    }

    /** Lets the phone sleep again. */
    fun release() {
        if (wake.isHeld) wake.release()
        wifi?.takeIf { it.isHeld }?.release()
    }

    @Suppress("DEPRECATION") // only used below Android 14, where it is still what it says
    private fun highPerformance() = WifiManager.WIFI_MODE_FULL_HIGH_PERF

    companion object {
        /** Shown in `dumpsys power`; `app:what` is Android's naming convention. */
        const val TAG = "beam:transfers"
        const val HOLD_MS = 10 * 60 * 1000L
    }
}
