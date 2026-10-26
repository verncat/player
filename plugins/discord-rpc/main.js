registerPlugin((ctx) => {
  let error = '';
  let last = null;
  let lastAt = 0;
  let sending = false;
  let pending;
  async function flush() {
    if (sending) return;
    sending = true;
    try {
      while (pending !== undefined) {
        const playback = pending;
        pending = undefined;
        try { await ctx.call('discord.update', { playback }); error = ''; }
        catch (e) { error = String(e.message || e); last = null; }
      }
    } finally { sending = false; ctx.ui.invalidate(); }
  }
  function publish(snapshot) {
    const current = snapshot.playing ? snapshot : null;
    const now = Date.now();
    if (!last && !current && lastAt) return;
    if (last && current && last.title === current.title && last.artist === current.artist
      && last.album === current.album && last.duration === current.duration
      && Math.abs(last.position + (now - lastAt) / 1000 - current.position) < 2) return;
    last = current && { ...current }; lastAt = now; pending = current; void flush();
  }
  return {
    activate() { ctx.playback.subscribe(publish); },
    render() { return { type: 'group', title: 'Discord Rich Presence', children: [
      { type: 'text', text: 'Your current track and artist appear in Discord. Activity is hidden while paused. Requires the Discord desktop app.' },
      ...(error ? [{ type: 'text', text: error, tone: 'error' }] : []),
    ] }; },
  };
});
