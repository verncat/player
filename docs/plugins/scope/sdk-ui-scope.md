# SDK / UI scope

Зависимости: [runtime](runtime-scope.md), [media](media-scope.md).
DTO: `src/plugins/types.ts`. Исполняемый SDK: `src/plugins/worker.js`.

## Entrypoint и контекст

Script один раз вызывает `registerPlugin(factory)`. Синхронная factory получает
`ctx` и возвращает объект с необязательными lifecycle/операционными методами.

```js
registerPlugin(ctx => ({
  async activate() {},
  async deactivate() {},
  render({viewId, context}) { return {type: 'text', text: 'Ready'}; },
  async action({action, payload, fields, context}) {},
  async search({providerId, query, cursor, limit}) { return {items: [], nextCursor: null}; },
  renderSearch({ids}) { return null; }
}));
```

| Контекст | Контракт |
| --- | --- |
| `ctx.pluginId` | ID текущего пакета |
| `ctx.generation` | Поколение native-активации для устойчивых намерений |
| `ctx.signal` | AbortSignal lifetime Worker; отменяется при deactivate |
| `ctx.call(method,args)` | Promise вызова API, права проверяет хост |
| `ctx.on(topic,handler)` | Собственные native-события; возвращает unsubscribe |
| `ctx.playback.subscribe(handler)` | Последний snapshot сразу, затем изменения; unsubscribe |
| `ctx.playback.play({sourceToken,resource})` | Воспроизведение через основной координатор |
| `ctx.playback.pause/resume/stop/seek` | Управление текущим transport; seek принимает секунды |
| `ctx.playback.adopt({resourceId,track})` | Обновить UI/очередь после импорта своего ресурса |
| `ctx.ui.navigate(pageId,context)` | Открыть собственную зарегистрированную страницу |
| `ctx.ui.invalidate()` | Запросить обновление представлений/результатов |
| `ctx.log(message)` | Локальное диагностическое сообщение; не включать секреты |

Playback snapshot: `{title,artist,album,position,duration,playing,resource?}`;
времена в секундах. Transport/позиция читаются из native engine. Данные поступают
из общей оболочки, включая воспроизведение на выбранном remote output.
Плагин не опрашивает `playback_status` самостоятельно.

## Contributions

Manifest декларативно регистрирует вклады. Общие поля: `id`, `kind`, `label`,
`description?`, `icon?`, `order?` (0 по умолчанию), `slot?`.
Полный ключ `<pluginId>/<id>` создаёт хост. ID — до 64 символов `[a-z0-9-]`.
Порядок: order, затем полный ключ. Основные элементы плеера не заменяются.

| kind | Встраивание и обработчик |
| --- | --- |
| `page` | Навигация, маршрут `plugins/<pluginId>/<id>`, render(viewId=id) |
| `settings` | Раздел настроек `settings/plugin:<pluginId>/<id>`, render |
| `search` | Отдельный блок общего поиска, явная кнопка Search, search |
| `track-action` | Меню библиотечного трека, action=id, payload.track |
| `player-widget` | render в `player.actions` или `player.status` |

Переданный context — данные маршрута/действия; не ссылка на refs оболочки.
UI должен работать с клавиатурой, мобильной шириной и общей темой Player.
Для подключения вклада разработчик не меняет `App.vue` или `SETTINGS_SECTIONS`.

## Дерево UI

Схема `UiNode` ограничена известными узлами, глубина до 12, детей до 100:

| type | Поля |
| --- | --- |
| `group` | title?, children? |
| `text` | text, tone? (`error` для ошибки) |
| `input` | key, label, value?, inputType? (`text` / `password`) |
| `toggle` | label, value:boolean, action, payload?, disabled? |
| `button` | label, action, payload?, disabled? |
| `results` | items: SearchItem[] (до 1000) |

`input` хранит draft в панели. Кнопка передаёт общий `fields` в action.
Ошибка сохраняет draft, успех перечитывает дерево. Пароль не подставляется
из сохранённого секрета. Текст выводится интерполяцией, без v-html/eval.

SearchItem: `{id,title,subtitle?,coverHash?,status?,actions?}`;
actions: `{id,label,disabled?}`. Нажатие вызывает plugin action с
`payload:{resourceId:item.id}`. Item ID уникален в пространстве плагина и
представляет внешний ресурс; он не является библиотечным track ID.

`search` возвращает `{items,nextCursor}`. Cursor — непрозрачная строка следующей
страницы или null. Хост запрашивает до 120 строк, отображает Previous/Next и
сохраняет историю курсоров текущего запроса. Первая страница получает null;
смена query сбрасывает курсоры и отбрасывает поздние ответы прошлого запроса.
`renderSearch({ids})` обновляет уже найденные
строки после transfer/cover событий без нового сетевого поиска; null сохраняет
предыдущие строки. Изменение query отбрасывает поздний результат прошлого поиска.

Приёмка: все типы вкладов через пример/fixture; некорректное дерево; ошибка
рендера; быстрые смены query; drafts/ошибки формы; disabled-действия; мобильные
строки результатов; удаление вкладов после остановки Worker.
