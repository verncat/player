import { computed, reactive, ref } from 'vue';

export interface AppearancePreferences {
  reduceMotion: boolean;
  album3d: boolean;
  rarityAnimations: boolean;
}

const STORAGE_KEY = 'player.appearance';
const defaults: AppearancePreferences = { reduceMotion: false, album3d: true, rarityAnimations: true };
const preferences = reactive({ ...defaults });
const storageError = ref('');
const media = typeof window !== 'undefined' ? window.matchMedia?.('(prefers-reduced-motion: reduce)') : undefined;
const systemReducedMotion = ref(media?.matches ?? false);
media?.addEventListener('change', (event) => { systemReducedMotion.value = event.matches; });

try {
  const saved = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? '{}');
  for (const key of Object.keys(defaults) as Array<keyof AppearancePreferences>) {
    if (typeof saved?.[key] === 'boolean') preferences[key] = saved[key];
  }
} catch {
  // An unreadable preference never prevents the player from starting.
}

const reducedMotion = computed(() => preferences.reduceMotion || systemReducedMotion.value);

function setPreference(key: keyof AppearancePreferences, value: boolean) {
  const next = { ...preferences, [key]: value };
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(next));
    Object.assign(preferences, next);
    storageError.value = '';
  } catch {
    storageError.value = 'Could not save appearance preferences on this device. Please try again.';
  }
}

export function useAppearance() {
  return { preferences, reducedMotion, systemReducedMotion, storageError, setPreference };
}
