<script setup lang="ts">
defineProps<{ modelValue: boolean; label: string; disabled?: boolean }>();
const emit = defineEmits<{ 'update:modelValue': [value: boolean] }>();
</script>

<template>
  <button type="button" class="settings-switch" role="switch" :aria-checked="modelValue" :aria-label="label" :disabled="disabled" @click="emit('update:modelValue', !modelValue)">
    <span />
  </button>
</template>

<style scoped>
.settings-switch { width: 42px; height: 24px; border: 1px solid var(--color-border-strong); border-radius: 99px; background: var(--color-surface-hover); padding: 3px; cursor: pointer; flex-shrink: 0; transition: background .15s; }
.settings-switch span { display: block; width: 16px; height: 16px; border-radius: 50%; background: var(--color-text); transition: transform .15s; }
.settings-switch[aria-checked=true] { background: var(--color-accent); border-color: var(--color-accent); }
.settings-switch[aria-checked=true] span { transform: translateX(17px); background: var(--color-on-accent); }
.settings-switch:disabled { opacity: .5; cursor: default; }
.settings-switch:focus-visible { outline: 2px solid var(--color-text); outline-offset: 4px; }
</style>
