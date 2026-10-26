# Android native scope: APK-расширения

Статус: экспериментальная реализация поверх API 1. Общая система:
[readme](../../plugin/readme.md). Пример:
[android-native](../../../plugins/examples/android-native/README.md).

## Граница эксперимента

Расширение устанавливается системой Android как самостоятельный APK. Player
читает его manifest/script и после подтверждения подключает Kotlin-класс через
`PathClassLoader`. Класс может загрузить собственную Rust `.so` через JNI.
Пересборка Player для нового плагина не нужна. Отдельный Service не требуется:
код выполняется внутри процесса Player с его Android-разрешениями.

Реализованы обнаружение, preview/установка в реестр, включение, native-вызовы,
закрытие экземпляра и повторное включение. Обычный JS Worker и декларативный UI
остаются прежними. Soulseek и Discord пока используют существующие встроенные
адаптеры; пример подтверждает механизм загрузки, а не миграцию их функций.

Приватная установка APK внутри Player, скачивание/установка APK из URL,
магазин, desktop-загрузчик DLL и полноценный native media API сюда не входят.
Кнопка Find APK plugins ищет уже установленные пакеты. Uninstall в Player
удаляет подключение из реестра; APK удаляется средствами Android отдельно.

## Формат APK

APK должен быть монолитным, без split APK; размер до 64 MiB.
Минимальная версия Android — API 26. Пример по умолчанию собирается для arm64;
автор добавляет нужные ABI. Native-библиотеки должны извлекаться системным
установщиком: Gradle `packaging.jniLibs.useLegacyPackaging = true`.

В application manifest:

```xml
<meta-data android:name="dev.verncat.player.plugin.API" android:value="1" />
<meta-data android:name="dev.verncat.player.plugin.ID" android:value="example.android-native" />
<meta-data android:name="dev.verncat.player.plugin.CLASS"
    android:value="dev.verncat.player.example.nativeplugin.NativePlugin" />
```

Для обнаружения нужна exported Activity с intent action
`dev.verncat.player.PLUGIN`. Player её не запускает. У хоста есть соответствующий
`queries/intent`, без QUERY_ALL_PACKAGES. Пример использует пустую Activity,
которая сразу завершается и не имеет launcher icon.

В `assets/player-plugin.json` — стандартный `{manifest,script}`. Manifest
объявляет `platforms: ["android"]`, `permissions: ["ui","native.android"]`.
Скрипт использует обычные contributions; размер пакета с добавленным binding
не больше 4 MiB. APK asset сам по себе не устанавливается стандартным packer:
его manifest получает обязательный binding при сканировании.

Хост формирует `manifest.androidNative` из установленного APK:

```json
{
  "packageName": "dev.verncat.player.example.nativeplugin",
  "entryClass": "dev.verncat.player.example.nativeplugin.NativePlugin",
  "certificateSha256": "64 lowercase hex characters",
  "apkSha256": "64 lowercase hex characters"
}
```

Binding и permission обязательны вместе. Entry class находится в namespace
APK-пакета. APK имеет ровно одного текущего signer. При сканировании его код
не загружается; читаются package metadata, подпись, hash и ZIP asset.
Невалидный APK не мешает обнаружению других: причина показывается в UI.

## Native ABI 1

Класс с публичным конструктором без аргументов предоставляет методы:

```kotlin
fun apiVersion(): Int           // 1
fun pluginId(): String          // тот же ID, что в manifest
fun call(method: String, argsJson: String): String // результат как JSON
fun close()                    // остановить workers, закрыть sockets и файлы
```

Общий SDK-класс в APK не требуется: контракт вызывается reflection, payload
передаётся JSON. R8/ProGuard должен сохранять имя класса, ABI-методы и JNI-имена.
`apiVersion` и `pluginId` проверяются после создания экземпляра; конструктор
уже является исполнением доверенного кода. `close` должен быть быстрым,
освобождать всё состояние экземпляра и не бросать исключения.

