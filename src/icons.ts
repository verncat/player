import { h, type Component, type FunctionalComponent } from 'vue';
import {
  ArrowLeft, ArrowRight, Check, ChevronDown, ChevronLeft, ChevronRight,
  CircleCheck, Download, ExternalLink, FolderOpen, Headphones, Heart, House,
  Info, LibraryBig, List, ListFilter, ListMusic, ListPlus, Menu, Minus,
  Palette, Pause, Pencil, Pin, Play, Plus, Radio, RefreshCw, Repeat, Repeat1, Search,
  Settings, Shuffle, SkipBack, SkipForward, Speaker, Square, Trash2,
  Volume2, VolumeX, WandSparkles, Wifi, X,
} from '@lucide/vue';

function filled(icon: Component): FunctionalComponent {
  const FilledIcon: FunctionalComponent = (_, { attrs }) => h(icon, { ...attrs, fill: attrs.color ?? 'currentColor' });
  FilledIcon.inheritAttrs = false;
  return FilledIcon;
}

// Icons are named by what they mean in the app, not by how they look, so an
// icon set can give each meaning its own glyph. Several names may share one.
export const icons = {
  // Navigation
  home: House,
  search: Search,
  library: LibraryBig,
  playlist: ListMusic,
  'smart-playlist': ListFilter,
  network: Wifi,
  settings: Settings,
  menu: Menu,
  'history-back': ChevronLeft,
  'history-forward': ChevronRight,
  back: ArrowLeft,
  go: ArrowRight,
  'page-previous': ArrowLeft,
  'page-next': ArrowRight,
  disclosure: ChevronRight,
  dropdown: ChevronDown,
  'external-link': ExternalLink,
  folder: FolderOpen,

  // Settings sections
  playback: Headphones,
  presence: Radio,
  appearance: Palette,
  info: Info,

  // Playback
  play: Play,
  pause: Pause,
  stop: Square,
  previous: SkipBack,
  next: SkipForward,
  shuffle: Shuffle,
  repeat: Repeat,
  'repeat-one': Repeat1,
  queue: ListMusic,
  'output-device': Speaker,
  volume: Volume2,
  muted: VolumeX,
  'now-playing': Play,
  'up-next': Play,
  'play-count': Play,

  // Actions
  like: Heart,
  liked: filled(Heart),
  'add-to-playlist': ListPlus,
  edit: Pencil,
  identify: WandSparkles,
  pin: Pin,
  remove: X,
  delete: Trash2,
  download: Download,
  sync: RefreshCw,
  refresh: RefreshCw,
  minimize: Minus,
  close: X,

  // Status
  selected: Check,
  done: Check,
  success: CircleCheck,
  error: X,
  activity: List,
  'update-available': Plus,
} satisfies Record<string, Component>;

export type AppIconName = keyof typeof icons;
