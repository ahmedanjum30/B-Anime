package app.banime.desktop

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder

/**
 * Keeps B-Anime running while it's in a Watch together room and in the background: the video
 * and its sound keep playing and the room stays connected. Without it Android freezes the app
 * soon after it leaves the screen. Shows a notification while it runs; tapping it opens the app.
 */
class RoomService : Service() {
  override fun onBind(intent: Intent?): IBinder? = null

  override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
    val manager = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
      manager.createNotificationChannel(
        NotificationChannel(CHANNEL, "Watch together", NotificationManager.IMPORTANCE_LOW).apply {
          description = "Shown while you're in a room, so it keeps playing in the background."
          setShowBadge(false)
        },
      )
    }
    val open = PendingIntent.getActivity(
      this,
      0,
      Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
      PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )
    val builder =
      if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) Notification.Builder(this, CHANNEL)
      else @Suppress("DEPRECATION") Notification.Builder(this)
    val notification = builder
      .setSmallIcon(android.R.drawable.ic_media_play)
      .setContentTitle("Watching together")
      .setContentText("B-Anime keeps playing while it's in the background.")
      .setContentIntent(open)
      .setOngoing(true)
      .build()
    runCatching {
      if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
        startForeground(ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK)
      } else {
        startForeground(ID, notification)
      }
    }.onFailure { stopSelf() }
    return START_NOT_STICKY
  }

  companion object {
    private const val CHANNEL = "watch-together"
    private const val ID = 7301
  }
}
