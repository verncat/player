# Synthetic codec fixtures

These files contain generated sine waves, not recorded music. They are checked in so codec tests need neither FFmpeg nor an audio device. They were generated with FFmpeg, libopus, libmp3lame, and FFmpeg's native experimental Vorbis encoder (`-c:a vorbis -strict experimental -ac 2`).

| File | Signal | Purpose |
| --- | --- | --- |
| opus-stereo.opus | 440 Hz left, 880 Hz right, 3.25 s, 44.1 kHz input | Multi-page Opus, 48 kHz decode, pre-skip, seeking |
| opus-mono.opus | 440 Hz, 0.48 s, 24 kHz input | Single-page mono, end padding |
| opus-120ms.opus | 440 Hz left, 880 Hz right, 0.5 s | Maximum packet duration, single-page end padding |
| opus-surround.opus | 220/330/440/55/660/770 Hz in FFmpeg 5.1 order, 1 s | Vorbis-to-Symphonia channel order |
| reference.flac | 440 Hz left, 880 Hz right, 0.25 s | Built-in FLAC decoder regression |
| reference.mp3 | 440 Hz, 0.5 s | Built-in MP3 decoder regression |
| reference.ogg | 440 Hz, 0.5 s | Upstream Vorbis demux/decode regression |

For example, regenerate the stereo fixture with:

```sh
ffmpeg -f lavfi -i 'aevalsrc=0.1*sin(2*PI*440*t)|0.1*sin(2*PI*880*t):s=44100:d=3.25' \
  -c:a libopus -b:a 96k -frame_duration 20 -y opus-stereo.opus
```

The other Opus fixtures use `-c:a libopus`; the long-packet fixture uses `-frame_duration 120`, mono uses `-b:a 48k`, and surround uses `-channel_layout 5.1 -b:a 256k`.
