For sprites optimization pls execute:

```bash
for file in zippi_sit_chill000?.webp; do
    magick "$file" -resize 800 -quality 95 "${file%.webp}_optimized.webp"
done
```

todo: make automation lol