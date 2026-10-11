import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Port 1430, so the scanner's dev server never collides with TradierPro's (1420).
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 1430, strictPort: true, watch: { ignored: ["**/src-tauri/**"] } },
});