В JavaScript:

```js
const result = await ctx.android.call('echo', {message: 'Привет'});
// Эквивалент ctx.call('android.call', {method: 'echo', args: {message: 'Привет'}})
```

Rust gateway выбирает binding из собственного реестра, а не из аргументов
Worker. Проверяет ID, permission, generation и платформу до вызова и повторно
после результата. Имя метода — 1–100 символов; JSON request/response до 1 MiB.
Двоичные медиа-данные через этот канал не передавать: file handles/staging и
потоковый источник для внешнего native-плагина потребуют отдельного контракта.

Kotlin-часть выполняет команды последовательно на отдельном executor; основной
поток Android не занят JNI-работой. Долгие операции плагин запускает на своих
workers и возвращает ID задания, затем допускает polling/cancel через `call`.
Native-код не должен бросать Rust panic через FFI. Пример использует
`catch_unwind` и JSON-ошибки. Таймаут Worker не прерывает исполняющийся JNI-код.

## Lifecycle и обновление

Перед каждым вызовом повторно проверяются текущие package metadata, подпись и
SHA-256 APK. Если APK удалён или заменён, вызов заканчивается ошибкой. Для
новой версии нужны disable → scan → review/install → перезапуск Player → enable.
Даже APK с прежней подписью не получает автоматического доверия новой версии.

ClassLoader и загруженная `.so` удерживаются до завершения процесса. Отключение
сначала отзывает generation в Rust, затем вызывает `close` экземпляра в Kotlin.
Kotlin запоминает нижнюю допустимую generation; запоздалый запрос не открывает
отключённый экземпляр заново. При включении создаётся новый экземпляр через тот
же ClassLoader. Замена уже загруженного APK требует перезапуска процесса.
Ошибка конструктора/handshake/close блокирует модуль до перезапуска.
При уничтожении Android Activity экземпляры закрываются; ClassLoader сохраняется
на время процесса. Работа native-плагина в фоне после закрытия Activity в этой
версии не гарантируется.

Это кооперативный lifecycle: зависший JNI-вызов задерживает executor и close.
Произвольно написанный native-код нельзя принудительно остановить без завершения
процесса. Сбой `.so` может завершить Player. Permission `native.android` означает
доверие всему native-коду; остальные permissions не ограничивают его доступ к
памяти, сети и файлам. UI сообщает это при подтверждении подключения.

## Проверки

- Rust: pairing binding/permission, Android-only, корректность имён и digest;
  совместимость старого fingerprint без `androidNative`.
- JS: активация/вызов/рендер настоящего примера через SDK, ошибка после revoke.
- Android instrumentation: поиск установленного APK, pinning подписи и hash до
  исполнения, реальный вызов Rust через JNI, Unicode JSON, stale generation,
  повторное включение с уже загруженной native-библиотекой.
- Вручную: обновление APK, повторный review, требование restart, неправильный ABI,
  отсутствие библиотеки, снятие подключения без удаления APK из системы.

Проверено 2026-10-26 на Android 14 / API 34 / arm64 эмуляторе: instrumentation
test с настоящей `.so`, затем сквозной путь Settings → Find APK plugins → review →
install → enable → страница → Call Rust. Unicode-сообщение прошло через Worker,
Tauri Rust gateway, Kotlin и JNI, результат появился в UI. После disable старое
поколение отвергнуто. Другие версии Android и ABI этим прогоном не покрыты.
При UI-проверке исправлена передача Vue proxies в Worker: broker нормализует
все сообщения к JSON; регрессионный тест проверяет вложенные reactive context,
action payload и playback snapshot.

Основание Android-механизма:
[PathClassLoader](https://developer.android.com/reference/dalvik/system/PathClassLoader),
[JNI](https://developer.android.com/ndk/guides/jni-tips),
[package visibility](https://developer.android.com/training/package-visibility).
