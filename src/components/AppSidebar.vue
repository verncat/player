<script setup lang="ts">
import AppIcon from './AppIcon.vue';
defineProps<{
  activeNav: string;
  playlistTab: "regular" | "smart";
  open: boolean;
  syncActive: boolean;
  peersCount: number;
  updateAvailable: boolean;
}>();

const emit = defineEmits<{
  close: [];
  navigateHome: [];
  navigateSearch: [];
  navigateLibrary: [];
  navigatePlaylists: [];
  navigateSmartPlaylists: [];
  navigateDiscovery: [];
  navigateSettings: [];
}>();
</script>

<template>
  <aside class="sidebar" :class="{ 'sidebar-open': open }">
    <div class="sidebar-mobile-header">
      <button class="icon-btn sidebar-close-btn" @click="emit('close')" aria-label="Close menu">
        <AppIcon name="x" :size="22" />
      </button>
    </div>

    <nav>
      <a class="nav-item" :class="{ active: activeNav === 'home' }" @click.prevent="emit('navigateHome')" href="#">
        <AppIcon name="house" :size="22" />
        Home
      </a>
      <a class="nav-item" :class="{ active: activeNav === 'search' }" @click.prevent="emit('navigateSearch')" href="#">
        <AppIcon name="search" :size="22" />
        Search
      </a>
      <a class="nav-item" :class="{ active: activeNav === 'library' }" @click.prevent="emit('navigateLibrary')" href="#">
        <AppIcon name="library-big" :size="22" />
        Your Library
      </a>
      <a class="nav-item" :class="{ active: activeNav === 'playlists' }" @click.prevent="emit('navigatePlaylists')" href="#">
        <AppIcon name="list-music" :size="22" />
        Playlists
      </a>
      <a class="nav-item" :class="{ active: activeNav === 'playlists' && playlistTab === 'smart' }" @click.prevent="emit('navigateSmartPlaylists')" href="#">
        <AppIcon name="list-filter" :size="22" />
        Flexible Playlists
      </a>
    </nav>

    <div class="sidebar-divider" />

    <nav>
      <a class="nav-item" :class="{ active: activeNav === 'discovery' }" @click.prevent="emit('navigateDiscovery')" href="#">
        <AppIcon name="wifi" :size="22" />
        Devices
        <span v-if="syncActive" class="nav-sync-spinner" aria-label="Sync in progress" />
        <span v-else-if="peersCount" class="peer-badge">{{ peersCount }}</span>
      </a>
    </nav>
    <nav class="sidebar-settings">
      <a class="nav-item" :class="{ active: activeNav === 'settings' || activeNav.startsWith('settings/') || activeNav === 'about' }" href="#" @click.prevent="emit('navigateSettings')">
        <AppIcon name="settings" :size="22" />
        Settings
        <span v-if="updateAvailable" class="peer-badge" aria-label="Update available"><AppIcon name="plus" :size="12" /></span>
      </a>
    </nav>
  </aside>
</template>

<style scoped>
.sidebar {
  grid-row: 1 / 2;
  background: #000;
  padding: 20px 8px;
  display: flex;
  flex-direction: column;
  gap: 6px;
  overflow-y: auto;
}
.sidebar-settings { margin-top: auto; padding-top: 24px; }
.sidebar::-webkit-scrollbar { display: none; }

.nav-item {
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 9px 12px;
  border-radius: 6px;
  color: #b3b3b3;
  text-decoration: none;
  font-size: var(--fs-nav);
  font-weight: 600;
  cursor: pointer;
  transition: color .12s;
  user-select: none;
  position: relative;
}
.nav-item:hover { color: #fff; }
.nav-item:focus-visible { outline: 2px solid #1db954; outline-offset: -2px; }
.nav-item.active { color: #fff; }

.peer-badge {
  margin-left: auto;
  background: #1db954;
  color: #000;
  font-size: var(--fs-eyebrow);
  font-weight: 700;
  width: 18px;
  height: 18px;
  border-radius: 9px;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 0;
}
.nav-sync-spinner {
  box-sizing: border-box;
  margin-left: auto;
  width: 18px;
  height: 18px;
  border: 2px solid #555;
  border-top-color: #1db954;
  border-radius: 50%;
  animation: spin 0.75s linear infinite;
  flex-shrink: 0;
}

.sidebar-divider {
  height: 1px;
  background: #282828;
  margin: 8px 12px;
}

.icon-btn {
  background: none;
  border: none;
  color: #b3b3b3;
  cursor: pointer;
  padding: 4px;
  display: flex;
  align-items: center;
  justify-content: center;
  position: relative;
  border-radius: 4px;
  transition: color .1s;
}
.icon-btn:hover { color: #fff; }

.mobile-nav-overlay { display: none; }
.sidebar-mobile-header { display: none; }

@keyframes spin {
  to { transform: rotate(360deg); }
}

@media (max-width: 768px) {
  .sidebar {
    position: fixed;
    top: 0;
    left: 0;
    bottom: 0;
    width: 260px;
    z-index: 400;
    flex-direction: column;
    padding: 0 0 24px;
    padding-top: env(safe-area-inset-top);
    gap: 0;
    overflow-x: hidden;
    overflow-y: auto;
    border-bottom: none;
    border-right: 1px solid #282828;
    background: #121212;
    transform: translateX(-100%);
    transition: transform 0.28s cubic-bezier(0.4, 0, 0.2, 1);
  }
  .sidebar.sidebar-open {
    transform: translateX(0);
  }
  .sidebar-mobile-header {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    padding: 12px 12px 4px;
    flex-shrink: 0;
  }
  .sidebar-close-btn {
    color: #b3b3b3;
    padding: 8px;
  }
  .sidebar nav {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 4px 8px;
    width: 100%;
  }
  .sidebar .sidebar-divider { display: block; }
  .nav-item {
    gap: 12px;
    padding: 12px 14px;
    font-size: var(--fs-nav);
    white-space: nowrap;
    flex-shrink: 0;
    border-radius: 6px;
  }
  .nav-item > svg {
    width: 20px;
    height: 20px;
  }
}

@media (max-width: 480px) {
  .nav-item > svg {
    width: 18px;
    height: 18px;
  }
  .nav-item {
    padding: 10px 12px;
    font-size: var(--fs-nav);
  }
}
</style>
