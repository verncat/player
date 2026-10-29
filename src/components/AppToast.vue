<script setup lang="ts">
import { onUnmounted, watch } from 'vue';
import AppIcon from './AppIcon.vue';

const props = defineProps<{ message: string }>();
const emit = defineEmits<{ dismiss: [] }>();
let timer: ReturnType<typeof setTimeout> | undefined;
function pause() { clearTimeout(timer); timer = undefined; }
function resume() {
  pause();
  if (props.message) timer = setTimeout(() => emit('dismiss'), 6000);
}
watch(() => props.message, resume, { immediate: true });
onUnmounted(pause);
</script>

<template>
  <Teleport to="body">
    <Transition name="toast">
      <div v-if="message" class="app-toast" role="alert" @mouseenter="pause" @mouseleave="resume" @focusin="pause" @focusout="resume" @keydown.esc="emit('dismiss')">
        <AppIcon class="toast-icon" name="info" :size="20" />
        <p>{{ message }}</p>
        <button type="button" aria-label="Dismiss notification" @click="emit('dismiss')"><AppIcon name="close" :size="18" /></button>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.app-toast { position: fixed; top: calc(env(safe-area-inset-top, 0px) + 20px); right: 20px; z-index: 1200; display: flex; align-items: flex-start; gap: 12px; width: min(420px, calc(100vw - 40px)); padding: 16px; border: 1px solid var(--color-border-strong); border-radius: 12px; background: var(--color-surface); color: var(--color-text); box-shadow: 0 8px 32px color-mix(in srgb, var(--color-shadow) 24%, transparent); }
.toast-icon { margin-top: 2px; flex-shrink: 0; color: var(--color-text-muted); }
.app-toast p { flex: 1; min-width: 0; margin: 0; font-size: 13px; line-height: 1.6; overflow-wrap: anywhere; }
.app-toast button { display: grid; place-items: center; flex-shrink: 0; width: 24px; height: 24px; padding: 0; border: 0; border-radius: 4px; background: transparent; color: var(--color-text-muted); cursor: pointer; }
.app-toast button:hover { background: var(--color-surface-hover); color: var(--color-text); }
.app-toast button:focus-visible { outline: 2px solid var(--color-accent); outline-offset: 3px; }
.toast-enter-active,.toast-leave-active { transition: opacity .15s, transform .15s; }
.toast-enter-from,.toast-leave-to { opacity: 0; transform: translateY(-8px); }
@media(prefers-reduced-motion: reduce) { .toast-enter-active,.toast-leave-active { transition: none; } }
</style>
