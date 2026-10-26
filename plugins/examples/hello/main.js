registerPlugin((ctx) => {
  let settings = { revision: 0, values: {} };
  let title = 'Nothing playing';
  return {
    async activate() {
      settings = await ctx.call('settings.get');
      ctx.playback.subscribe((snapshot) => { title = snapshot.playing ? snapshot.title : 'Nothing playing'; ctx.ui.invalidate(); });
    },
    render({ viewId }) {
      if (viewId === 'status') return { type: 'text', text: title };
      return { type: 'group', title: settings.values.greeting || 'Hello Player', children: [
        { type: 'text', text: `Now playing: ${title}` },
        { type: 'input', key: 'greeting', label: 'Greeting', value: settings.values.greeting || '' },
        { type: 'button', label: 'Save', action: 'save' },
      ] };
    },
    async action({ action, fields }) {
      if (action === 'save') settings = await ctx.call('settings.update', { revision: settings.revision, values: { greeting: String(fields.greeting || '').slice(0, 100) } });
      ctx.ui.invalidate();
    },
  };
});
