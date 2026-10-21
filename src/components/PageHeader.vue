<script setup lang="ts">
import { ref } from "vue";
import CoverImage from "./CoverImage.vue";
import ImagePreview from "./ImagePreview.vue";

const props = defineProps<{
  kind: string;
  title: string;
  coverUrl?: string | null;
  coverGradient?: [string, string];
  coverPreviewUrl?: string | null;
}>();

const previewOpen = ref(false);
</script>

<template>
  <header class="page-header">
    <CoverImage
      v-if="coverUrl !== undefined || coverGradient"
      class="page-cover"
      :src="coverUrl ?? null"
      :placeholder="coverGradient"
      :class="{ previewable: coverPreviewUrl }"
      :role="coverPreviewUrl ? 'button' : undefined"
      :tabindex="coverPreviewUrl ? 0 : undefined"
      :aria-label="coverPreviewUrl ? 'Show full-size cover' : undefined"
      @click="previewOpen = !!coverPreviewUrl"
      @keydown.enter.space.prevent="previewOpen = !!coverPreviewUrl"
    />
    <ImagePreview
      :src="previewOpen ? props.coverPreviewUrl ?? null : null"
      :fallback-src="coverUrl"
      :alt="title"
      @close="previewOpen = false"
    />
    <div class="page-heading">
      <div class="page-kind">{{ kind }}</div>
      <h1 class="page-title">{{ title }}</h1>
      <div class="page-details"><slot name="details" /></div>
      <div class="page-actions"><slot name="actions" /></div>
    </div>
  </header>
</template>

<style scoped>
.page-header {
  display: flex;
  align-items: flex-end;
  gap: 24px;
  margin-bottom: 24px;
}
.page-cover {
  width: 180px;
  height: 180px;
  flex-shrink: 0;
  border-radius: var(--radius-cover, 6px);
  background-color: var(--color-surface);
  background-size: cover;
  background-position: center;
  box-shadow: 0 8px 24px color-mix(in srgb, var(--color-shadow) 50%, transparent);
}
.page-cover.previewable { cursor: zoom-in; }
.page-heading {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.page-kind {
  font-size: 12px;
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: .04em;
  color: var(--color-text-muted);
}
.page-title {
  font-size: 32px;
  font-weight: 800;
  color: var(--color-text);
  overflow-wrap: anywhere;
}
.page-details {
  font-size: 14px;
  color: var(--color-text-muted);
}
.page-actions {
  display: flex;
  gap: 8px;
  margin-top: 8px;
}
.page-actions:empty { display: none; }
:slotted(.play-button) {
  background: var(--color-accent);
  color: var(--color-on-accent);
  border: none;
  border-radius: 999px;
  padding: 8px 28px;
  font-size: var(--fs-button);
  font-weight: 700;
  cursor: pointer;
}
:slotted(.play-button:hover) { filter: brightness(1.1); }

@media (max-width: 640px) {
  .page-header { flex-direction: column; align-items: flex-start; gap: 16px; }
  .page-cover { width: 140px; height: 140px; }
  .page-title { font-size: 24px; }
}
</style>
