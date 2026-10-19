import { computed, onMounted, onUnmounted, ref, watch, type WatchSource } from "vue";

// Back/forward navigation kept in the browser history, so the topbar arrows,
// the mouse back button and Android's back button all walk the same stack.
// (Wry's Android activity goes back inside the webview while it can, and
// only leaves the app once there is nothing to go back to.)
//
// Besides pages, the history can hold one transient entry: UI state on top
// of the current page that back should undo, and forward should never redo.

interface HistoryEntry {
  /** Position in this app's history, which the History API doesn't expose. */
  index: number;
  page: unknown;
  transientId?: number;
}

interface TransientEntry {
  id: number;
  index: number;
  onLeave: () => void;
}

function isHistoryEntry(state: unknown): state is HistoryEntry {
  return typeof state === 'object' && state !== null && typeof (state as HistoryEntry).index === 'number';
}

// Module-level: there is one browser history per window.
const currentIndex = ref(0);
const lastIndex = ref(0);
let currentPage: unknown = null;
let showPage: ((page: unknown) => void) | null = null;
let transient: TransientEntry | null = null;
let nextTransientId = 1;

/** Stops tracking the transient entry. It stays in the browser history, past where forward can reach. */
function forgetTransient(): TransientEntry | null {
  const forgotten = transient;
  if (forgotten) {
    transient = null;
    lastIndex.value = forgotten.index - 1;
  }
  return forgotten;
}

/** Adds an entry. A transient entry gets replaced, so back never returns to it. */
function addEntry(page: unknown, transientId?: number) {
  const replaced = forgetTransient();
  replaced?.onLeave();
  if (!replaced) currentIndex.value += 1;
  lastIndex.value = currentIndex.value;
  currentPage = page;
  const state: HistoryEntry = { index: currentIndex.value, page, transientId };
  if (replaced) history.replaceState(state, '');
  else history.pushState(state, '');
}

function onPopState(event: PopStateEvent) {
  if (!isHistoryEntry(event.state)) return;
  const entry = event.state;
  if (transient && entry.transientId !== transient.id) forgetTransient()?.onLeave();
  if (entry.transientId !== undefined && entry.transientId !== transient?.id) {
    // A forgotten transient entry, reached with the mouse's forward button.
    history.back();
    return;
  }
  currentIndex.value = entry.index;
  currentPage = entry.page;
  showPage?.(entry.page);
}

/**
 * Page navigation for the app shell. `show` displays a page reached with
 * back/forward; pages must be plain data (they're stored with pushState).
 */
export function useHistoryNavigation<P>(initialPage: P, show: (page: P) => void) {
  currentPage = initialPage;
  currentIndex.value = 0;
  lastIndex.value = 0;
  history.replaceState({ index: 0, page: initialPage } satisfies HistoryEntry, '');
  showPage = show as (page: unknown) => void;

  onMounted(() => window.addEventListener('popstate', onPopState));
  onUnmounted(() => {
    window.removeEventListener('popstate', onPopState);
    showPage = null;
  });

  return {
    current: () => currentPage as P,
    push: (page: P) => addEntry(page),
    /** Changes the current entry's page, e.g. after the page's subject was renamed. */
    replace: (page: P) => {
      currentPage = page;
      history.replaceState({ ...(history.state as HistoryEntry), page }, '');
    },
    back: () => history.back(),
    forward: () => history.forward(),
    canBack: computed(() => currentIndex.value > 0),
    canForward: computed(() => currentIndex.value < lastIndex.value),
  };
}

/**
 * Keeps a transient history entry while `isActive` is true, for state that
 * back should undo (an open dialog, a selection mode). Back, navigating to a
 * page, or another transient entry ends it and calls `onLeave`; becoming
 * inactive on its own removes the entry.
 */
export function useTransientHistoryEntry(isActive: WatchSource<boolean>, onLeave: () => void) {
  const id = nextTransientId++;

  function remove() {
    if (transient?.id !== id) return;
    forgetTransient();
    history.back();
  }

  watch(isActive, (active) => {
    if (active && transient?.id !== id) {
      addEntry(currentPage, id);
      transient = { id, index: currentIndex.value, onLeave };
    } else if (!active) {
      remove();
    }
  });
  onUnmounted(remove);
}
