<script setup lang="ts">
defineProps<{
  open: boolean;
  wrapperClass?: string;
  panelClass?: string;
}>();

const emit = defineEmits<{
  close: [];
}>();
</script>

<template>
  <div class="responsive-popup-wrapper" :class="wrapperClass">
    <slot name="trigger" />
    <Transition name="dropdown">
      <div v-if="open" class="responsive-popup-desktop dropdown" :class="panelClass">
        <slot />
      </div>
    </Transition>
  </div>

  <Teleport to="body">
    <Transition name="mobile-popup">
      <div
        v-if="open"
        class="responsive-popup-mobile"
        @click.stop="emit('close')"
      >
        <div
          class="responsive-popup-mobile-panel dropdown"
          :class="panelClass"
          @click.stop
        >
          <slot />
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style>
.responsive-popup-wrapper {
  position: relative;
}

.responsive-popup-desktop {
  position: absolute;
  top: auto;
  right: 0;
  bottom: calc(100% + 12px);
  background: #282828;
  border-radius: 4px;
  padding: 4px 0;
  min-width: 190px;
  box-shadow: 0 16px 32px rgba(0, 0, 0, .5);
  z-index: 200;
}

.responsive-popup-mobile {
  display: none;
}

.responsive-popup-desktop.queue-dropdown {
  min-width: 300px;
  max-width: 360px;
}

.responsive-popup-desktop.device-dropdown {
  min-width: 220px;
}

@media (max-width: 900px) {
  .responsive-popup-desktop {
    display: none;
  }

  .responsive-popup-mobile {
    position: fixed;
    inset: 0;
    z-index: 1000;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: max(12px, env(safe-area-inset-top)) 12px max(12px, env(safe-area-inset-bottom));
    background: rgba(0, 0, 0, 0.56);
  }

  .responsive-popup-mobile-panel {
    position: relative !important;
    top: auto !important;
    right: auto !important;
    bottom: auto !important;
    left: auto !important;
    width: calc(100vw - 24px) !important;
    min-width: 0 !important;
    max-width: calc(100vw - 24px) !important;
    max-height: min(72vh, 560px);
    overflow: hidden;
    transform: none;
    background: #282828;
    border-radius: 4px;
    padding: 4px 0;
    box-shadow: 0 16px 32px rgba(0, 0, 0, .5);
  }

  .mobile-popup-enter-active,
  .mobile-popup-leave-active {
    transition: opacity .14s;
  }

  .mobile-popup-enter-active .responsive-popup-mobile-panel,
  .mobile-popup-leave-active .responsive-popup-mobile-panel {
    transition: transform .14s, opacity .14s;
  }

  .mobile-popup-enter-from,
  .mobile-popup-leave-to {
    opacity: 0;
  }

  .mobile-popup-enter-from .responsive-popup-mobile-panel,
  .mobile-popup-leave-to .responsive-popup-mobile-panel {
    opacity: 0;
    transform: scale(.98);
  }
}
</style>
