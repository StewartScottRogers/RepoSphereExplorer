import js from "@eslint/js";
import vue from "eslint-plugin-vue";

export default [
  js.configs.recommended,
  ...vue.configs["flat/recommended"],
  {
    files: ["src/**/*.{js,vue}", "test/**/*.js"],
    languageOptions: {
      ecmaVersion: 2024,
      sourceType: "module",
    },
    rules: {
      "vue/multi-word-component-names": "error",
      "vue/require-default-prop": "error",
      "no-unused-vars": "error",
      eqeqeq: ["error", "always"],
    },
  },
];
