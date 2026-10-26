# System scope: архитектура

Статус: нормативный контракт API 1. Точка входа:
[readme](../../plugin/readme.md). Область отвечает за границы модулей и платформ.

## Архитектура

```text
.package → Rust registry → frontend broker → отдельный Worker плагина
                                  ↓                 ↓ JSON RPC
                           Vue UI renderer ← декларации / действия
                                  ↓
                              host gateway
                                  ↓
                библиотека / аудио / настройки / native adapters
```

JavaScript-пакеты обнаруживаются из native-реестра. Для каждого включённого
пакета broker запускает Worker из его script и переносимого SDK prelude.
Все вызовы привязаны к ID пакета и поколению активации. Worker не выбирает
идентичность вызывающей стороны. UI никогда не выполняет полученный HTML/JS.

## Владение состоянием

- Rust registry: установленные пакеты, enabled, generation, настройки и revision.
- Native adapters: сетевые сессии, Discord IPC, активные передачи Soulseek.
- Аудиодвижок: источник, позиция, transport, decode/output, identity запуска.
- Оболочка Player: очередь, навигация, remote output и сводные playback snapshots.
- Worker: логика плагина, представления и обработчики. Устойчивое состояние
  Worker сохраняет через settings API; оперативная память исчезает при reload.

Ядро не содержит ветвлений по ID плагина при рисовании страниц, поиска,
настроек, контекстных действий или виджетов. Библиотека не импортирует код
Soulseek. Декодер знает только файл, CUE-сегмент и общий GrowingFile.

Rust-адаптеры размещены в `src-tauri/src/plugins`; дополнительные сервисы
проходят через gateway. `native.discord` и `native.soulseek` зарезервированы
для встроенных пакетов. На desktop изменение native-набора требует сборки
Player; на Android есть эксперимент установки отдельного APK с permission
`native.android`: [контракт](android-native-scope.md).
Добавление JavaScript-пакета выполняется через менеджер.

## Платформы

| Возможность | macOS / Windows / Linux | Android |
| --- | --- | --- |
| Установка JavaScript-пакета | Да | Да |
| Декларативный UI и SDK | Да | Да |
| Soulseek adapter | Feature `soulseek` | Feature `soulseek` |
| Discord IPC | Feature `discord-rpc` | Недоступен |
| Native runtime нового плагина | Включается в сборку | Встроенный адаптер либо экспериментальный APK / Kotlin / JNI |

Desktop-загрузчика сторонних native-библиотек пока нет. Нет OS sandbox, Node.js,
произвольных Vue-компонентов или воспроизведения plugin PCM в realtime.

## Инварианты при приёмке

1. Пример нового пакета устанавливается и добавляет UI без изменения оболочки.
2. Без обеих Cargo features работают библиотека, импорт, CUE и локальное аудио.
3. Ошибка внешнего сервиса не останавливает аудиодвижок или другой Worker;
   crash сторонней APK `.so` может завершить процесс Player.
4. Reload UI не перезапускает native-аудио или начатую native-передачу.
5. Native-код не вызывает Worker или сетевой API из аудиопотока.
6. Установленный пакет использует тот же SDK, что встроенные плагины.

Обоснование границ Tauri: [plugin development](https://v2.tauri.app/develop/plugins/),
[security](https://tauri.app/security/). Это сведения о Tauri;
выбор Worker runtime и состава SDK — архитектура Player.
