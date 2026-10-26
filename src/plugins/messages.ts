/** The plugin wire format is JSON. Vue proxies must not cross the Worker boundary. */
export function postPluginMessage(worker: Pick<Worker, 'postMessage'>, message: Record<string, unknown>): void {
  worker.postMessage(JSON.parse(JSON.stringify(message)));
}
