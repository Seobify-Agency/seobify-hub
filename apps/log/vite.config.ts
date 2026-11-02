import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

const host = process.env.TAURI_DEV_HOST;
// Each hub tool has its own dev port so several tools can run `tauri dev` at once.
const PORT = 1431;

export default defineConfig(async () => ({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: PORT,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: PORT + 100 } : undefined,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  test: {
    environment: "node",
  },
}));
