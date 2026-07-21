<script setup lang="ts">
defineProps<{
  canNavBack: boolean;
  canNavForward: boolean;
  showDuplicateTracks: boolean;
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
      <button class="arrow-btn" :disabled="!canNavBack" @click="emit('navBack')">&lsaquo;</button>
      <button class="arrow-btn" :disabled="!canNavForward" @click="emit('navForward')">&rsaquo;</button>
    </div>
    <button
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
  padding: 16px 24px;
  padding-top: calc(16px + env(safe-area-inset-top));
  flex-shrink: 0;
}

.nav-arrows {
  display: flex;
  gap: 8px;
}
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
    margin-right: 8px;
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
}
</style>
