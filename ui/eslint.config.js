import js from '@eslint/js';
import svelte from 'eslint-plugin-svelte';
import globals from 'globals';
import ts from 'typescript-eslint';

// eslint-plugin-svelte **v2.46 与 eslint 9.39 不兼容**：
// 它包装核心规则时调 `Object.create(coreRule, options)`，而 eslint 9
// 的 `no-inner-declarations` 改成要 `context.options[1]`——v2 传的是
// 单参数，于是 `context.options[1]` 是 undefined，一进 .svelte 就炸。
//
// 这不是配置写错，是版本错配。四次报错连起来看才看清：
//   ① `configs.recommended` 用 `extends`（flat 不支持）
//   ② `flat/recommended` 是「数组的数组」，要展开
//   ③ 展开后核心规则包装层崩溃 → **这才是根因**
//
// 修法是升到 v3：v3 重新实现了对 eslint 9 的规则包装。
// 顺手也解决了 ①②——v3 的配置是纯扁平的。

const svelteFlat = svelte.configs['flat/recommended'];
if (!Array.isArray(svelteFlat)) {
  throw new Error(
    `eslint-plugin-svelte 的 flat/recommended 形状变了（实际 ${typeof svelteFlat}）。` +
      '升级插件后请重新确认——这里刻意用断言挡住静默的语义变化。',
  );
}

export default [
  {
    ignores: ['build/', '.svelte-kit/', 'node_modules/', 'src-tauri/target/', 'src-tauri/gen/'],
  },
  js.configs.recommended,
  ...ts.configs.recommended,
  ...svelteFlat,
  {
    languageOptions: {
      globals: { ...globals.browser, ...globals.node },
    },
    rules: {
      // Svelte 5 用 $state/$derived 表达响应式；模板里引用的变量会被
      // 基础规则的 no-unused-vars 误报。
      'no-unused-vars': 'off',
    },
  },
  {
    files: ['**/*.svelte', '**/*.svelte.ts', '**/*.svelte.js'],
    languageOptions: {
      parserOptions: {
        projectService: true,
        extraFileExtensions: ['.svelte'],
        parser: ts.parser,
      },
    },
  },
];
