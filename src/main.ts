import { createApp, type Directive } from "vue";
import App from "./App.vue";

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

createApp(App)
  .directive("horizontal-scroll", horizontalScroll)
  .mount("#app");
