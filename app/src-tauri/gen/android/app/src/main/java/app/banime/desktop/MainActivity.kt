package app.banime.desktop

import android.os.Bundle
import android.webkit.JavascriptInterface
import android.webkit.WebView
import androidx.activity.enableEdgeToEdge
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
  }

  // The player's full screen. A web page can't hide Android's status and navigation bars, so the
  // page calls window.BAnimeAndroid.setImmersive(true/false) and the activity hides them.
  override fun onWebViewCreate(webView: WebView) {
    webView.addJavascriptInterface(SystemBars(), "BAnimeAndroid")
    // The next episode starts by itself, like on the PC. By default Android's WebView only plays
    // a video after a tap, so play() at the start of each episode was refused.
    webView.settings.mediaPlaybackRequiresUserGesture = false
  }

  inner class SystemBars {
    @JavascriptInterface
    fun setImmersive(on: Boolean) {
      runOnUiThread {
        val bars = WindowCompat.getInsetsController(window, window.decorView)
        if (on) {
          // A swipe from the edge shows them for a moment, then they hide again.
          bars.systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
          bars.hide(WindowInsetsCompat.Type.systemBars())
        } else {
          bars.show(WindowInsetsCompat.Type.systemBars())
        }
      }
    }
  }
}
