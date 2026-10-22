<script setup lang="ts">
import type { HexColor } from '../utils/color';

/** `color` is the current custom choice, or `null` when a preset is chosen instead. */
defineProps<{ color: HexColor | null; label: string }>();
const emit = defineEmits<{ pick: [color: HexColor] }>();
</script>

<template>
  <label class="swatch custom-color-swatch" :class="{ checked: color !== null }" :style="color ? { background: color } : undefined" :title="label">
    <!-- change, not input: report the final pick, not every step while dragging -->
    <input type="color" :value="color ?? '#808080'" :aria-label="label" @change="emit('pick', ($event.target as HTMLInputElement).value as HexColor)" />
  </label>
</template>

<style scoped>
.custom-color-swatch:not(.checked) { background: conic-gradient(#f44336, #ffeb3b, #4caf50, #00bcd4, #3f51b5, #e91e63, #f44336); }
.custom-color-swatch input { position: absolute; inset: 0; width: 100%; height: 100%; opacity: 0; cursor: pointer; }
</style>
