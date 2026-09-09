import { vitePreprocess } from "@sveltejs/vite-plugin-svelte";

export default {
  preprocess: vitePreprocess(),
  compilerOptions: {
    // Runes, so reactivity is explicit rather than inferred from an
    // assignment somewhere in the file.
    runes: true,
  },
};
