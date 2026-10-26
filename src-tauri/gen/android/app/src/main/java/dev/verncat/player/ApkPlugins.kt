package dev.verncat.player

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.content.pm.PackageInfo
import android.content.pm.PackageManager
import android.os.Build
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import dalvik.system.PathClassLoader
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.lang.reflect.InvocationTargetException
import java.security.MessageDigest
import java.util.concurrent.Executors
import java.util.zip.ZipFile

@InvokeArg
internal class ApkBinding {
    lateinit var packageName: String
    lateinit var entryClass: String
    lateinit var certificateSha256: String
    lateinit var apkSha256: String
}

@InvokeArg
internal class ApkCallArgs {
    lateinit var pluginId: String
    var generation: Long = 0
    lateinit var binding: ApkBinding
    lateinit var method: String
    var argsJson: String = "{}"
}

@InvokeArg
internal class ApkDeactivateArgs {
    lateinit var pluginId: String
    var generation: Long = 0
}

/** Installed APKs only. Code executes with Player's permissions, in Player's process. */
@TauriPlugin
class ApkPlugins(private val activity: Activity) : Plugin(activity) {
    private companion object {
        val executor = Executors.newSingleThreadExecutor()
        var sharedRuntime: InstalledApkRuntime? = null
    }
    // Activity recreation must not try to load the same JNI library in a new classloader.
    private val runtime = synchronized(executor) {
        sharedRuntime ?: InstalledApkRuntime(activity.applicationContext).also { sharedRuntime = it }
    }

    override fun onDestroy() { executor.execute { runtime.closeAll() } }

    private fun perform(invoke: Invoke, block: () -> JSObject) {
        executor.execute {
            try { invoke.resolve(block()) }
            catch (error: Throwable) {
                // Linkage errors are recoverable; native crashes are process-wide.
                val cause = if (error is InvocationTargetException) error.targetException else error
                invoke.reject((cause.message ?: cause.javaClass.simpleName).take(2000))
            }
        }
    }

    @Command fun discover(invoke: Invoke) = perform(invoke) { runtime.discover() }
    @Command fun call(invoke: Invoke) = perform(invoke) { runtime.call(invoke.parseArgs(ApkCallArgs::class.java)) }
    @Command fun deactivate(invoke: Invoke) = perform(invoke) { runtime.deactivate(invoke.parseArgs(ApkDeactivateArgs::class.java)) }
}

/** Access is serialized by the Tauri adapter. Kept separate for device integration tests. */
internal class InstalledApkRuntime(private val context: Context) {
    // Retain loaders: JNI libraries cannot be safely hot-replaced in a live process.
    private val loaded = mutableMapOf<String, Loaded>()
    private val minimumGeneration = mutableMapOf<String, Long>()

    private data class Descriptor(
        val pkg: PackageInfo, val id: String, val entry: String,
        val certificate: String, val hash: String, val content: String,
    )

    private class Loaded(val hash: String, val loader: ClassLoader, val type: Class<*>) {
        var instance: Any? = null
        var generation: Long = -1
        var failed = false
        val call = type.getMethod("call", String::class.java, String::class.java)
        val close = type.getMethod("close")
    }

