import { defineConfig } from "vite";

// Tauri는 ui/ 폴더를 프론트엔드 루트로 사용
export default defineConfig({
  root: "ui",
  // Tauri 개발 서버 설정
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  build: {
    // 빌드 결과물을 config-tool/dist 에 생성
    outDir: "../dist",
    emptyOutDir: true,
    target: "safari13",
  },
});
