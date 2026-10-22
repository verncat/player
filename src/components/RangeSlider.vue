<script setup lang="ts">
import { ref, watch } from 'vue';

withDefaults(defineProps<{ label: string; min?: number; max?: number; step?: number; unit?: string }>(), { min: 0, max: 100, step: 1, unit: '' });
const model = defineModel<number>({ required: true });

const shown = ref(model.value);
watch(model, (value) => { shown.value = value; });
</script>

<template>
  <div class="range-slider">
    <!-- the value is reported on change, not input: the final pick, not every step while dragging -->
    <input
      type="range"
      :min="min"
      :max="max"
      :step="step"
      :value="shown"
      :aria-label="label"
      @input="shown = Number(($event.target as HTMLInputElement).value)"
      @change="model = shown"
    />
    <span class="range-slider-value">{{ shown }}{{ unit }}</span>
  </div>
</template>

<style scoped>
.range-slider { display: inline-flex; align-items: center; gap: 12px; flex-shrink: 0; }
.range-slider input { width: 160px; accent-color: var(--color-accent); cursor: pointer; }
.range-slider-value { min-width: 4ch; color: var(--color-text-muted); font-size: var(--fs-control); font-variant-numeric: tabular-nums; text-align: right; }
</style>
