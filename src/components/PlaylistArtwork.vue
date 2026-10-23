<script setup lang="ts">
import AppIcon from './AppIcon.vue';

defineProps<{ kind: 'regular' | 'smart' }>();
</script>

<template>
  <div class="playlist-artwork" :class="`playlist-artwork-${kind}`">
    <AppIcon :name="kind === 'smart' ? 'smart-playlist' : 'playlist'" :size="20" />
  </div>
</template>

<style scoped>
.playlist-artwork { display: flex; align-items: center; justify-content: center; }
.playlist-artwork-regular {
  background: linear-gradient(135deg, var(--color-surface-hover), var(--color-card));
  color: var(--color-text-muted);
}
.playlist-artwork-smart {
  background: linear-gradient(135deg, color-mix(in srgb, var(--color-info) 15%, var(--color-card)), color-mix(in srgb, var(--color-info) 12%, var(--color-bg)));
  color: var(--color-info);
}
</style>
