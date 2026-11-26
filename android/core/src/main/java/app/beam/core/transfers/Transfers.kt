package app.beam.core.transfers

import android.content.Context
import app.beam.core.history.TransferHistory
import app.beam.core.network.Hotspot
import app.beam.core.network.PhoneHotspot
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.map

/**
 * The app's transfers: one [registry] and [runner] per process, outliving every screen.
 * Starting a transfer through [runner] keeps it running in the background until it ends;
 * [history] keeps what was sent and received, and [hotspot] is the phone's hotspot button,
 * which stays open while transfers run.
 */
class Transfers private constructor(context: Context) {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    val registry = TransferRegistry()
    val history = TransferHistory.forApp(context)
    val runner = TransferRunner(scope, registry, onStarted = { TransferService.start(context) })
    val hotspot = PhoneHotspot(
        Hotspot(context),
        registry.transfers.map { all -> all.any { it.isRunning } },
        scope
    )

    companion object {
        @Volatile
        private var instance: Transfers? = null

        /** This process's transfers. */
        fun get(context: Context): Transfers = instance ?: synchronized(this) {
            instance ?: Transfers(context.applicationContext).also { instance = it }
        }
    }
}
