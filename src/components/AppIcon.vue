<script setup lang="ts">
withDefaults(defineProps<{
  name: 'arrow-back' | 'arrow-forward' | 'chevron-left' | 'chevron-right' | 'open-in-new';
  size?: number;
}>(), { size: 16 });

const paths = {
  'arrow-back': 'M20 11H7.83l5.59-5.59L12 4l-8 8 8 8 1.41-1.41L7.83 13H20v-2z',
  'arrow-forward': 'M12 4l-1.41 1.41L16.17 11H4v2h12.17l-5.58 5.59L12 20l8-8z',
  'chevron-left': 'M15.41 7.41 14 6l-6 6 6 6 1.41-1.41L10.83 12z',
  'chevron-right': 'M8.59 16.59 13.17 12 8.59 7.41 10 6l6 6-6 6z',
  'open-in-new': 'M19 19H5V5h7V3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7h-2v7zM14 3v2h3.59l-9.83 9.83 1.41 1.41L19 6.41V10h2V3h-7z',
};
</script>

<template>
  <svg class="app-icon" viewBox="0 0 24 24" fill="currentColor" :width="size" :height="size" aria-hidden="true" focusable="false">
    <path :d="paths[name]" />
  </svg>
</template>

<style scoped>
.app-icon { display: inline-block; flex-shrink: 0; vertical-align: middle; pointer-events: none; }
</style>
