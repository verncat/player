package dev.verncat.player

import android.app.Activity
import android.content.Intent
import android.net.Uri
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

@InvokeArg
internal class OpenFolderArgs {
    lateinit var path: String
}

@TauriPlugin
class FolderPlugin(private val activity: Activity) : Plugin(activity) {

    @Command
    fun openFolder(invoke: Invoke) {
        val args = invoke.parseArgs(OpenFolderArgs::class.java)
        try {
            val uri = Uri.parse("file://${args.path}")
            val intent = Intent(Intent.ACTION_VIEW).apply {
                setDataAndType(uri, "resource/folder")
                addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            }
            activity.startActivity(Intent.createChooser(intent, "Open folder"))
            invoke.resolve()
        } catch (e: Exception) {
            invoke.reject(e.message ?: "No file manager found")
        }
    }
}
