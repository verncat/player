<script setup lang="ts">
import SegmentedProgressBar from './SegmentedProgressBar.vue';

defineProps<{ total: number; completed: number; failed: readonly number[]; stage: string }>();

const stageColors: Record<string, string> = {
  preparing: 'var(--color-text-faint)',
  fingerprinting: 'var(--color-info)',
  looking_up: 'var(--color-warning)',
  fetching_cover: 'var(--color-rarity-epic)',
  saving: 'var(--color-accent)',
};
const processingStages = new Set(Object.keys(stageColors));
</script>

<template>
  <SegmentedProgressBar :total="total" :completed="completed" :failed="failed"
    :processing="processingStages.has(stage)" :processing-color="stageColors[stage]"
    label="Track identification" />
</template>
