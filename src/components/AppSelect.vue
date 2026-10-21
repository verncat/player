<script setup lang="ts" generic="T">
import { computed, useAttrs } from 'vue';
import AppIcon from './AppIcon.vue';

defineOptions({ inheritAttrs: false });

withDefaults(defineProps<{ variant?: 'outlined' | 'filled' }>(), { variant: 'outlined' });
const model = defineModel<T>({ required: true });

// Layout attributes stay on the wrapper; everything else (disabled, listeners) belongs to the <select>.
const attrs = useAttrs();
const selectAttrs = computed(() => {
  const { class: _class, style: _style, ...rest } = attrs;
  return rest;
});
</script>

<template>
  <span class="app-select" :class="[`app-select-${variant}`, attrs.class]" :style="attrs.style as string">
    <select v-model="model" v-bind="selectAttrs"><slot /></select>
    <AppIcon class="app-select-chevron" name="dropdown" :size="12" />
  </span>
</template>

<style scoped>
.app-select { position: relative; display: inline-flex; vertical-align: middle; }
.app-select select {
  width: 100%;
  min-height: inherit;
  color: inherit;
  line-height: 1.2;
  cursor: pointer;
  outline: none;
  appearance: none;
  -webkit-appearance: none;
  transition: border-color .15s, color .15s;
}
.app-select select:disabled { opacity: .5; cursor: default; }
.app-select option { background: var(--color-surface); color: var(--color-text); }
.app-select-chevron { position: absolute; top: 50%; right: 12px; transform: translateY(-50%); }
.app-select select:disabled + .app-select-chevron { opacity: .5; }

.app-select-outlined { min-height: 30px; color: var(--color-text-muted); }
.app-select-outlined select {
  padding: 5px 32px 5px 12px;
  border: 1px solid var(--color-border-strong);
  border-radius: var(--radius-pill);
  background: transparent;
  font-size: var(--fs-control);
  font-weight: 600;
}
.app-select-outlined:is(:hover, :focus-within) { color: var(--color-text); }
.app-select-outlined:is(:hover, :focus-within) select { border-color: var(--color-text); }

.app-select-filled { min-height: 42px; color: var(--color-text); }
.app-select-filled select {
  padding: 10px 40px 10px 12px;
  border: none;
  border-radius: var(--radius-sm);
  background: var(--color-surface-hover);
  font-size: var(--fs-field-input);
}
.app-select-filled select:focus { outline: 1px solid var(--color-accent); }
</style>
