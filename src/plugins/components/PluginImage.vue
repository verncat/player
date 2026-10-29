<script setup lang="ts">
import { computed, ref, useId, watch } from 'vue';
import AppIcon from '../../components/AppIcon.vue';
import { icons, type AppIconName } from '../../icons';
import type { PluginManifest } from '../types';

const props = defineProps<{ manifest: PluginManifest; monochrome?: boolean }>();
const tintId = `plugin-image-tint-${useId()}`;
const failed = ref(false);
watch(() => props.manifest.image, () => { failed.value = false; });
const fallback = computed<AppIconName>(() => {
  const name = props.manifest.contributions.find(c => c.icon)?.icon;
  return name && Object.prototype.hasOwnProperty.call(icons, name) ? name as AppIconName : 'settings';
});
</script>

<template>
  <span class="plugin-image" :class="{ 'plugin-image-monochrome': monochrome }" aria-hidden="true">
    <svg v-if="monochrome && manifest.image && !failed" class="plugin-image-filter" width="0" height="0" focusable="false">
      <defs>
        <filter :id="tintId" color-interpolation-filters="sRGB">
          <feFlood style="flood-color: var(--color-accent-text)" />
          <feComposite in2="SourceAlpha" operator="in" />
        </filter>
      </defs>
    </svg>
    <img v-if="manifest.image && !failed" :src="manifest.image" :style="monochrome ? { filter: `url(#${tintId})` } : undefined" alt="" loading="lazy" decoding="async" referrerpolicy="no-referrer" @error="failed = true" />
    <AppIcon v-else :name="fallback" :size="26" />
  </span>
</template>

<style scoped>
.plugin-image { display: grid; place-items: center; width: 56px; height: 56px; flex: 0 0 56px; overflow: hidden; border-radius: 10px; color: var(--color-text-muted); }
.plugin-image img { width: 100%; height: 100%; object-fit: contain; }
@media(max-width: 600px) { .plugin-image { width: 44px; height: 44px; flex-basis: 44px; } }
.plugin-image-monochrome { width: var(--plugin-icon-size, 19px); height: var(--plugin-icon-size, 19px); flex-basis: var(--plugin-icon-size, 19px); background: none; border-radius: 0; color: var(--color-accent-text); }
.plugin-image-monochrome > svg:not(.plugin-image-filter) { width: 100%; height: 100%; }
.plugin-image-filter { position: absolute; width: 0; height: 0; overflow: hidden; pointer-events: none; }
</style>
