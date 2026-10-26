<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import AppIcon from '../../components/AppIcon.vue';
import { icons, type AppIconName } from '../../icons';
import type { PluginManifest } from '../types';

const props = defineProps<{ manifest: PluginManifest }>();
const failed = ref(false);
watch(() => props.manifest.image, () => { failed.value = false; });
const fallback = computed<AppIconName>(() => {
  const name = props.manifest.contributions.find(c => c.icon)?.icon;
  return name && Object.prototype.hasOwnProperty.call(icons, name) ? name as AppIconName : 'settings';
});
</script>

<template>
  <span class="plugin-image" aria-hidden="true">
    <img v-if="manifest.image && !failed" :src="manifest.image" alt="" loading="lazy" decoding="async" referrerpolicy="no-referrer" @error="failed = true" />
    <AppIcon v-else :name="fallback" :size="26" />
  </span>
</template>

<style scoped>
.plugin-image { display: grid; place-items: center; width: 56px; height: 56px; flex: 0 0 56px; overflow: hidden; border-radius: 10px; color: var(--color-text-muted); }
.plugin-image img { width: 100%; height: 100%; object-fit: contain; }
@media(max-width: 600px) { .plugin-image { width: 44px; height: 44px; flex-basis: 44px; } }
</style>
