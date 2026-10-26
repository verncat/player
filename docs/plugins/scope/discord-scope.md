# Discord RPC scope

Пакет: `plugins/discord-rpc`; адаптер: `src-tauri/src/plugins/discord.rs`.
Зависимости: runtime, playback.read, native.discord, UI settings.
Feature `discord-rpc`; поддержка macOS/Windows/Linux. По умолчанию отключён.

## Функциональность

Worker подписывается на общий playback snapshot, отбрасывает повторные
обновления, отправляет meaningful change в `discord.update({playback})`.
Playback — title/artist/album/position/duration/playing; на pause/stop передаётся
null. Native worker владеет IPC, reconnect и time stamps. JS не открывает
Discord socket самостоятельно и не управляет транспортом плеера.

Существующий публичный application ID сохраняется; `DISCORD_CLIENT_ID`
переопределяет его при сборке. Обложки/управление Discord-плеером не входят
в текущий функционал.

Native поведение:

- публикация не чаще раза за 5 секунд;
- retry через 15 секунд, heartbeat через 30 секунд;
- после reconnect отправляется последнее желаемое состояние;
- seek пересчитывает временные метки;
- pause/stop/disable очищают активность;
- отсутствие клиента Discord не мешает аудио или включению плагина;
- выключенный адаптер не публикует состояние; финальный app shutdown
  завершает native worker и закрывает IPC.

Отключение пакета отзывает его capability и передаёт native enabled=false.
Native worker может оставаться ожидающим до выхода приложения и повторно
используется при включении, без дублирующихся потоков. Worker JS всегда новый.
Reload UI не очищает native-активность: после attach приходит новый snapshot.

## UI и миграция

Пакет объявляет settings section `general`; UI показывает назначение,
необходимость desktop Discord и ошибки обновления. Основной switch включения
находится в Settings → Plugins. Отдельных Discord props в SettingsView нет.

Legacy `player.discord-presence` переносится из localStorage в native enabled
один раз. Устойчивый migrated marker препятствует повторной перезаписи настроек.
Заблокированный localStorage не препятствует старту остальных плагинов.

## Приёмка

Автономные тесты Worker: meaningful changes, устранение повторов, pause.
Native тесты: reconnect/latest state, rate limit, failure/clear.
Ручные проверки: клиент отсутствует/запускается позже, pause/resume/seek,
reload UI, disable/enable и app exit при открытом Discord.
