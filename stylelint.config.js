export default {
  extends: ['stylelint-config-standard'],
  rules: {
    'no-duplicate-selectors': true,
    'selector-class-pattern': null,
    'custom-property-pattern': null,
    'keyframes-name-pattern': null,
  },
  // 忽略 Tauri 的 Rust 源码目录
  ignoreFiles: [
    'src-tauri/**/*',
    'dist/**/*',
    'node_modules/**/*'
  ]
};