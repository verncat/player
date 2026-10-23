<script setup lang="ts" generic="T">
defineProps<{ options: readonly { value: T; label: string; background: string }[]; label: string }>();
const model = defineModel<T>({ required: true });
</script>

<template>
  <div class="swatch-picker" role="radiogroup" :aria-label="label">
    <button
      v-for="option in options"
      :key="option.label"
      type="button"
      role="radio"
      class="swatch"
      :aria-checked="model === option.value"
      :aria-label="option.label"
      :title="option.label"
      :style="{ background: option.background }"
      @click="model = option.value"
    />
    <slot />
  </div>
</template>

<style scoped>
.swatch-picker { display: flex; flex-wrap: wrap; gap: 10px; }
.swatch, :slotted(.swatch) {
  position: relative;
  width: 28px;
  height: 28px;
  padding: 0;
  border: 0;
  border-radius: 50%;
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--color-text) 12%, transparent);
  cursor: pointer;
}
.swatch[aria-checked="true"], :slotted(.swatch.checked) { outline: 2px solid var(--color-text); outline-offset: 3px; }
.swatch:focus-visible, :slotted(.swatch:focus-within) { outline: 2px solid var(--color-accent); outline-offset: 3px; }
</style>
