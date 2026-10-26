# Android APK / Rust example

Эксперимент самостоятельного native-плагина. UI работает в стандартном Worker,
APK содержит Kotlin entrypoint и Rust JNI-библиотеку. APK не добавляется в сборку
Player. Контракт и ограничения: [android-native scope](../../../docs/plugins/scope/android-native-scope.md).

## Сборка

Нужны Android SDK/NDK, JDK 17+, Rust Android target и Node.js. Из корня Player:

```sh
export ANDROID_HOME="$HOME/Library/Android/sdk" # путь вашего SDK
rustup target add aarch64-linux-android
pnpm plugins:android:build
```

Результат: `plugins/examples/android-native/build/outputs/apk/debug/android-native-debug.apk`.
Debug APK подписан локальным debug key. Для публикации настройте release signing
собственным постоянным ключом; `.player-plugin` packer здесь не устанавливает APK.
Cargo.lock включён, Rust собирается через NDK clang без зависимости от cargo-ndk.

Другие ABI можно выбрать свойством Gradle; сначала установите их Rust targets:

```sh
./src-tauri/gen/android/gradlew -p plugins/examples/android-native assembleDebug -PpluginAbis=arm64-v8a,x86_64
```

## Подключение

Соберите актуальный Android Player обычным `pnpm android:build`. Для native
Opus-зависимости Player задайте `ANDROID_NDK_HOME` на каталог установленной
версии NDK внутри `$ANDROID_HOME/ndk`. Установите
Player и APK-пример через системный installer или adb, затем:

1. Settings → Plugins → Find APK plugins.
2. Native example → Review connection; проверьте сведения и разрешение native-кода.
3. Install and grant these permissions; включите плагин.
4. На странице Native example должен появиться `Rust via JNI` и архитектура.
5. Введите сообщение и нажмите Call Rust; счётчик вызовов увеличится.

При обновлении APK отключите подключение, пересканируйте и подтвердите новую
версию. Если `.so` уже загружалась, полностью перезапустите процесс Player.
Удаление подключения в Player не удаляет установленный Android APK.

## Проверка

```sh
cargo test --manifest-path plugins/examples/android-native/native/Cargo.toml
pnpm test:plugins
adb -s emulator-5554 install -r plugins/examples/android-native/build/outputs/apk/debug/android-native-debug.apk
```

После сборки и установки актуального Player и APK-примера, из `src-tauri/gen/android`:

```sh
./gradlew :app:assembleArm64DebugAndroidTest -x :app:rustBuildArm64Debug
adb -s emulator-5554 install -r app/build/outputs/apk/androidTest/arm64/debug/app-arm64-debug-androidTest.apk
adb -s emulator-5554 shell am instrument -w -e class dev.verncat.player.ApkPluginsTest dev.verncat.player.test/androidx.test.runner.AndroidJUnitRunner
```

Исключение rustBuild в этой команде допустимо только после нормальной сборки
Player: Gradle иначе требует запущенного Tauri CLI build server.
Тест вызывает настоящий Kotlin-загрузчик и Rust из установленного APK.
