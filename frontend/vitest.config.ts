import { svelte } from "@sveltejs/vite-plugin-svelte";
import { defineConfig } from "vitest/config";

export default defineConfig({
  plugins: [svelte()],
  resolve: { conditions: ["browser"] },
  test: {
    environment: "jsdom",
    environmentOptions: {
      jsdom: { url: "http://localhost" },
    },
    setupFiles: ["./src/web/test/setup.ts"],
    clearMocks: true,
  },
});
