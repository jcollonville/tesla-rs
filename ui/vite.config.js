import { resolve } from "node:path";

export default {
  build: {
    lib: {
      entry: resolve("src/index.js"),
      formats: ["es"],
      fileName: () => "tesla-logger-ui.js",
    },
    outDir: "dist",
    emptyOutDir: true,
    sourcemap: false,
    minify: true,
    cssCodeSplit: false,
    rollupOptions: {
      output: { inlineDynamicImports: true },
    },
  },
};
