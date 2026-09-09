import { createApp } from "vue";

import TaskBoard from "./components/TaskBoard.vue";

// The shell mounts the component and nothing else. Anything worth testing
// belongs in the component, where a test can reach it without a browser.
createApp(TaskBoard).mount("#app");
