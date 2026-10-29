import type { CatalogEntry, PluginManifest } from './types';

export function filterCatalog(entries: CatalogEntry[], query: string): CatalogEntry[] {
  const terms = query.toLocaleLowerCase().trim().split(/\s+/).filter(Boolean);
  return entries.filter(entry => {
    const author = typeof entry.author === 'string' ? entry.author : entry.author?.name || '';
    const text = [entry.id, entry.name, entry.description, author, ...entry.platforms, ...entry.tags].join(' ').toLocaleLowerCase();
    return terms.every(term => text.includes(term));
  }).sort((a, b) => b.addedAt.localeCompare(a.addedAt) || a.name.localeCompare(b.name));
}

/** Catalog metadata is only for display; install preview always uses the downloaded manifest. */
export function catalogManifest(entry: CatalogEntry): PluginManifest {
  return { id: entry.id, name: entry.name, version: entry.version || '', apiVersion: 1,
    description: entry.description, author: entry.author, image: entry.image,
    repository: entry.repository, platforms: entry.platforms, permissions: [], contributions: [] };
}
