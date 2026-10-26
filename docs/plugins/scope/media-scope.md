# Media scope: источники, передачи и библиотека

Зависимости: [runtime](runtime-scope.md), [security](storage-security-scope.md).
Источники: `plugin_sources.rs`, `playback.rs`, gateway и адаптер Soulseek.

## Идентичность

```ts
type ResourceIdentity = {
  pluginId: string; resourceId: string;
  title: string; artist: string; album: string;
  durationSeconds: number | null; coverHash: string | null;
};
```

ResourceIdentity обозначает внешний ресурс. Native `source_id` обозначает
конкретный запуск аудио; это разные ключи. `sourceToken` — выданный хостом
handle проверенного файла, связанный с plugin ID и generation. Путь файла
не является публичным аргументом play/import API.

Library Track сохраняет обычный положительный числовой ID. Временные строки
плеера имеют `plugin_source` и уникальный отрицательный ID, выделенный хостом
без хеш-коллизий; он нужен только совместимому UI очереди и не сохраняется в БД.
Устойчивый ключ — ResourceIdentity, не этот временный номер.

## Получение источника

Встроенный Soulseek вызывает `soulseek.prepare` для уже начатого preview.
Обычный JS-плагин может вызвать `http.download`:

```js
const {sourceToken} = await ctx.call('http.download', {
  url: 'https://example.org/audio.mp3', filename: 'audio.mp3',
  resource: {pluginId: ctx.pluginId, resourceId: 'audio-1',
    title: 'Audio', artist: '', album: '', durationSeconds: null, coverHash: null}
});
await ctx.playback.play({sourceToken, resource});
```

HTTP download разрешён с network.http и правом на использование источника
(playback.control или library.import/replace). Схемы URL — HTTP(S), размер до
64 MiB, сетевой таймаут 30 секунд. Неполный файл удаляется при ошибке.
`filename` — только безопасное имя, не путь. Source token действует в текущем
native-сеансе; при отключении отзывается.

## GrowingFile

Native типы: File, CueSegment и GrowingFile(resource, cache_path).
Общий реестр состояния файла содержит Active/Completed/Failed/TimedOut/Cancelled.
EOF при Active ждёт следующую порцию, после Completed завершает трек;
ошибка/отмена/stop завершают ожидание. Неожиданное закрытие download channel
переводит preview в Failed. Декодер не вызывает код Soulseek.

Soulseek дописывает непрерывный префикс файла. Готовность preview оценивается
буфером и возможностью декодера открыть формат; некоторые форматы могут
потребовать завершения файла. Отдельные попытки получают разные staging-пути,
чтобы поздний writer не повредил новое воспроизведение.

## Импорт и замена

- `library.import({sourceToken,coverHash?})` принимает завершённую передачу,
  копирует файл, размещает по тегам Artist/Album, индексирует, возвращает Track.
- Одинаковый уже импортированный файл не создаёт ещё один трек; конфликт имени
  с другим содержимым получает суффикс по hash.
- `library.replace({sourceToken,trackId,coverHash?})` заменяет файл штатным
  сервисом, сохраняя ID и пользовательские данные трека. CUE-сегмент нельзя
  заменять в месте общего album image: такой запрос отклоняется, файл можно
  импортировать отдельно.
- Операции не принимают произвольный абсолютный путь от Worker.
- Native engine и UI после импорта принимают библиотечный ID без повторного
  запуска звука. Worker вызывает playback.adopt для обновления очереди/UI.
- Decoder продолжает читать старый staging-файл до закрытия: импорт не
  перемещает открытый файл, что необходимо в том числе на Windows.
- До фиксации изменения файла/SQLite исходные файлы резервируются; при
  ошибке операции они восстанавливаются. Это не обещание полной атомарности
  файловой системы и SQLite при внезапном завершении процесса.
- Cache нельзя очищать, пока он читается декодером, даже если ресурс уже
  отображается как библиотечный трек.

## Очередь и восстановление

Plugin source проходит через общий координатор Player: выбранный remote
output сначала приостанавливается; временный источник воспроизводится локально.
Передать внешний ресурс на peer можно после индексации и получения file hash.
Статистика, лайки и библиотечные плейлисты работают для импортированного Track.

Reload читает ResourceIdentity из native session и создаёт UI-строку без play.
Для повторного воспроизведения активного ресурса `plugin_current_source`
выдаёт новый token того же проверенного файла. Отсутствующий/отключённый плагин
не превращает ResourceIdentity в непроверенный filesystem path.

Приёмка: EOF во время загрузки и после завершения, channel failure, cancel,
disable, повтор preview, reload, импорт проигрываемого файла, конфликт имён,
замена с сохранением ID/лайков/плейлистов и cache cleanup с активным decoder.
