package app.beam.core.transfers

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.text.format.Formatter
import app.beam.core.R

/** The transfer notifications: one ongoing while transfers run, and a summary when they end. */
internal class TransferNotifications(private val context: Context) {
    private val manager = context.getSystemService(NotificationManager::class.java)

    fun createChannel() {
        val channel = NotificationChannel(
            CHANNEL,
            context.getString(R.string.beam_channel_transfers),
            NotificationManager.IMPORTANCE_LOW // progress shouldn't buzz
        ).apply { description = context.getString(R.string.beam_channel_transfers_description) }
        manager.createNotificationChannel(channel)
    }

    /** The ongoing notification for the [running] transfers, with overall progress. */
    fun ongoing(running: List<TransferStatus>): Notification {
        val done = running.sumOf { it.bytesDone }
        val total = running.sumOf { it.bytesTotal }
        val title = running.singleOrNull()?.let { one ->
            val format = if (one.direction == Direction.SENDING) {
                R.string.beam_sending
            } else {
                R.string.beam_receiving
            }
            context.getString(format, one.title)
        } ?: context.resources.getQuantityString(R.plurals.beam_running, running.size, running.size)
        val cancel = PendingIntent.getService(
            context,
            0,
            Intent(context, TransferService::class.java).setAction(TransferService.ACTION_CANCEL),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
        )
        return builder()
            .setContentTitle(title)
            .setContentText(context.getString(R.string.beam_progress, size(done), size(total)))
            .setProgress(PROGRESS_MAX, permille(done, total), total <= 0)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .addAction(
                Notification.Action.Builder(
                    null,
                    context.getString(R.string.beam_cancel),
                    cancel
                ).build()
            )
            .build()
    }

    /** Replaces the ongoing notification with how the [ended] transfers went. */
    fun showSummary(ended: List<TransferStatus>) {
        if (ended.isEmpty()) return
        val one = ended.singleOrNull()
        val oneState = one?.state as? TransferState.Ended
        val text = if (one != null && oneState != null) {
            describe(one, oneState)
        } else {
            run {
                val problems = ended.count { it.state != TransferState.Done }
                if (problems == 0) {
                    context.resources.getQuantityString(
                        R.plurals.beam_finished,
                        ended.size,
                        ended.size
                    )
                } else {
                    context.resources.getQuantityString(
                        R.plurals.beam_finished_with_problems,
                        ended.size,
                        ended.size,
                        problems
                    )
                }
            }
        }
        val notification = builder().setContentTitle(text).setAutoCancel(true).build()
        manager.notify(SUMMARY_ID, notification)
    }

    private fun describe(status: TransferStatus, state: TransferState.Ended): String =
        when (state) {
            TransferState.Done -> context.getString(
                if (status.direction ==
                    Direction.SENDING
                ) {
                    R.string.beam_sent
                } else {
                    R.string.beam_received
                },
                status.title
            )
            is TransferState.Declined -> context.getString(R.string.beam_declined, status.title)
            is TransferState.Failed -> context.getString(
                R.string.beam_failed,
                status.title,
                state.problem?.let { context.getString(it.explanation) } ?: state.reason
            )
            TransferState.Cancelled -> context.getString(R.string.beam_cancelled, status.title)
        }

    private fun builder(): Notification.Builder = Notification.Builder(context, CHANNEL)
        .setSmallIcon(R.drawable.ic_beam_notification)
        .setCategory(Notification.CATEGORY_PROGRESS)
        .setContentIntent(openApp())

    /** Tapping opens the app, whatever its main screen is. */
    private fun openApp(): PendingIntent? =
        context.packageManager.getLaunchIntentForPackage(context.packageName)?.let { launch ->
            PendingIntent.getActivity(context, 0, launch, PendingIntent.FLAG_IMMUTABLE)
        }

    private fun size(bytes: Long) = Formatter.formatShortFileSize(context, bytes)

    private fun permille(done: Long, total: Long): Int =
        if (total <= 0) 0 else (done.coerceIn(0, total) * PROGRESS_MAX / total).toInt()

    companion object {
        const val CHANNEL = "beam.transfers"
        const val ONGOING_ID = 1
        const val SUMMARY_ID = 2
        private const val PROGRESS_MAX = 1000
    }
}

/** What to tell the person about this problem. */
private val Problem.explanation: Int
    get() = when (this) {
        Problem.VPN_BLOCKS -> R.string.beam_problem_vpn
        Problem.NETWORK_BLOCKS_DEVICES -> R.string.beam_problem_network
        Problem.NOT_RECEIVING -> R.string.beam_problem_not_receiving
        Problem.OUT_OF_REACH -> R.string.beam_problem_out_of_reach
    }
