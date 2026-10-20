<script setup lang="ts">
import AppIcon from './AppIcon.vue';

defineProps<{
  canNavBack: boolean;
  canNavForward: boolean;
  title?: string;
  subtitle?: string;
}>();

const emit = defineEmits<{
  openMenu: [];
  navBack: [];
  navForward: [];
}>();
</script>

<template>
  <header class="topbar" :class="{ 'has-page-heading': title }">
    <button class="burger-btn" @click="emit('openMenu')" aria-label="Menu">
      <AppIcon name="menu" :size="24" />
    </button>
    <div class="nav-arrows">
      <button class="arrow-btn" aria-label="Back" :disabled="!canNavBack" @click="emit('navBack')"><AppIcon name="chevron-left" :size="22" /></button>
      <button class="arrow-btn" aria-label="Forward" :disabled="!canNavForward" @click="emit('navForward')"><AppIcon name="chevron-right" :size="22" /></button>
    </div>
    <div v-if="title" class="page-heading">
      <h1 :title="title"><slot name="title">{{ title }}</slot></h1>
      <p v-if="subtitle" :title="subtitle">{{ subtitle }}</p>
    </div>
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
.page-heading h1 { color: #fff; font-size: 24px; font-weight: 800; letter-spacing: -.5px; line-height: 1.25; margin: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.page-heading p { color: #a7a7a7; font-size: 13px; line-height: 1.5; margin: 4px 0 0; display: -webkit-box; -webkit-box-orient: vertical; -webkit-line-clamp: 2; overflow: hidden; }
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
  .page-heading h1 { font-size: 20px; }
  .page-heading p { font-size: 12px; }
  .topbar.has-page-heading { display: grid; grid-template-columns: auto auto minmax(0, 1fr); }
}
</style>
