# Runtime scope: lifecycle и сообщения

Зависимости: [package](package-scope.md), [SDK](sdk-ui-scope.md).
Основные файлы: `host.ts`, `worker.js`, `store.rs`, native gateway `mod.rs`.
Экспериментальные APK-расширения дополняют этот lifecycle отдельным
[Android native контрактом](android-native-scope.md): APK-классы и `.so`
загружаются в процесс Player, а UI остаётся в Worker.

## Жизненный цикл

Устойчивое состояние: `enabled` и `generation` в native registry. Оперативное
состояние UI: starting, running или ошибка. Поддержка платформы определяется
независимо от enabled. Включённый пакет не считается работающим до ответа activate.

При включении native-хост проверяет платформу, сохраняет enabled и новое
поколение. Broker загружает script, создаёт Worker, внедряет SDK и вызывает
`activate`. Только успешный Worker получает видимые UI-вклады. Ошибка активации
отзывает native-права, завершает Worker и показывается рядом с плагином.

При отключении native-хост сначала сохраняет revocation и новое generation,
отзывает source handles, останавливает активный источник этого плагина,
отменяет его native-передачи/активность. Затем broker вызывает deactivate
(до 2 секунд), отвергает оставшиеся RPC и принудительно завершает Worker.
Операции включения/отключения одного плагина сериализуются.

Повторное включение создаёт новый Worker и контекст. Старые ответы не должны
изменять UI нового поколения. Native-права проверяются до операции и перед
возвращением результата; передачи также проверяют собственное поколение.

## Reload и выход

Reload WebView завершает только frontend Workers. Enabled/generation,
запущенный звук и native-передачи сохраняются. Broker подписывается на события,
затем получает package и native event snapshots, активирует новый Worker и
доставляет накопленные события в порядке revision, устраняя дубликаты.

Аудио восстанавливается через `playback_session`; это чтение состояния,
а не повторный play/seek. JavaScript-состояние, которое требуется после reload,
плагин сохраняет в settings. После полного рестарта прерванные сетевые передачи
автоматически не возобновляются. Выход приложения завершает native-адаптеры.

## Протокол Worker ↔ broker

Сообщения — structured-clone JSON DTO; функции, компоненты и handles ОС
не передаются. Идентичность Worker broker определяет по своей записи runtime.

| Направление | Message |
| --- | --- |
| Broker → Worker | `{type:'request', id, method, args}` |
| Worker → broker | `{type:'response', id, result}` или `{type:'response', id, error}` |
| Worker → broker | `{type:'call', id, method, args}` |
| Broker → Worker | `{type:'reply', id, result}` или `{type:'reply', id, error}` |
| Broker → Worker | `{type:'event', topic, payload}` |
| Broker → Worker | `{type:'playback', snapshot}` |
| Worker → broker | `{type:'invalidate'}` |

Lifecycle методы: activate/deactivate. Рабочие методы: render, action,
search, renderSearch. Неизвестный метод завершается ошибкой. Обычный RPC имеет
таймаут 45 секунд, activate — 15 секунд. Таймаут frontend-ожидания не равен
отмене уже созданной native-передачи; для неё предусмотрен cancelPreview.

Native события имеют `{pluginId,generation,revision,topic,payload}`.
Хост хранит до 512 последних состояний, ключ — plugin/topic/transfer ID.
Это восстановление текущих состояний, не долговременный журнал всех событий.

## Ошибки и ограничения

- Ошибка action/search отображается в соответствующем разделе.
- Ошибка Worker завершает его и показывается в менеджере; Retry пересоздаёт его.
- Worker не может зависанием остановить основной JS-поток; broker может
  завершить его без участия plugin lifecycle.
- Native-доступ и сетевые ограничения описаны отдельно в security scope;
  Worker не объявляется полной sandbox.
- UI-вклад не владеет native-операцией: размонтирование панели не отменяет
  загрузку или воспроизведение.

Приёмка: repeated enable/disable, activate throw/timeout, pending RPC при
отключении, поздний ответ, reload с растущим файлом, failure одного Worker
при работающем втором плагине и штатный app shutdown.
