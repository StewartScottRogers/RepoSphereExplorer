// Flat configuration, which is what eslint 9 reads.
export default [
  {
    files: ["src/**/*.js", "test/**/*.js"],
    languageOptions: {
      ecmaVersion: 2024,
      sourceType: "module",
    },
    rules: {
      "no-unused-vars": "error",
      "no-undef": "error",
      eqeqeq: ["error", "always"],
      "prefer-const": "error",
    },
  },
];