    private fun digest(bytes: ByteArray): String =
        MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it) }

    @Suppress("DEPRECATION")
    private fun descriptor(packageName: String): Descriptor {
        val flags = PackageManager.GET_META_DATA or PackageManager.GET_SIGNATURES or
            (if (Build.VERSION.SDK_INT >= 28) PackageManager.GET_SIGNING_CERTIFICATES else 0)
        val pkg = context.packageManager.getPackageInfo(packageName, flags)
        val app = requireNotNull(pkg.applicationInfo)
        require(app.splitSourceDirs.isNullOrEmpty()) { "Split APK plugins are not supported; use a universal APK" }
        val meta = requireNotNull(app.metaData) { "Missing plugin metadata" }
        require(meta.getInt("dev.verncat.player.plugin.API") == 1) { "Unsupported APK plugin API" }
        val id = requireNotNull(meta.getString("dev.verncat.player.plugin.ID"))
        val entry = requireNotNull(meta.getString("dev.verncat.player.plugin.CLASS"))
        require(entry.startsWith("$packageName.")) { "Entrypoint must belong to the plugin package" }
        val signatures = requireNotNull(if (Build.VERSION.SDK_INT >= 28) pkg.signingInfo?.apkContentsSigners else pkg.signatures)
        require(signatures.size == 1) { "APK must have exactly one current signer" }
        val certificate = digest(signatures[0].toByteArray())
        val file = File(app.sourceDir)
        require(file.length() in 1..64L * 1024 * 1024) { "Plugin APK exceeds 64 MiB" }
        val hash = MessageDigest.getInstance("SHA-256").let { hasher ->
            file.inputStream().buffered().use { stream ->
                val buffer = ByteArray(65536)
                while (true) {
                    val count = stream.read(buffer)
                    if (count < 0) break
                    hasher.update(buffer, 0, count)
                }
            }
            hasher.digest().joinToString("") { "%02x".format(it) }
        }
        val content = ZipFile(file).use { zip ->
            val asset = requireNotNull(zip.getEntry("assets/player-plugin.json")) { "Missing assets/player-plugin.json" }
            require(asset.size in 1..4L * 1024 * 1024) { "Plugin package exceeds 4 MiB" }
            val bytes = zip.getInputStream(asset).use { it.readBytesLimited(4 * 1024 * 1024) }
            val data = JSONObject(String(bytes, Charsets.UTF_8))
            val manifest = data.getJSONObject("manifest")
            require(manifest.getString("id") == id) { "Plugin IDs differ in APK metadata and manifest" }
            // Binding is derived from the installed file, never trusted from assets.
            manifest.put("androidNative", JSONObject().apply {
                put("packageName", packageName); put("entryClass", entry)
                put("certificateSha256", certificate); put("apkSha256", hash)
            })
            data.toString()
        }
        return Descriptor(pkg, id, entry, certificate, hash, content)
    }

    fun discover(): JSObject {
        val packages = JSONArray()
        val warnings = JSONArray()
        @Suppress("DEPRECATION")
        val entries = context.packageManager.queryIntentActivities(
            Intent("dev.verncat.player.PLUGIN"), PackageManager.GET_META_DATA,
        ).map { it.activityInfo.packageName }.distinct().sorted().take(128)
        for (name in entries) {
            try { packages.put(descriptor(name).content) }
            catch (error: Exception) { warnings.put("$name: ${error.message}") }
        }
        return JSObject().apply { put("packages", packages); put("warnings", warnings) }
    }

    fun call(args: ApkCallArgs): JSObject {
        require(args.generation >= (minimumGeneration[args.pluginId] ?: 0)) { "Plugin activation has ended" }
        require(args.argsJson.toByteArray(Charsets.UTF_8).size <= 1024 * 1024) { "Request exceeds 1 MiB" }
        val installed = descriptor(args.binding.packageName)
        require(installed.id == args.pluginId && installed.entry == args.binding.entryClass &&
            installed.certificate == args.binding.certificateSha256 && installed.hash == args.binding.apkSha256
        ) { "Installed APK changed. Disable the plugin, scan APK plugins again and review the update" }
        val module = loaded[args.pluginId]?.also {
            require(it.hash == installed.hash) { "APK updated: restart Player before loading the new native library" }
            require(!it.failed) { "Native lifecycle failed: restart Player" }
        } ?: run {
            val app = requireNotNull(installed.pkg.applicationInfo)
            val loader = PathClassLoader(app.sourceDir, app.nativeLibraryDir, context.classLoader)
            val type = Class.forName(installed.entry, false, loader)
            require(type.classLoader === loader) { "Entrypoint resolved outside the APK" }
            Loaded(installed.hash, loader, type).also { loaded[args.pluginId] = it }
        }
        if (module.instance == null || module.generation != args.generation) {
            stop(module)
            try {
                val instance = module.type.getDeclaredConstructor().newInstance()
                module.instance = instance
                require(module.type.getMethod("apiVersion").invoke(instance) == 1) { "Unsupported native API" }
                require(module.type.getMethod("pluginId").invoke(instance) == args.pluginId) { "Native plugin ID mismatch" }
                module.generation = args.generation
            } catch (error: Throwable) {
                module.failed = true
                throw error
            }
        }
        val response = module.call.invoke(module.instance, args.method, args.argsJson) as? String
            ?: error("Native call must return JSON as a string")
        require(response.toByteArray(Charsets.UTF_8).size <= 1024 * 1024) { "Response exceeds 1 MiB" }
        val parsed = org.json.JSONTokener(response)
        val result = parsed.nextValue()
        require(parsed.nextClean() == '\u0000') { "Trailing data in native response" }
        return JSObject().apply { put("result", result) }
    }

    private fun stop(module: Loaded) {
        val instance = module.instance ?: return
        module.instance = null
        module.generation = -1
        try { module.close.invoke(instance) }
        catch (error: Throwable) { module.failed = true; throw error }
    }

    fun deactivate(args: ApkDeactivateArgs): JSObject {
        minimumGeneration[args.pluginId] = maxOf(minimumGeneration[args.pluginId] ?: 0, args.generation)
        loaded[args.pluginId]?.let { stop(it) }
        return JSObject()
    }

    fun closeAll() {
        for (module in loaded.values) {
            try { stop(module) } catch (_: Throwable) { /* Failed modules stay blocked until process restart. */ }
        }
    }
}

private fun java.io.InputStream.readBytesLimited(limit: Int): ByteArray {
    val output = java.io.ByteArrayOutputStream()
    val buffer = ByteArray(8192)
    while (true) {
        val count = read(buffer)
        if (count < 0) break
        require(output.size() + count <= limit) { "Plugin asset exceeds limit" }
        output.write(buffer, 0, count)
    }
    return output.toByteArray()
}
