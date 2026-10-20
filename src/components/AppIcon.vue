<script setup lang="ts">
import { icons, type AppIconName } from '../icons';

withDefaults(defineProps<{
  name: AppIconName;
  size?: number;
  color?: string;
  filled?: boolean;
}>(), { size: 16, color: 'currentColor', filled: false });
</script>

<template>
  <component :is="icons[name]" class="app-icon" :size="size" :color="color" :stroke-width="2" :fill="filled ? color : 'none'" aria-hidden="true" focusable="false" />
</template>

<style scoped>
.app-icon { display: inline-block; flex-shrink: 0; vertical-align: middle; pointer-events: none; }
</style>
