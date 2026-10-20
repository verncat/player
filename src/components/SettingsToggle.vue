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
.settings-switch { width: 42px; height: 24px; border: 1px solid #686868; border-radius: 99px; background: #363636; padding: 3px; cursor: pointer; flex-shrink: 0; transition: background .15s; }
.settings-switch span { display: block; width: 16px; height: 16px; border-radius: 50%; background: #fff; transition: transform .15s; }
.settings-switch[aria-checked=true] { background: #1db954; border-color: #1db954; }
.settings-switch[aria-checked=true] span { transform: translateX(17px); background: #061d0e; }
.settings-switch:disabled { opacity: .5; cursor: default; }
.settings-switch:focus-visible { outline: 2px solid #fff; outline-offset: 4px; }
</style>
