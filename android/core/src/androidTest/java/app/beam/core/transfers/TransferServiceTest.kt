package app.beam.core.transfers

import android.Manifest
import android.app.Notification
import android.app.NotificationManager
import android.content.Intent
import android.os.Build
import android.service.notification.StatusBarNotification
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

/** The foreground service on a real phone: shown while transfers run, gone after. */
@RunWith(AndroidJUnit4::class)
class TransferServiceTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val context = instrumentation.targetContext
    private val transfers = Transfers.get(context)
    private val manager = context.getSystemService(NotificationManager::class.java)

    @Before
    fun allowNotifications() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            instrumentation.uiAutomation.grantRuntimePermission(
                context.packageName,
                Manifest.permission.POST_NOTIFICATIONS
            )
        }
    }

    @After
    fun cleanUp() {
        transfers.registry.cancelAll()
        transfers.registry.clearFinished()
        manager.cancelAll()
    }

    @Test
    fun aRunningTransferIsForegroundUntilItEndsThenSummarised() = runBlocking {
        val finish = CompletableDeferred<Unit>()
        transfers.runner.start(Direction.RECEIVING, "album.zip", 100) { progress ->
            progress.onProgress(40u, 100u)
            finish.await()
            TransferState.Done
        }
        val ongoing = waitFor(TransferNotifications.ONGOING_ID)
        assertTrue(
            "shown by a foreground service",
            ongoing.notification.flags and Notification.FLAG_FOREGROUND_SERVICE != 0
        )
        assertEquals("Receiving album.zip", ongoing.title())
        assertTrue("the phone stays awake for it", holdsWakeLock())

        finish.complete(Unit)
        assertEquals("Received album.zip", waitFor(TransferNotifications.SUMMARY_ID).title())
        waitForGone(TransferNotifications.ONGOING_ID)
        assertFalse("and may sleep after", holdsWakeLock())
    }

    @Test
    fun theNotificationsCancelStopsTransfers() = runBlocking {
        val id = transfers.runner.start(Direction.SENDING, "video.mp4", 100) {
            awaitCancellation()
        }
        waitFor(TransferNotifications.ONGOING_ID)

        context.startService(
            Intent(context, TransferService::class.java).setAction(TransferService.ACTION_CANCEL)
        )
        val ended = withTimeout(TIMEOUT_MS) {
            transfers.registry.transfers.first { all -> all.none { it.isRunning } }
        }
        assertEquals(TransferState.Cancelled, ended.single { it.id == id }.state)
        val summary = waitFor(TransferNotifications.SUMMARY_ID).title()
        assertEquals("Cancelled video.mp4", summary)
        waitForGone(TransferNotifications.ONGOING_ID)
    }

    private suspend fun waitFor(id: Int): StatusBarNotification = withTimeout(TIMEOUT_MS) {
        var found: StatusBarNotification? = null
        while (found == null) {
            found = manager.activeNotifications.firstOrNull { it.id == id }
            if (found == null) delay(POLL_MS)
        }
        found
    }

    private suspend fun waitForGone(id: Int) = withTimeout(TIMEOUT_MS) {
        while (manager.activeNotifications.any { it.id == id }) delay(POLL_MS)
    }

    /** Whether Android lists the transfers' wake lock as held. */
    private fun holdsWakeLock(): Boolean {
        val dump = instrumentation.uiAutomation.executeShellCommand("dumpsys power")
        val text = android.os.ParcelFileDescriptor.AutoCloseInputStream(dump)
            .bufferedReader()
            .use { it.readText() }
        return text.lineSequence().any { "PARTIAL_WAKE_LOCK" in it && "'${StayAwake.TAG}'" in it }
    }

    private fun StatusBarNotification.title(): String? =
        notification.extras.getCharSequence(Notification.EXTRA_TITLE)?.toString()

    private companion object {
        const val TIMEOUT_MS = 10_000L
        const val POLL_MS = 100L
    }
}
