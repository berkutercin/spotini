// Builds the website (site/) into dist-site/ for GitHub Pages: the landing page,
// the live demo (the app's screens built to WebAssembly by demo/build.sh) and
// the brand kit.
import fs from "node:fs";
import path from "node:path";
import { defineConfig, type Plugin } from "vite";

const root = import.meta.dirname;
const brandDir = path.join(root, "brand");
const outDir = path.join(root, "dist-site");
// What visitors get from brand/: the guide and the generated files, not the generator.
const brandSkip = new Set(["build.mjs"]);

/** Serves brand/ at /brand/ in dev and copies it into the build. */
function brandKit(): Plugin {
  return {
    name: "spotini-brand-kit",
    configureServer(server) {
      server.middlewares.use((req, res, next) => {
        const url = decodeURIComponent((req.url ?? "").split("?")[0]).replace(/^\/spotini/, "");
        if (!url.startsWith("/brand/")) return next();
        let file = path.join(brandDir, url.slice("/brand/".length));
        if (fs.existsSync(file) && fs.statSync(file).isDirectory()) file = path.join(file, "index.html");
        if (!file.startsWith(brandDir) || !fs.existsSync(file)) return next();
        const types: Record<string, string> = { ".svg": "image/svg+xml", ".png": "image/png", ".html": "text/html" };
        res.setHeader("Content-Type", types[path.extname(file)] ?? "application/octet-stream");
        fs.createReadStream(file).pipe(res);
      });
    },
    closeBundle() {
      fs.cpSync(brandDir, path.join(outDir, "brand"), {
        recursive: true,
        filter: (src) => !brandSkip.has(path.basename(src)),
      });
    },
  };
}

export default defineConfig({
  root: path.join(root, "site"),
  base: "/spotini/",
  plugins: [brandKit()],
  build: {
    outDir,
    emptyOutDir: true,
    rollupOptions: {
      input: {
        index: path.join(root, "site/index.html"),
        demo: path.join(root, "site/demo.html"),
      },
    },
  },
  server: { port: 1430, strictPort: true },
});
