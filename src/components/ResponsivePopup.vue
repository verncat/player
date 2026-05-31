<script setup lang="ts">
import { nextTick, onMounted, onUnmounted, ref, watch } from 'vue';

const props = defineProps<{
  open: boolean;
  wrapperClass?: string;
  panelClass?: string;
  showActions?: boolean;
  cancelLabel?: string;
  saveLabel?: string;
  saveDisabled?: boolean;
  anchorX?: number | null;
  anchorY?: number | null;
  centered?: boolean;
}>();

const emit = defineEmits<{
  close: [];
  cancel: [];
  save: [];
}>();

const wrapperRef = ref<HTMLElement | null>(null);
const desktopPanelRef = ref<HTMLElement | null>(null);
const desktopStyle = ref<Record<string, string>>({});

function updateDesktopPosition() {
  const wrapper = wrapperRef.value;
  const panel = desktopPanelRef.value;
  if (!props.open || !wrapper || !panel || window.innerWidth <= 900) {
    desktopStyle.value = {};
    return;
  }

  const gap = 12;
  const margin = 12;
  const panelRect = panel.getBoundingClientRect();
  const panelWidth = panelRect.width;
  const panelHeight = panelRect.height;

  if (props.centered) {
    desktopStyle.value = {
      top: `${Math.round(Math.max(margin, (window.innerHeight - panelHeight) / 2))}px`,
      left: `${Math.round(Math.max(margin, (window.innerWidth - panelWidth) / 2))}px`,
    };
    return;
  }

  if (props.anchorX !== null && props.anchorX !== undefined && props.anchorY !== null && props.anchorY !== undefined) {
    const maxLeft = Math.max(margin, window.innerWidth - panelWidth - margin);
    const maxTop = Math.max(margin, window.innerHeight - panelHeight - margin);
    desktopStyle.value = {
      top: `${Math.round(Math.max(margin, Math.min(props.anchorY, maxTop)))}px`,
      left: `${Math.round(Math.max(margin, Math.min(props.anchorX, maxLeft)))}px`,
    };
    return;
  }

  const triggerRect = wrapper.getBoundingClientRect();
  const spaceBelow = window.innerHeight - triggerRect.bottom;
  const spaceAbove = triggerRect.top;
  const openBelow = spaceBelow >= panelHeight + gap || spaceBelow >= spaceAbove;

  const top = openBelow
    ? Math.min(triggerRect.bottom + gap, window.innerHeight - panelHeight - margin)
    : Math.max(margin, triggerRect.top - panelHeight - gap);
  const left = Math.min(
    Math.max(margin, triggerRect.right - panelWidth),
    window.innerWidth - panelWidth - margin,
  );

  desktopStyle.value = {
    top: `${Math.round(top)}px`,
    left: `${Math.round(left)}px`,
  };
}

watch(
  () => [props.open, props.anchorX, props.anchorY, props.centered],
  async ([open]) => {
    if (open) {
      await nextTick();
      updateDesktopPosition();
    }
  },
);

onMounted(() => {
  window.addEventListener('resize', updateDesktopPosition);
  window.addEventListener('scroll', updateDesktopPosition, true);
});

onUnmounted(() => {
  window.removeEventListener('resize', updateDesktopPosition);
  window.removeEventListener('scroll', updateDesktopPosition, true);
});
</script>

<template>
  <div ref="wrapperRef" class="responsive-popup-wrapper" :class="wrapperClass">
    <slot name="trigger" />
    <Transition name="dropdown">
      <div
        v-if="open && centered"
        class="responsive-popup-desktop-backdrop"
        @click="emit('close')"
      />
    </Transition>
    <Transition name="dropdown">
      <div
        v-if="open"
        ref="desktopPanelRef"
        class="responsive-popup-desktop dropdown"
        :class="panelClass"
        :style="desktopStyle"
        @click.stop
      >
        <div class="responsive-popup-body">
          <slot />
        </div>
        <div v-if="showActions" class="responsive-popup-actions">
          <button class="responsive-popup-cancel" @click="emit('cancel')">
            {{ cancelLabel ?? 'Cancel' }}
          </button>
          <button class="responsive-popup-save" :disabled="saveDisabled" @click="emit('save')">
            {{ saveLabel ?? 'Save' }}
          </button>
        </div>
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
          <div class="responsive-popup-body">
            <slot />
          </div>
          <div v-if="showActions" class="responsive-popup-actions">
            <button class="responsive-popup-cancel" @click="emit('cancel')">
              {{ cancelLabel ?? 'Cancel' }}
            </button>
            <button class="responsive-popup-save" :disabled="saveDisabled" @click="emit('save')">
              {{ saveLabel ?? 'Save' }}
            </button>
          </div>
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
  position: fixed;
  background: #282828;
  border-radius: 4px;
  padding: 4px 0;
  min-width: 190px;
  box-shadow: 0 16px 32px rgba(0, 0, 0, .5);
  z-index: 200;
  max-height: min(72vh, 560px);
  overflow: hidden;
  display: flex;
  flex-direction: column;
}

.responsive-popup-desktop-backdrop {
  position: fixed;
  inset: 0;
  z-index: 199;
  background: rgba(0, 0, 0, 0.42);
}

.responsive-popup-body {
  min-height: 0;
  overflow-y: auto;
  overscroll-behavior: contain;
}

.responsive-popup-body::-webkit-scrollbar {
  width: 6px;
}

.responsive-popup-body::-webkit-scrollbar-thumb {
  background: #555;
  border-radius: 3px;
}

.responsive-popup-body::-webkit-scrollbar-track {
  background: transparent;
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

.responsive-popup-desktop.settings-dropdown {
  min-width: 340px;
  max-width: 380px;
}

.responsive-popup-actions {
  display: flex;
  justify-content: flex-end;
  gap: 10px;
  padding: 12px 16px 14px;
  flex-shrink: 0;
  background: #282828;
  border-top: 1px solid rgba(255, 255, 255, 0.08);
}

.responsive-popup-cancel,
.responsive-popup-save {
  border-radius: 20px;
  font-size: var(--fs-button, 13px);
  font-weight: 700;
  padding: 8px 22px;
  cursor: pointer;
}

.responsive-popup-cancel {
  background: transparent;
  border: 1px solid #535353;
  color: #fff;
}

.responsive-popup-cancel:hover {
  border-color: #fff;
}

.responsive-popup-save {
  background: #1db954;
  border: none;
  color: #000;
}

.responsive-popup-save:hover:not(:disabled) {
  background: #1ed760;
}

.responsive-popup-save:disabled {
  opacity: 0.55;
  cursor: default;
}

@media (max-width: 900px) {
  .responsive-popup-desktop {
    display: none;
  }

  .responsive-popup-desktop-backdrop {
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
    display: flex;
    flex-direction: column;
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
