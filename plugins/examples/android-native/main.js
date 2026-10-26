registerPlugin(ctx => {
  let result;
  async function ping(message) {
    result = await ctx.android.call('echo', { message });
    ctx.ui.invalidate();
  }
  return {
    async activate() { await ping('Loaded from an APK'); },
    render() {
      return { type: 'group', title: 'Android native plugin', children: [
        { type: 'text', text: result ? `${result.message} · ${result.backend} · ${result.arch}` : 'Not connected' },
        { type: 'text', text: `Rust calls: ${result?.callCount || 0}` },
        { type: 'input', key: 'message', label: 'Message', value: 'Hello from Player' },
        { type: 'button', action: 'ping', label: 'Call Rust' },
      ] };
    },
    async action({ action, fields }) {
      if (action === 'ping') await ping(String(fields.message || 'Hello from Player'));
    },
  };
});
