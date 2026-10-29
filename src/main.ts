import { createApp, type Directive } from "vue";
import App from "./App.vue";
import { startTheme } from "./composables/useTheme";
import "./styles/appearance.css";
import "./styles/base.css";
import "./styles/glass.css";
import "./styles/rarity.css";
import "./styles/button-controls.css";
import "./styles/button-loading.css";
import "./styles/form-controls.css";
import "./styles/tokens.css";

const horizontalScroll: Directive = {
  mounted(el: HTMLElement) {
    el.addEventListener("wheel", (e: WheelEvent) => {
      if (Math.abs(e.deltaY) <= Math.abs(e.deltaX)) return;
      const { scrollWidth, clientWidth } = el;
      if (scrollWidth <= clientWidth) return;
      e.preventDefault();
      el.scrollBy({ left: e.deltaY, behavior: "instant" as ScrollBehavior });
    }, { passive: false });
  },
};

startTheme();

createApp(App)
  .directive("horizontal-scroll", horizontalScroll)
  .mount("#app");
