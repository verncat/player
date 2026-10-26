import groovy.json.JsonOutput
import groovy.json.JsonSlurper

plugins {
    id("com.android.application") version "8.11.0"
    id("org.jetbrains.kotlin.android") version "1.9.25"
}

val pluginAbis = (findProperty("pluginAbis") as? String ?: "arm64-v8a").split(',')
val generatePluginAssets by tasks.registering {
    inputs.files("plugin.json", "main.js")
    outputs.dir(layout.buildDirectory.dir("generated/pluginAssets"))
    doLast {
        val output = layout.buildDirectory.file("generated/pluginAssets/player-plugin.json").get().asFile
        output.parentFile.mkdirs()
        output.writeText(JsonOutput.toJson(mapOf(
            "manifest" to JsonSlurper().parse(file("plugin.json")), "script" to file("main.js").readText(),
        )))
    }
}
val buildPluginRust by tasks.registering(Exec::class) {
    inputs.dir("native/src")
    inputs.file("native/Cargo.toml")
    inputs.file("native/Cargo.lock")
    inputs.property("pluginAbis", pluginAbis)
    inputs.file("../../../scripts/build-android-plugin.mjs")
    outputs.dir(layout.buildDirectory.dir("generated/jniLibs"))
    commandLine("node", "../../../scripts/build-android-plugin.mjs", projectDir.absolutePath, pluginAbis.joinToString(","))
}

android {
    namespace = "dev.verncat.player.example.nativeplugin"
    compileSdk = 36
    defaultConfig {
        applicationId = "dev.verncat.player.example.nativeplugin"
        minSdk = 26
        targetSdk = 36
        versionCode = 1
        versionName = "0.1.0"
        ndk { abiFilters += pluginAbis }
    }
    buildTypes {
        getByName("release") {
            isMinifyEnabled = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
        }
    }
    sourceSets.getByName("main") {
        assets.srcDir(layout.buildDirectory.dir("generated/pluginAssets"))
        jniLibs.srcDir(layout.buildDirectory.dir("generated/jniLibs"))
    }
    // The loader searches the installed APK's nativeLibraryDir.
    packaging.jniLibs.useLegacyPackaging = true
    kotlinOptions { jvmTarget = "1.8" }
}
tasks.named("preBuild") { dependsOn(generatePluginAssets, buildPluginRust) }
