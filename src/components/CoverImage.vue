<script setup lang="ts">
import { computed, ref, watch } from "vue";

// A cover image filling its box, over an optional gradient placeholder that
// shows while it loads and when it fails. Size and shape come from the
// parent's class; children go on top of the image.
const props = defineProps<{
  src: string | null;
  placeholder?: [string, string];
}>();

const failed = ref(false);
watch(() => props.src, () => { failed.value = false; });

const placeholderStyle = computed(() => (props.placeholder
  ? { background: `linear-gradient(135deg, ${props.placeholder[0]}, ${props.placeholder[1]})` }
  : undefined));
</script>

<template>
  <div class="cover-image" :style="placeholderStyle">
    <!-- crossorigin: WebKitGTK only loads cover:// images in CORS mode. -->
    <img
      v-if="src && !failed"
      :src="src"
      alt=""
      crossorigin="anonymous"
      loading="lazy"
      decoding="async"
      draggable="false"
      @error="failed = true"
    >
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
</style>
