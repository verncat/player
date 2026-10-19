<script setup lang="ts">
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
        <svg viewBox="0 0 24 24" fill="currentColor" width="22" height="22"><path d="M19 6.41 17.59 5 12 10.59 6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 12 13.41 17.59 19 19 17.59 13.41 12z"/></svg>
      </button>
    </div>

    <nav>
      <a class="nav-item" :class="{ active: activeNav === 'home' }" @click.prevent="emit('navigateHome')" href="#">
        <svg viewBox="0 0 24 24" fill="currentColor" width="22" height="22"><path d="M10 20v-6h4v6h5v-8h3L12 3 2 12h3v8z"/></svg>
        Home
      </a>
      <a class="nav-item" :class="{ active: activeNav === 'search' }" @click.prevent="emit('navigateSearch')" href="#">
        <svg viewBox="0 0 24 24" fill="currentColor" width="22" height="22"><path d="M15.5 14h-.79l-.28-.27A6.471 6.471 0 0 0 16 9.5 6.5 6.5 0 1 0 9.5 16c1.61 0 3.09-.59 4.23-1.57l.27.28v.79l5 4.99L20.49 19l-4.99-5zm-6 0C7.01 14 5 11.99 5 9.5S7.01 5 9.5 5 14 7.01 14 9.5 11.99 14 9.5 14z"/></svg>
        Search
      </a>
      <a class="nav-item" :class="{ active: activeNav === 'library' }" @click.prevent="emit('navigateLibrary')" href="#">
        <svg viewBox="0 0 24 24" fill="currentColor" width="22" height="22"><path d="M4 6H2v14c0 1.1.9 2 2 2h14v-2H4V6zm16-4H8c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h12c1.1 0 2-.9 2-2V4c0-1.1-.9-2-2-2zm-1 9H9V9h10v2zm-4 4H9v-2h6v2zm4-8H9V5h10v2z"/></svg>
        Your Library
      </a>
      <a class="nav-item" :class="{ active: activeNav === 'playlists' }" @click.prevent="emit('navigatePlaylists')" href="#">
        <svg viewBox="0 0 24 24" fill="currentColor" width="22" height="22"><path d="M15 6H3v2h12V6zm0 4H3v2h12v-2zM3 16h8v-2H3v2zM17 6v8.18c-.31-.11-.65-.18-1-.18-1.66 0-3 1.34-3 3s1.34 3 3 3 3-1.34 3-3V8h3V6h-5z"/></svg>
        Playlists
      </a>
      <a class="nav-item" :class="{ active: activeNav === 'playlists' && playlistTab === 'smart' }" @click.prevent="emit('navigateSmartPlaylists')" href="#">
        <svg viewBox="0 0 24 24" fill="currentColor" width="22" height="22"><path d="M19 3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2V5a2 2 0 0 0-2-2zm-7 3c1.93 0 3.5 1.57 3.5 3.5S13.93 13 12 13s-3.5-1.57-3.5-3.5S10.07 6 12 6zm7 13H5v-.23c0-.62.28-1.2.76-1.58C7.47 15.82 9.64 15 12 15s4.53.82 6.24 2.19c.48.38.76.97.76 1.58V19z"/></svg>
        Flexible Playlists
      </a>
    </nav>

    <div class="sidebar-divider" />

    <nav>
      <a class="nav-item" :class="{ active: activeNav === 'discovery' }" @click.prevent="emit('navigateDiscovery')" href="#">
        <svg viewBox="0 0 24 24" fill="currentColor" width="22" height="22"><path d="M1 9l2 2c4.97-4.97 13.03-4.97 18 0l2-2C16.93 2.93 7.08 2.93 1 9zm8 8 3 3 3-3a4.237 4.237 0 0 0-6 0zm-4-4 2 2a7.074 7.074 0 0 1 10 0l2-2C15.14 9.14 8.87 9.14 5 13z"/></svg>
        Devices
        <span v-if="syncActive" class="nav-sync-spinner" aria-label="Sync in progress" />
        <span v-else-if="peersCount" class="peer-badge">{{ peersCount }}</span>
      </a>
    </nav>
    <nav class="sidebar-settings">
      <a class="nav-item" :class="{ active: activeNav === 'settings' || activeNav.startsWith('settings/') || activeNav === 'about' }" href="#" @click.prevent="emit('navigateSettings')">
        <svg viewBox="0 0 24 24" fill="currentColor" width="22" height="22" aria-hidden="true"><path d="m19.4 13 .1-1-.1-1 2.1-1.6-2-3.4-2.5 1a8 8 0 0 0-1.7-1L15 3h-4l-.4 2.9-1.7 1-2.5-1-2 3.4L6.5 11l-.1 1 .1 1-2.1 1.6 2 3.4 2.5-1 1.7 1L11 21h4l.4-2.9 1.7-1 2.5 1 2-3.4zM13 16a4 4 0 1 1 0-8 4 4 0 0 1 0 8z" /></svg>
        Settings
        <span v-if="updateAvailable" class="peer-badge" aria-label="Update available">+</span>
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
  .nav-item svg {
    width: 20px;
    height: 20px;
  }
}

@media (max-width: 480px) {
  .nav-item svg {
    width: 18px;
    height: 18px;
  }
  .nav-item {
    padding: 10px 12px;
    font-size: var(--fs-nav);
  }
}
</style>
