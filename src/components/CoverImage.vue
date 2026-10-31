<script setup lang="ts">
import { computed, ref, watch } from "vue";
import ImageLoader from "./ImageLoader.vue";

// A cover image filling its box, with striped loading feedback and an optional
// gradient fallback. Size and shape come from the parent's class;
// children go on top of the image.
const props = defineProps<{
  src: string | null;
  placeholder?: [string, string];
}>();

const failed = ref(false);
const loaded = ref(false);
watch(() => props.src, () => { failed.value = false; loaded.value = false; }, { flush: 'sync' });
const loading = computed(() => Boolean(props.src && !failed.value && !loaded.value));

const placeholderStyle = computed(() => (props.placeholder
  ? { background: `linear-gradient(135deg, ${props.placeholder[0]}, ${props.placeholder[1]})` }
  : undefined));
</script>

<template>
  <div class="cover-image" :style="placeholderStyle">
    <!-- crossorigin: WebKitGTK only loads cover:// images in CORS mode. -->
    <img
      v-if="src && !failed"
      :key="src"
      :src="src"
      :class="{ 'image-pending': loading }"
      alt=""
      crossorigin="anonymous"
      loading="lazy"
      decoding="async"
      draggable="false"
      @load="loaded = true"
      @error="failed = true"
    >
    <ImageLoader v-if="loading" />
    <slot />
  </div>
</template>

<style>
/* Zero specificity, so a parent's class can position the box differently. */
:where(.cover-image) {
  position: relative;
}
</style>

<style scoped>
img {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  object-fit: cover;
  border-radius: inherit;
}
.image-pending { visibility: hidden; }
</style>
