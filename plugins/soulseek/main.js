registerPlugin((ctx) => {
  let config = { revision: 0, values: {} };
  let hasPassword = false;
  let error = '';
  let notice = '';
  let currentPlayback = null;
  let query = '';
  let replacementResults = [];
  let replacementPage = 0;
  const searches = new Map();
  const results = new Map();
  const transfers = new Map();
  const pendingPlay = new Set();
  const pendingSave = new Map();
  const busy = new Set();
  const tokens = new Map();
  const key = (r) => JSON.stringify([r.username, r.filename]);
  const request = (r) => ({ username: r.username, filename: r.filename, size: r.size,
    coverFilename: r.coverFilename || null, coverSize: r.coverSize || null });
  const identity = (r) => ({ pluginId: ctx.pluginId, resourceId: key(r), title: r.basename,
    artist: r.username, album: '', durationSeconds: r.duration || null, coverHash: r.coverHash || null });
  let writes = Promise.resolve();
  function persistIntents() {
    writes = writes.catch(() => {}).then(async () => {
      const ids = new Set([...pendingPlay, ...pendingSave.keys()]);
      const intents = [...ids].map(id => ({ result: results.get(id), play: pendingPlay.has(id), save: pendingSave.has(id), trackId: pendingSave.get(id) || null, generation: ctx.generation })).filter(i => i.result);
      const latest = await ctx.call('settings.get');
      config = await ctx.call('settings.update', { revision: latest.revision, values: { ...latest.values, transferIntents: intents } });
    });
    return writes;
  }
  function row(r) {
    const transfer = transfers.get(key(r));
    const format = [r.extension, r.bitrate && `${r.bitrate} kbps`, r.sampleRate && `${r.sampleRate} Hz`, r.bitDepth && `${r.bitDepth}-bit`].filter(Boolean).join(' · ');
    return { id: key(r), title: r.basename, subtitle: `${r.username} · ${Math.round(r.size / 1024)} KiB · ${format}${r.duration ? ` · ${Math.round(r.duration)} s` : ''}${r.peerSpeed ? ` · ${Math.round(r.peerSpeed / 1024)} KiB/s` : ''}${r.freeUploadSlots ? ` · ${r.freeUploadSlots} slots` : ''}`,
      coverHash: r.coverHash || null, status: transfer ? `${transfer.state}${transfer.bytesDownloaded != null ? ` · ${Math.round(transfer.bytesDownloaded / 1024)} KiB` : ''}${transfer.queuePosition != null ? ` · queue ${transfer.queuePosition}` : ''}` : '',
      actions: [{ id: 'play', label: pendingPlay.has(key(r)) ? 'Cancel preview' : currentPlayback?.playing && currentPlayback.resource?.resourceId === key(r) ? 'Stop preview' : 'Preview' },
        { id: 'save', label: busy.has(key(r)) ? 'Saving…' : 'Save to library', disabled: busy.has(key(r)) }] };
  }
  async function search(queryText) {
    const status = await ctx.call('soulseek.status');
    if (!status.configured) throw new Error('Add your username and password in Soulseek settings first.');
    error = ''; const found = await ctx.call('soulseek.search', { query: queryText });
    for (const r of found) results.set(key(r), r);
    return found;
  }
  const coverCache = new Map();
  const coverQueued = new Set();
  const coverQueue = [];
  let coverWorkers = 0;
  function scheduleCovers(rows) {
    for (const r of rows) {
      if (!r.coverFilename || !r.coverSize) continue;
      const id = JSON.stringify([r.username, r.coverFilename]);
      if (coverCache.has(id)) { r.coverHash = coverCache.get(id); continue; }
      if (!coverQueued.has(id)) { coverQueued.add(id); coverQueue.push(r); }
    }
    while (coverWorkers < 2 && coverQueue.length) {
      coverWorkers++;
      void (async () => {
        try {
          while (coverQueue.length && !ctx.signal.aborted) {
            const r = coverQueue.shift(); const id = JSON.stringify([r.username, r.coverFilename]);
            let hash = null;
            try { hash = await ctx.call('soulseek.cover', { request: { username: r.username, coverFilename: r.coverFilename, coverSize: r.coverSize } }); } catch { /* A missing cover does not invalidate music. */ }
            coverCache.set(id, hash); coverQueued.delete(id);
            for (const item of results.values()) if (item.username === r.username && item.coverFilename === r.coverFilename) item.coverHash = hash;
            ctx.ui.invalidate();
          }
        } finally { coverWorkers--; }
      })();
    }
  }
  async function prepare(r) {
    const id = key(r);
    if (!tokens.has(id)) tokens.set(id, await ctx.call('soulseek.prepare', { username: r.username, filename: r.filename, resource: identity(r) }));
    return tokens.get(id);
  }
  function ready(r, t) {
    if (!t || !t.localPath) return false;
    if (t.state === 'completed') return true;
    const threshold = r.bitrate ? Math.max(384 * 1024, r.bitrate * 1000 / 8 * 18) : 1500000;
    return t.bytesDownloaded >= Math.min(r.size, threshold);
  }
  async function play(r) {
    await ctx.playback.play({ sourceToken: await prepare(r), resource: identity(r) });
  }
  async function save(r, trackId) {
    const id = key(r); if (busy.has(id)) return;
    busy.add(id); ctx.ui.invalidate();
    try {
      const sourceToken = await prepare(r);
      const track = await ctx.call(trackId ? 'library.replace' : 'library.import', { sourceToken, trackId, coverHash: r.coverHash || null });
      if (!track) throw new Error('The downloaded file could not be indexed');
      await ctx.playback.adopt({ resourceId: id, track });
      notice = trackId ? 'Track replaced.' : 'Saved to your library.';
    } finally { busy.delete(id); pendingSave.delete(id); await persistIntents(); ctx.ui.invalidate(); }
  }
  async function onTransfer(t) {
    const id = key(t); transfers.set(id, t); const r = results.get(id);
    ctx.ui.invalidate();
    if (!r) return;
    if (['failed', 'timed_out', 'cancelled'].includes(t.state)) { pendingPlay.delete(id); pendingSave.delete(id); await persistIntents(); error = t.error || 'Transfer failed'; return; }
    if (pendingPlay.has(id) && ready(r, t)) { pendingPlay.delete(id); await play(r); await persistIntents(); }
    if (t.state === 'completed' && pendingSave.has(id)) await save(r, pendingSave.get(id));
  }
  async function start(r) {
    const id = key(r); const t = transfers.get(id);
    if (t && !['failed', 'timed_out', 'cancelled'].includes(t.state)) return;
    tokens.delete(id);
    transfers.set(id, { state: 'starting', bytesDownloaded: 0 });
    try { await ctx.call('soulseek.preview', { request: request(r) }); }
    catch (error) {
      transfers.set(id, { state: 'failed', error: String(error.message || error) });
      pendingPlay.delete(id); pendingSave.delete(id); await persistIntents().catch(() => {}); throw error;
    }
  }
  return {
    async activate() {
      config = await ctx.call('settings.get');
      for (const intent of config.values.transferIntents || []) {
        if (intent.generation !== ctx.generation || !intent.result) continue;
        const id = key(intent.result); results.set(id, intent.result);
        if (intent.play) pendingPlay.add(id);
        if (intent.save) pendingSave.set(id, intent.trackId || null);
      }
      ctx.playback.subscribe(snapshot => {
        currentPlayback = snapshot; ctx.ui.invalidate();
        const resource = snapshot.resource;
        if (resource?.pluginId === ctx.pluginId && pendingPlay.delete(resource.resourceId)) void persistIntents().catch(e => { error = e.message; ctx.ui.invalidate(); });
      });
      try { hasPassword = await ctx.call('secrets.has', { key: 'password' }); } catch (e) { error = e.message; }
      ctx.on('soulseek-preview', (event) => onTransfer(event).catch((e) => { error = e.message; ctx.ui.invalidate(); }));
      ctx.on('soulseek-download', (event) => onTransfer(event).catch((e) => { error = e.message; ctx.ui.invalidate(); }));
    },
    renderSearch({ ids }) { return ids.map(id => results.get(id)).filter(Boolean).map(row); },
    async search({ query: text, cursor = null, limit = 120 }) {
      if (!cursor) { searches.set(text, await search(text)); if (searches.size > 4) searches.delete(searches.keys().next().value); }
      const found = searches.get(text); if (!found) throw new Error('Search expired. Run the search again.');
      const offset = cursor ? Number(cursor) : 0;
      if (!Number.isSafeInteger(offset) || offset < 0) throw new Error('Invalid search cursor');
      const count = Math.max(1, Math.min(120, Number(limit) || 120));
      const page = found.slice(offset, offset + count); scheduleCovers(page);
      return { items: page.map(row), nextCursor: offset + count < found.length ? String(offset + count) : null };
    },
    render({ viewId, context }) {
      const messages = [...(error ? [{ type: 'text', text: error, tone: 'error' }] : []), ...(notice ? [{ type: 'text', text: notice }] : [])];
      if (viewId === 'connection') return { type: 'group', title: 'Soulseek connection', children: [
        { type: 'text', text: 'Use your Soulseek account. Downloaded music is imported into your library using artist and album tags.' },
        { type: 'input', key: 'username', label: 'Username', value: config.values.username || '' },
        { type: 'input', key: 'password', label: hasPassword ? 'New password (leave blank to keep saved)' : 'Password', inputType: 'password', value: '' },
        { type: 'button', label: 'Save connection', action: 'save-settings' }, ...messages,
      ] };
      if (viewId === 'buffer') {
        const active = [...transfers.values()].filter((t) => ['starting', 'queued_local', 'queued_remote', 'progress'].includes(t.state));
        return { type: 'text', text: active.length ? `Soulseek · ${active.length} active transfer${active.length === 1 ? '' : 's'}` : '' };
      }
      scheduleCovers(replacementResults.slice(replacementPage * 120, (replacementPage + 1) * 120));
      return { type: 'group', title: context?.track ? `Replace ${context.track.title || context.track.path}` : 'Soulseek', children: [
        { type: 'input', key: 'query', label: 'Search query', value: query || [context?.track?.artist, context?.track?.title].filter(Boolean).join(' ') },
        { type: 'button', label: 'Search', action: 'replacement-search' }, ...messages,
        { type: 'results', items: replacementResults.slice(replacementPage * 120, (replacementPage + 1) * 120).map((r) => ({ ...row(r), actions: [...row(r).actions, ...(context?.track ? [{ id: 'replace', label: 'Replace track' }] : [])] })) },
        ...(replacementPage > 0 ? [{ type: 'button', label: 'Previous results', action: 'replacement-previous' }] : []),
        ...((replacementPage + 1) * 120 < replacementResults.length ? [{ type: 'button', label: 'Next results', action: 'replacement-next' }] : []),
      ] };
    },
    async action({ action, payload = {}, fields = {}, context }) {
      error = ''; notice = '';
      try {
        if (action === 'save-settings') {
          if (fields.password) await ctx.call('secrets.set', { key: 'password', value: fields.password });
          await writes.catch(() => {});
          const latest = await ctx.call('settings.get');
          config = await ctx.call('settings.update', { revision: latest.revision, values: { ...latest.values, username: String(fields.username || '').trim() } });
          hasPassword = await ctx.call('secrets.has', { key: 'password' }); notice = 'Connection saved.'; return;
        }
        if (action === 'find-replacement') { query = ''; replacementResults = []; await ctx.ui.navigate('replacement', { track: payload.track }); return; }
        if (action === 'replacement-search') { query = String(fields.query || '').trim(); replacementPage = 0; if (query) replacementResults = await search(query); return; }
        if (action === 'replacement-next') { replacementPage = Math.min(Math.ceil(replacementResults.length / 120) - 1, replacementPage + 1); return; }
        if (action === 'replacement-previous') { replacementPage = Math.max(0, replacementPage - 1); return; }
        const r = results.get(payload.resourceId); if (!r) throw new Error('Search again to restore this result');
        const id = key(r);
        if (action === 'play') {
          if (currentPlayback?.playing && currentPlayback.resource?.pluginId === ctx.pluginId && currentPlayback.resource.resourceId === id) { await ctx.playback.stop(); return; }
          if (pendingPlay.has(id)) { pendingPlay.delete(id); transfers.set(id, { state: 'cancelled' }); tokens.delete(id); await persistIntents(); await ctx.call('soulseek.cancelPreview', { request: { username: r.username, filename: r.filename } }); return; }
          const t = transfers.get(id); if (ready(r, t)) { await play(r); return; }
          pendingPlay.clear(); pendingPlay.add(id); try { await persistIntents(); await start(r); } catch (e) { pendingPlay.delete(id); await persistIntents().catch(() => {}); throw e; }
        } else if (action === 'save' || action === 'replace') {
          const trackId = action === 'replace' ? context?.track?.id : null;
          if (action === 'replace' && !trackId) throw new Error('Choose the track to replace first');
          if (transfers.get(id)?.state === 'completed') await save(r, trackId);
          else { pendingSave.set(id, trackId); try { await persistIntents(); await start(r); } catch (e) { pendingSave.delete(id); await persistIntents().catch(() => {}); throw e; } }
        }
      } catch (e) { error = String(e.message || e); throw e; }
      finally { ctx.ui.invalidate(); }
    },
  };
});
