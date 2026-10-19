<script setup lang="ts">
import { onUnmounted, ref, watch } from "vue";
import { useTransientHistoryEntry } from "../composables/useHistoryNavigation";

const props = defineProps<{
  src: string | null;
  fallbackSrc?: string | null;
  alt?: string;
}>();

const emit = defineEmits<{
  close: [];
}>();

const failed = ref(false);
watch(() => props.src, () => { failed.value = false; });

function onKeydown(e: KeyboardEvent) {
  if (e.key === 'Escape') {
    e.stopPropagation();
    emit('close');
  }
}
watch(() => props.src, (src) => {
  if (src) window.addEventListener('keydown', onKeydown, true);
  else window.removeEventListener('keydown', onKeydown, true);
}, { immediate: true });
onUnmounted(() => window.removeEventListener('keydown', onKeydown, true));

useTransientHistoryEntry(() => !!props.src, () => emit('close'));
</script>

<template>
  <Teleport to="body">
    <Transition name="image-preview">
      <div v-if="src" class="image-preview" @click="emit('close')">
        <img
          :src="failed && fallbackSrc ? fallbackSrc : src"
          :alt="alt ?? ''"
          crossorigin="anonymous"
          draggable="false"
          @error="failed = true"
        >
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.image-preview {
  position: fixed;
  inset: 0;
  z-index: 1100;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 32px;
  background: var(--color-backdrop, rgba(0, 0, 0, .85));
  cursor: zoom-out;
}
img {
  max-width: 100%;
  max-height: 100%;
  object-fit: contain;
  border-radius: var(--radius-cover, 6px);
  box-shadow: 0 16px 48px rgba(0, 0, 0, .6);
}
.image-preview-enter-active,
.image-preview-leave-active {
  transition: opacity .15s;
}
.image-preview-enter-from,
.image-preview-leave-to {
  opacity: 0;
}
</style>
