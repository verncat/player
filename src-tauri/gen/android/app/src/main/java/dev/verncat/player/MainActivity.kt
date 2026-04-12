package dev.verncat.player

import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.Environment
import android.provider.Settings
import android.webkit.JavascriptInterface
import android.webkit.WebView
import androidx.activity.enableEdgeToEdge

class MainActivity : TauriActivity() {

  override fun onWebViewCreate(webView: WebView) {
    super.onWebViewCreate(webView)
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
    }, "AndroidBridge")
  }

  override fun onCreate(savedInstanceState: Bundle?) {
    // Request MANAGE_EXTERNAL_STORAGE on Android 11+ so we can use /sdcard/Player/
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
      if (!Environment.isExternalStorageManager()) {
        val intent = Intent(Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION).apply {
          data = Uri.parse("package:$packageName")
        }
        startActivity(intent)
      }
    }
    // Ensure /sdcard/Player/ exists before Rust init
    Environment.getExternalStorageDirectory()
      .resolve("Player")
      .mkdirs()
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
  }
}
