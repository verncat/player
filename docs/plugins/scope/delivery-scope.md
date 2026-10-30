# Delivery scope: сборка и проверка

Зависимости: все [scope](../../plugin/readme.md#документы-по-областям).
Изменение считается готовым после проверки контрактов ниже. Live-интеграции
проверяются отдельно от детерминированных тестов; отсутствие клиента/аккаунта
не заменяется утверждением об успешном live-тесте.

## Последовательность разработки

| Этап | Результат и зависимость |
| --- | --- |
| Реестр и пакет | Валидация package, настройки, права, install/update/remove |
| Runtime | Изолированный Worker, RPC, lifecycle, поколения, failure/reload |
| Общий UI | Pages/settings/search/actions/widgets, renderer и менеджер |
| Медиа API | Source handles, GrowingFile, transfer states, импорт/замена |
| Discord | Пакет и native adapter используют общий playback contract |
| Soulseek | Пакет и native adapter используют общий UI/media contract |
| Миграции | Старые пользовательские настройки не теряются |
| Сборки | Полная, без каждого адаптера и без обоих; Android compile |

## Автономные проверки

```sh
pnpm test:plugins
pnpm test:plugin-sdk
pnpm test:playback-session
pnpm test:seek-bar
pnpm exec vue-tsc --noEmit
pnpm exec vite build
cargo test --manifest-path src-tauri/Cargo.toml --lib
cargo check --manifest-path src-tauri/Cargo.toml --no-default-features --lib
cargo check --manifest-path src-tauri/Cargo.toml --no-default-features --features soulseek --lib
cargo check --manifest-path src-tauri/Cargo.toml --no-default-features --features discord-rpc --lib
```

`tests/plugins.test.mjs` исполняет настоящий SDK prelude и scripts плагинов
в автономном JS-контексте, моделируя только внешние/native-вызовы.
Native тесты проверяют packages, permissions, persistence/CAS/generation и
существующий Discord engine. Регрессии session/seek проверяют основной плеер.

## Матрица интеграционной приёмки

- Установить собранный Hello package без новой сборки Player: открыть страницу,
  сменить greeting, увидеть playback event, reload, disable, update, uninstall.
- Загрузить пакет по repo/tree/blob/package URL; проверить author/links и pinned
  commit, подтвердить установку, перезапустить и проверить сохранённый источник.
- Ошибочный script/activate, таймаут, late response и запрещённый capability
  не должны затрагивать второй плагин или библиотеку.
- Discord: отсутствующий клиент, поздний запуск, смена/seek/pause, reload,
  disable/enable, shutdown.
- Soulseek: логин, поиск, cover failure, remote queue, preview buffer,
  cancel/retry, reload, save, replace, disable во время сети.
- Любой источник: проигрывание/пауза переживают UI reload без нового play;
  импорт проигрываемого файла не ломает открытый decoder на Windows.
- Проверить работу settings/secrets на каждой OS. Android Keystore и Windows
  Credential Manager требуют соответствующей среды; macOS-тест их не заменяет.
- Убедиться, что пакеты/секреты не попадают в музыкальный sync и registry не
  содержит plaintext password. Legacy миграция повторяема после ошибки provider.
- Импорт обычного файла и CUE, очередь, history/likes/playlists остаются рабочими
  в сборке без Soulseek и Discord.

## Границы версии

Полная система API 1 включает установку JavaScript-пакетов, сетевой каталог из
репозитория, общий UI, права/хранилище, медиа API и оба требуемых native-адаптера. Подписи
издателей, automatic update, OS sandbox, desktop native binaries, произвольные
Vue/HTML, custom codecs/PCM и resumable downloads после app restart — отдельные
расширения протокола. Эти возможности нельзя заявлять в UI до реализации.

Отдельно реализован эксперимент Android APK / Kotlin / Rust:
[контракт и проверки](android-native-scope.md),
[сборка и запуск примера](../../../plugins/examples/android-native/README.md).
Он поддерживает уже установленные монолитные APK и требует доверия native-коду.
Установка APK по URL, private APK loading и перенос Soulseek остаются вне прототипа.

## Отдельный репозиторий пакетов

Пакеты, SDK и инструменты перенесены в [player-plugins](https://github.com/verncat/player-plugins).
Player хранит только gitlink `plugins/`, фиксирующий проверенный коммит пакетов.
Перед сборкой: `git submodule update --init -- plugins`. Release workflow
загружает именно этот submodule; upstream Soulseek workbench для сборки не нужен.
Для обновления встроенных пакетов обновите checkout `plugins/` до выбранного
коммита и закоммитьте gitlink в Player. Каталог загружается независимо из
`player-plugins/main/catalog.json`, без изменения закреплённого кода.
