# Local Opus timing patch

Source: `symphonia-format-ogg` 0.5.5, upstream commit `6d533f26150953a882a6a111ebd13f0abf7129d5`.
Upstream: https://github.com/pdeljanov/Symphonia/tree/v0.5.5/symphonia-format-ogg
License: MPL-2.0, preserved in LICENSE and source headers.

Only Opus timing is changed:

- Normalize granule positions by the explicit OpusHead pre-skip.
- Use that explicit pre-skip when inspecting the first audio page.
- Derive end padding for a stream whose only audio page is also its last page.
- Establish bounds before assigning timestamps to the first buffered audio packets, also for growing streams that cannot yet be probed to their end.
- Read the full six-bit frame count in code-3 packets so 48 x 2.5 ms packets retain their correct 120 ms duration.

This prevents encoder delay and final padding from becoming audible audio and keeps duration and seeking consistent. Regression coverage is in the app's `audio_codecs` tests, including short single-page mono, stereo 120 ms packets, longer multi-page stereo, and surround Opus fixtures. Remove this Cargo patch when upgrading to an upstream version with equivalent handling.
