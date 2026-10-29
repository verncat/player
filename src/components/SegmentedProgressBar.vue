<script setup lang="ts">
import { computed } from 'vue';
import { progressSegments } from '../utils/progressSegments';

const props = withDefaults(defineProps<{
  total: number;
  completed: number;
  /** Zero-based indices of failed, completed items. */
  failed?: readonly number[];
  /** Animate the next item's share (at least 5% wide) without increasing the completed count. */
  processing?: boolean;
  /** Any CSS color, preferably a theme variable. */
  processingColor?: string;
  label: string;
  valueText?: string;
}>(), {
  failed: () => [],
  processing: false,
  processingColor: 'var(--color-accent)',
});

const segments = computed(() => progressSegments(props.total, props.completed, props.failed, props.processing));
</script>

<template>
  <div class="segmented-progress" role="progressbar" :aria-label="label"
    :aria-valuemin="0" :aria-valuemax="Math.max(1, segments.total)" :aria-valuenow="segments.completed"
    :aria-valuetext="valueText ?? `${segments.completed} of ${segments.total} processed; ${segments.failedCount} errors`">
    <span class="progress-completed" :style="{ width: `${segments.completedPercent}%` }" />
    <span v-for="error in segments.errors" :key="error.start" class="progress-error"
      :style="{ left: `${error.left}%`, width: `${error.width}%` }" />
    <span v-if="segments.active" class="progress-processing"
      :style="{ left: `${segments.active.left}%`, width: `${segments.active.width}%`, backgroundColor: processingColor }" />
  </div>
</template>

<style scoped>
.segmented-progress { position: relative; height: var(--progress-height, 6px); overflow: hidden; border-radius: 999px; background: var(--color-border); }
.segmented-progress > span { position: absolute; top: 0; bottom: 0; }
.progress-completed { left: 0; background: var(--color-accent); }
.progress-error { background: var(--color-text-faint); }
.progress-processing {
  background-image: linear-gradient(135deg,
    color-mix(in srgb, var(--color-text) 30%, transparent) 25%,
    transparent 25%, transparent 50%,
    color-mix(in srgb, var(--color-text) 30%, transparent) 50%,
    color-mix(in srgb, var(--color-text) 30%, transparent) 75%, transparent 75%);
  background-size: 32px 32px;
  animation: progress-stripes 1s linear infinite;
}
@keyframes progress-stripes { from { background-position: 0 0; } to { background-position: 32px 0; } }
@media (prefers-reduced-motion: reduce) { .progress-processing { animation: none; } }
</style>
