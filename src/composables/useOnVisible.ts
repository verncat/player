import { inject, onMounted, onUnmounted, provide, type InjectionKey, type Ref } from "vue";

// The element that scrolls the page content (App.vue's `.content`). Rows inside
// it are clipped by it, so it has to be the observer root for rootMargin to
// take effect; with the default root (the viewport) a row only counts as
// visible once it's actually on screen.
const scrollRootKey: InjectionKey<Ref<HTMLElement | null>> = Symbol("ScrollRoot");

export function provideScrollRoot(root: Ref<HTMLElement | null>) {
  provide(scrollRootKey, root);
}

// Calls `callback` once, the first time `el` comes within 240px of the visible
// part of `root` (null = the viewport). Returns a function that stops watching.
export function observeOnce(el: HTMLElement, callback: () => void, root: HTMLElement | null): () => void {
  const observer = new IntersectionObserver((entries) => {
    if (!entries.some((entry) => entry.isIntersecting)) return;
    observer.disconnect();
    callback();
  }, {
    root,
    // Start a little before the element is on screen to avoid visible pop-in.
    rootMargin: '240px 0px',
  });
  observer.observe(el);
  return () => observer.disconnect();
}

export function useOnVisible(el: Ref<HTMLElement | null>, callback: () => void) {
  const root = inject(scrollRootKey, null);
  let stop: (() => void) | null = null;

  onMounted(() => {
    if (el.value) stop = observeOnce(el.value, callback, root?.value ?? null);
  });

  onUnmounted(() => stop?.());
}
