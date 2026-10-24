import adapter from '@sveltejs/adapter-static';
import { sveltekit } from '@sveltejs/kit/vite';
import { vitePreprocess } from '@sveltejs/vite-plugin-svelte';
import { defineConfig } from 'vite';
import { fileURLToPath } from 'node:url';

const host = process.env.TAURI_DEV_HOST;

/**
 * `$lib` 的落点。
 *
 * SvelteKit 3 把 `$lib` 换成了 `#lib`，但 vite 8 的打包器（rolldown）认不出
 * `#lib` ⇒ 这里用 vite 的 `resolve.alias` 把 `$lib` 映回 `src/lib`，
 * **源码里的 `$lib/...` 导入一行都不用改**。
 *
 * ⚠ 为什么不用 kit 自己的 `alias` 选项（`sveltekit({ alias: … })`）：
 *   ① 它已被 kit 标成 deprecated（提示改用 subpath imports）；
 *   ② 更要紧的是 kit 3.1.0 在 **Windows** 上会**误报**
 *      `"paths" was overwritten. Imports from "$lib" may not typecheck`
 *      —— 那是 kit 自己校验器的 bug：它把别名拼成 `src\lib/*`（正斜杠），
 *      再拿 `path.resolve` 归一化后的 `src\lib\*`（反斜杠）逐字比对，
 *      分隔符不一致就判为「被覆盖」。
 *      实测（2026-10-24）：TypeScript 解析 `$lib/*` 完全正常，
 *      但每次 `svelte-kit sync` 都会刷这条警告。
 *   ⇒ 改成下面两处各管一半：
 *      · vite 的 `resolve.alias` —— 给打包器与 dev server
 *      · tsconfig 的 `compilerOptions.paths` —— 给 TypeScript
 *      代价：「$lib 在哪」写在两个文件里 ⇒ 换目录时两处都得改。
 */
const lib = fileURLToPath(new URL('./src/lib', import.meta.url));

// Tauri 开发时需要一个固定的 dev server 地址，见 https://v2.tauri.app/start/frontend/basics
export default defineConfig(async () => ({
  plugins: [
    // SvelteKit 3 不再读 `svelte.config.js` —— 它会**主动检查该文件是否存在**，
    // 留着就直接报 `config_file_unsupported` ⇒ 配置搬进这个插件里。
    //
    // ⚠ 形状是**扁平的**：kit 3 从顶层参数里挑出自己的选项（adapter / alias /
    //   preprocess / files / paths …），不再有 `kit: { … }` 这层。
    //   写成 `sveltekit({ kit: { … } })` 会被 kit 明确拒绝
    //   （`config_kit_namespace`）。
    sveltekit({
      // Tauri 打的是本地文件协议，不是 HTTP：所有页面必须能静态导出，
      // 否则 build 出来的 index.html 里的资源路径在 file:// 下全是坏的。
      adapter: adapter({
        pages: 'build',
        assets: 'build',
        fallback: 'index.html',
        precompress: false,
        strict: true,
      }),
      // 从前在 `svelte.config.js` 顶层；kit 3 里同样是扁平参数。
      preprocess: vitePreprocess(),
    }),
  ],

  resolve: {
    alias: {
      $lib: lib,
    },
  },

  // 防止 Tauri 在 Windows 上把源码目录整个暴露出去
  clearScreen: false,

  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: 'ws', host, port: 1421 } : undefined,
    watch: {
      // Rust 侧不归 Vite 管
      ignored: ['**/src-tauri/**'],
    },
  },
}));
