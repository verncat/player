<script setup lang="ts">
import AppIcon from './AppIcon.vue';

defineProps<{
  canNavBack: boolean;
  canNavForward: boolean;
  showDuplicateTracks: boolean;
  showDuplicateToggle?: boolean;
  title?: string;
  subtitle?: string;
}>();

const emit = defineEmits<{
  openMenu: [];
  navBack: [];
  navForward: [];
  "update:showDuplicateTracks": [value: boolean];
}>();
</script>

<template>
  <header class="topbar">
    <button class="burger-btn" @click="emit('openMenu')" aria-label="Menu">
      <svg viewBox="0 0 24 24" fill="currentColor" width="24" height="24"><path d="M3 18h18v-2H3v2zm0-5h18v-2H3v2zm0-7v2h18V6H3z"/></svg>
    </button>
    <div class="nav-arrows">
      <button class="arrow-btn" aria-label="Back" :disabled="!canNavBack" @click="emit('navBack')"><AppIcon name="chevron-left" :size="22" /></button>
      <button class="arrow-btn" aria-label="Forward" :disabled="!canNavForward" @click="emit('navForward')"><AppIcon name="chevron-right" :size="22" /></button>
    </div>
    <div v-if="title" class="page-heading">
      <h1>{{ title }}</h1>
      <p v-if="subtitle">{{ subtitle }}</p>
    </div>
    <button
      v-if="showDuplicateToggle !== false"
      class="duplicates-toggle"
      :class="{ active: showDuplicateTracks }"
      @click="emit('update:showDuplicateTracks', !showDuplicateTracks)"
    >
      {{ showDuplicateTracks ? "Hide duplicates" : "Show duplicates" }}
    </button>
  </header>
</template>

<style scoped>
.topbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 20px;
  padding: 16px 24px;
  padding-top: calc(16px + env(safe-area-inset-top));
  flex-shrink: 0;
}

.nav-arrows {
  display: flex;
  gap: 8px;
  flex-shrink: 0;
}
.page-heading { flex: 1; min-width: 0; overflow-wrap: anywhere; }
.page-heading h1 { color: #fff; font-size: 24px; font-weight: 800; letter-spacing: -.5px; line-height: 1.25; margin: 0; }
.page-heading p { color: #a7a7a7; font-size: 13px; line-height: 1.5; margin: 4px 0 0; }
.arrow-btn {
  width: 32px;
  height: 32px;
  border-radius: 50%;
  background: rgba(0,0,0,.45);
  border: none;
  color: #fff;
  font-size: 22px;
  line-height: 1;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
}
.arrow-btn:hover { background: rgba(0,0,0,.7); }

.duplicates-toggle {
  margin-left: auto;
  flex-shrink: 0;
  border: 1px solid #4a4a4a;
  background: rgba(20, 20, 20, 0.75);
  color: #d9dee8;
  font-size: 12px;
  font-weight: 700;
  letter-spacing: 0.02em;
  padding: 7px 12px;
  border-radius: 999px;
  cursor: pointer;
  transition: background .15s, border-color .15s, color .15s;
}
.duplicates-toggle:hover {
  background: rgba(36, 36, 36, 0.92);
}
.duplicates-toggle.active {
  background: rgba(29, 185, 84, 0.16);
  border-color: rgba(29, 185, 84, 0.7);
  color: #8cf7b0;
}

.burger-btn {
  display: none;
}

@media (max-width: 768px) {
  .burger-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    background: none;
    border: none;
    color: #fff;
    cursor: pointer;
    padding: 6px;
    flex-shrink: 0;
  }
  .topbar {
    padding: 10px 14px;
    padding-top: calc(10px + env(safe-area-inset-top));
    gap: 8px;
  }
  .duplicates-toggle {
    margin-left: auto;
    font-size: 11px;
    padding: 6px 10px;
    white-space: nowrap;
  }
  .page-heading h1 { font-size: 20px; }
  .page-heading p { font-size: 12px; }
}
</style>
