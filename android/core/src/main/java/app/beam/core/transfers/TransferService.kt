package app.beam.core.transfers

import android.app.NotificationManager
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import android.util.Log
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/**
 * Keeps the app process alive while transfers run, so they continue with the screen off or
 * the app closed (a `dataSync` foreground service). It shows overall progress, stops itself
 * when nothing is running and leaves a summary of how the transfers ended.
 */
class TransferService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private lateinit var registry: TransferRegistry
    private lateinit var notifications: TransferNotifications
    private lateinit var manager: NotificationManager
    private var watching: Job? = null

    @Volatile
    private var lastStartId = 0

    override fun onCreate() {
        super.onCreate()
        registry = Transfers.get(this).registry
        notifications = TransferNotifications(this).also { it.createChannel() }
        manager = getSystemService(NotificationManager::class.java)
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        lastStartId = startId
        // Android requires this after every start, even if the transfers already ended.
        if (!goForeground()) return START_NOT_STICKY
        if (intent?.action == ACTION_CANCEL) registry.cancelAll()
        if (watching == null) watching = scope.launch { watch() }
        return START_NOT_STICKY // after the process dies, transfers resume when started again
    }

    override fun onBind(intent: Intent?): IBinder? = null

    /** Android 15+: `dataSync` may run 6 hours a day. Stop cleanly; transfers resume later. */
    override fun onTimeout(startId: Int, fgsType: Int) {
        Log.w(TAG, "Android stopped the transfer service after its daily time limit")
        registry.cancelAll()
        stopForeground(STOP_FOREGROUND_REMOVE)
        stopSelf()
    }

    override fun onDestroy() {
        scope.cancel()
        super.onDestroy()
    }

    private fun goForeground(): Boolean {
        val notification = notifications.ongoing(registry.transfers.value.filter { it.isRunning })
        return try {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                startForeground(
                    TransferNotifications.ONGOING_ID,
                    notification,
                    ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC
                )
            } else {
                startForeground(TransferNotifications.ONGOING_ID, notification)
            }
            true
        } catch (e: IllegalStateException) {
            // Android refused (started from the background, or out of daily time): the
            // transfers still run, but only while the app process lives.
            Log.w(TAG, "could not keep transfers running in the background", e)
            stopSelf(lastStartId)
            false
        }
    }

    /**
     * Follows the registry: progress while running, then a summary, then stops. A StateFlow
     * keeps only the latest list, so a slow notification update skips ahead.
     */
    private suspend fun watch() {
        val seen = HashSet<Long>()
        registry.transfers.collect { all ->
            val running = all.filter { it.isRunning }
            seen += running.map { it.id }
            if (running.isNotEmpty()) {
                manager.notify(TransferNotifications.ONGOING_ID, notifications.ongoing(running))
                delay(UPDATE_EVERY_MS)
            } else if (seen.isNotEmpty()) {
                notifications.showSummary(all.filter { it.id in seen })
                seen.clear()
                stopForeground(STOP_FOREGROUND_REMOVE)
                // A transfer that started meanwhile has its own start ID, which keeps us alive.
                stopSelfResult(lastStartId)
            } else {
                stopForeground(STOP_FOREGROUND_REMOVE)
                stopSelfResult(lastStartId)
            }
        }
    }

    companion object {
        internal const val ACTION_CANCEL = "app.beam.core.transfers.CANCEL"
        private const val TAG = "BeamTransfers"
        private const val UPDATE_EVERY_MS = 1_000L

        /** Asks Android to keep transfers running; they run anyway if it refuses. */
        internal fun start(context: Context) {
            try {
                context.startForegroundService(Intent(context, TransferService::class.java))
            } catch (e: IllegalStateException) {
                Log.w(
                    TAG,
                    "could not start the transfer service; transfers run while Beam is open",
                    e
                )
            }
        }
    }
}
