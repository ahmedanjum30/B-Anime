package app.banime.desktop

import android.Manifest
import android.app.PictureInPictureParams
import android.content.Intent
import android.content.pm.ActivityInfo
import android.content.pm.PackageManager
import android.content.res.Configuration
import android.os.Build
import android.os.Bundle
import android.util.Rational
import android.webkit.JavascriptInterface
import android.webkit.WebView
import androidx.activity.enableEdgeToEdge
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import androidx.lifecycle.Lifecycle

class MainActivity : TauriActivity() {
  private var webView: WebView? = null
  /** In a Watch together room: the page keeps running when the app leaves the screen. */
  @Volatile private var keepAlive = false

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
  }

  // The player asks for what a web page can't do itself, through window.BAnimeAndroid.
  override fun onWebViewCreate(webView: WebView) {
    this.webView = webView
    webView.addJavascriptInterface(Player(), "BAnimeAndroid")
    // The next episode starts by itself, like on the PC. By default Android's WebView only plays
    // a video after a tap, so play() at the start of each episode was refused.
    webView.settings.mediaPlaybackRequiresUserGesture = false
  }

  inner class Player {
    /** The screen stays on while the episode plays (the WebView doesn't keep it on for video). */
    @JavascriptInterface
    fun setKeepScreenOn(on: Boolean) {
      runOnUiThread {
        if (on) window.addFlags(android.view.WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        else window.clearFlags(android.view.WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
      }
    }

    /** The app is closing (swiped away): its video's pause isn't one to tell the room about. */
    @JavascriptInterface
    fun isClosing(): Boolean = this@MainActivity.isFinishing || this@MainActivity.isDestroyed

    /**
     * In a room (or not): while in one, the app keeps playing and stays connected in the
     * background, with a notification (RoomService).
     */
    @JavascriptInterface
    fun setKeepAlive(on: Boolean) {
      runOnUiThread {
        if (keepAlive == on) return@runOnUiThread
        keepAlive = on
        val service = Intent(this@MainActivity, RoomService::class.java)
        if (on) {
          // Android 13 and later show the notification only once allowed (asked once).
          if (Build.VERSION.SDK_INT >= 33 &&
            checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
          ) {
            requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), 7301)
          }
          runCatching {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) startForegroundService(service) else startService(service)
          }
        } else {
          stopService(service)
        }
      }
    }

    /** Full screen: hides the status and navigation bars and turns the screen to landscape. */
    @JavascriptInterface
    fun setImmersive(on: Boolean) {
      runOnUiThread {
        val bars = WindowCompat.getInsetsController(window, window.decorView)
        if (on) {
          // A swipe from the edge shows them for a moment, then they hide again.
          bars.systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
          bars.hide(WindowInsetsCompat.Type.systemBars())
          // Either landscape side, following how the phone is held.
          requestedOrientation = ActivityInfo.SCREEN_ORIENTATION_SENSOR_LANDSCAPE
        } else {
          bars.show(WindowInsetsCompat.Type.systemBars())
          requestedOrientation = ActivityInfo.SCREEN_ORIENTATION_UNSPECIFIED
        }
      }
    }

    /**
     * Picture in picture: the app shrinks to a small window over other apps, showing the video.
     * Android's WebView has none of its own, so the activity does it. False before Android 8.
     */
    @JavascriptInterface
    fun enterPip(width: Int, height: Int): Boolean {
      if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return false
      runOnUiThread {
        // Android accepts ratios between 1:2.39 and 2.39:1.
        val ratio = if (width > 0 && height > 0) width.toDouble() / height else 16.0 / 9
        val clamped = ratio.coerceIn(1 / 2.39, 2.39)
        val params = PictureInPictureParams.Builder()
          .setAspectRatio(Rational((clamped * 1000).toInt(), 1000))
          .build()
        runCatching { enterPictureInPictureMode(params) }
      }
      return true
    }
  }

  // In picture in picture the activity is paused but still on screen, and in a room it keeps
  // playing in the background. The generated activity pauses the WebView then, which would stop
  // the video, so it's resumed straight away.
  override fun onPause() {
    super.onPause()
    if (keepAlive || (Build.VERSION.SDK_INT >= Build.VERSION_CODES.N && isInPictureInPictureMode)) webView?.onResume()
  }

  override fun onDestroy() {
    if (keepAlive) stopService(Intent(this, RoomService::class.java))
    super.onDestroy()
  }

  // Tells the page: "on" (shrunk), "off" (back to full size) or "closed" (the small window was
  // dismissed, so playback stops).
  override fun onPictureInPictureModeChanged(isInPictureInPictureMode: Boolean, newConfig: Configuration) {
    super.onPictureInPictureModeChanged(isInPictureInPictureMode, newConfig)
    val state = when {
      isInPictureInPictureMode -> "on"
      !lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED) -> "closed"
      else -> "off"
    }
    webView?.evaluateJavascript("window.dispatchEvent(new CustomEvent('banime-pip', { detail: '$state' }))", null)
  }
}
