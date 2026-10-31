<template>
  <span class="image-loader loading-stripes" aria-hidden="true" />
</template>

<style scoped>
.image-loader {
  position: absolute;
  inset: 0;
  border-radius: inherit;
  background-color: var(--color-surface);
  color: var(--color-accent-text);
  pointer-events: none;
}
</style>
