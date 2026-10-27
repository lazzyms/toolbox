import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { fileURLToPath } from "node:url";

const nativePlatform = () => {
  const explicit = process.env.VITE_TOOLBOX_TARGET_PLATFORM;
  if (explicit !== undefined) {
    return explicit === "macos" || explicit === "windows" ? explicit : "unsupported";
  }
  const tauriPlatform = process.env.TAURI_ENV_PLATFORM;
  if (tauriPlatform !== undefined) {
    return tauriPlatform === "macos" || tauriPlatform === "windows"
      ? tauriPlatform
      : "unsupported";
  }
  if (process.platform === "darwin") return "macos";
  if (process.platform === "win32") return "windows";
  return "unsupported";
};

export default defineConfig(async () => ({
  define: {
    "import.meta.env.VITE_TOOLBOX_TARGET_PLATFORM": JSON.stringify(nativePlatform()),
  },
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      "@": fileURLToPath(new URL("./src", import.meta.url)),
    },
  },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
  envPrefix: ["VITE_", "TAURI_ENV_"],
  build: {
    target: process.env.TAURI_ENV_PLATFORM === "windows" ? "chrome105" : "safari13",
    minify: process.env.TAURI_ENV_DEBUG ? false : "esbuild",
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
  },
}));
