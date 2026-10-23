<script setup lang="ts">
withDefaults(defineProps<{ label: string; min?: number; max?: number; step?: number; unit?: string }>(), { min: 0, max: 100, step: 1, unit: '' });
const model = defineModel<number>({ required: true });
</script>

<template>
  <div class="range-slider">
    <input
      type="range"
      :min="min"
      :max="max"
      :step="step"
      :value="model"
      :aria-label="label"
      @input="model = Number(($event.target as HTMLInputElement).value)"
    />
    <span class="range-slider-value">{{ model }}{{ unit }}</span>
  </div>
</template>

<style scoped>
.range-slider { display: inline-flex; align-items: center; gap: 12px; flex-shrink: 0; }
.range-slider input { width: 160px; accent-color: var(--color-accent); cursor: pointer; }
.range-slider-value { min-width: 4ch; color: var(--color-text-muted); font-size: var(--fs-control); font-variant-numeric: tabular-nums; text-align: right; }
</style>
