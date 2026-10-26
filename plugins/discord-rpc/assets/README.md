# Discord logo

The Discord mark comes from the [Simple Icons Discord asset](https://github.com/simple-icons/simple-icons/blob/develop/icons/discord.svg)
([Simple Icons license](https://github.com/simple-icons/simple-icons/blob/develop/LICENSE.md)).
The shape is preserved, with Discord blurple `#5865F2` and transparent padding.
SVG path spacing is expanded for ImageMagick compatibility.

`discord.png` is embedded as a data URL in `../plugin.json`, so the card works offline.
To regenerate the raster from the vector source:

```sh
magick -background none discord.svg discord.png
```
