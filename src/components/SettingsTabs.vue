<script setup lang="ts" generic="T extends string">
import { useId } from 'vue';

const props = defineProps<{ options: readonly { value: T; label: string }[]; label: string }>();
const model = defineModel<T>({ required: true });
const id = `settings-tabs-${useId()}`;

function navigate(event: KeyboardEvent) {
  if (!props.options.length || !['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return;
  event.preventDefault();
  const current = Math.max(0, props.options.findIndex(option => option.value === model.value));
  const next = event.key === 'Home' ? 0 : event.key === 'End' ? props.options.length - 1
    : (current + (event.key === 'ArrowRight' ? 1 : -1) + props.options.length) % props.options.length;
  model.value = props.options[next].value;
  (event.currentTarget as HTMLElement).querySelectorAll<HTMLButtonElement>('[role="tab"]')[next]?.focus();
}
</script>

<template>
  <div class="settings-tabs">
    <div class="settings-tab-list" role="tablist" :aria-label="label" @keydown="navigate">
      <button v-for="option in options" :id="`${id}-${option.value}`" :key="option.value" type="button" role="tab"
        :aria-selected="model === option.value" :aria-controls="`${id}-panel`" :tabindex="model === option.value ? 0 : -1"
        @click="model = option.value">{{ option.label }}</button>
    </div>
    <div :id="`${id}-panel`" role="tabpanel" :aria-labelledby="`${id}-${model}`" tabindex="0">
      <slot />
    </div>
  </div>
</template>

<style scoped>
.settings-tabs { display: grid; gap: 20px; min-width: 0; }
.settings-tab-list { display: flex; gap: 20px; overflow-x: auto; border-bottom: 1px solid var(--color-border); }
.settings-tab-list button { flex-shrink: 0; padding: 10px 0; border: 0; border-radius: 0; background: transparent; color: var(--color-text-muted); font-size: var(--fs-control); font-weight: 600; cursor: pointer; }
.settings-tab-list button:hover { color: var(--color-text); }
.settings-tab-list button[aria-selected="true"] { color: var(--color-accent-text); box-shadow: inset 0 -2px var(--color-accent); }
.settings-tab-list button:focus-visible { outline: 2px solid var(--color-accent); outline-offset: -2px; }
[role="tabpanel"] { min-width: 0; }
[role="tabpanel"]:focus-visible { outline: 2px solid var(--color-accent); outline-offset: 4px; }
</style>
