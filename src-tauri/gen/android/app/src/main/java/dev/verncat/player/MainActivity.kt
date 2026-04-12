package dev.verncat.player

import android.content.Intent
import android.net.Uri
import android.net.wifi.WifiManager
import android.os.Build
import android.os.Bundle
import android.os.Environment
import android.provider.Settings
import android.webkit.JavascriptInterface
import android.webkit.WebView
import androidx.activity.enableEdgeToEdge
import java.lang.ref.WeakReference

class MainActivity : TauriActivity() {

    private var webViewRef: WeakReference<WebView>? = null
    private var multicastLock: WifiManager.MulticastLock? = null

    companion object {
        var instance: WeakReference<MainActivity>? = null
    }

    /** Called by MediaPlaybackService to forward notification button presses to JS. */
    fun sendToJs(action: String) {
        webViewRef?.get()?.post {
            webViewRef?.get()?.evaluateJavascript(
                "window._mediaControl && window._mediaControl('$action')", null
            )
        }
    }

    override fun onWebViewCreate(webView: WebView) {
        super.onWebViewCreate(webView)
        webViewRef = WeakReference(webView)
        webView.addJavascriptInterface(object : Any() {

            @JavascriptInterface
            fun openFolder(relativePath: String) {
                val encoded = Uri.encode(relativePath)
                val uri = Uri.parse(
                    "content://com.android.externalstorage.documents/document/primary%3A$encoded"
                )
                val intent = Intent(Intent.ACTION_VIEW).apply {
                    setDataAndType(uri, "vnd.android.document/directory")
                    addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                }
                try { startActivity(intent) } catch (_: Exception) {}
            }

            /** Called from JS when track/play state changes — starts/updates foreground service. */
            @JavascriptInterface
            fun updatePlayback(title: String, artist: String, isPlaying: Boolean, positionSec: Long, durationSec: Long) {
                val intent = Intent(this@MainActivity, MediaPlaybackService::class.java).apply {
                    action = MediaPlaybackService.ACTION_UPDATE
                    putExtra(MediaPlaybackService.EXTRA_TITLE, title)
                    putExtra(MediaPlaybackService.EXTRA_ARTIST, artist)
                    putExtra(MediaPlaybackService.EXTRA_IS_PLAYING, isPlaying)
                    putExtra(MediaPlaybackService.EXTRA_POSITION, positionSec * 1000L)
                    putExtra(MediaPlaybackService.EXTRA_DURATION, durationSec * 1000L)
                }
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                    startForegroundService(intent)
                } else {
                    startService(intent)
                }
            }

            /** Called from JS when playback stops entirely. */
            @JavascriptInterface
            fun stopPlayback() {
                stopService(Intent(this@MainActivity, MediaPlaybackService::class.java))
            }
        }, "AndroidBridge")
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        instance = WeakReference(this)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            if (!Environment.isExternalStorageManager()) {
                val intent = Intent(Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION).apply {
                    data = Uri.parse("package:$packageName")
                }
                startActivity(intent)
            }
        }
        Environment.getExternalStorageDirectory().resolve("Player").mkdirs()
        enableEdgeToEdge()

        // Acquire Wi-Fi multicast lock so mDNS (mdns-sd) can receive multicast packets
        val wm = applicationContext.getSystemService(WIFI_SERVICE) as WifiManager
        multicastLock = wm.createMulticastLock("player_mdns").apply {
            setReferenceCounted(true)
            acquire()
        }

        super.onCreate(savedInstanceState)
    }

    override fun onDestroy() {
        super.onDestroy()
        multicastLock?.release()
        multicastLock = null
    }
}
