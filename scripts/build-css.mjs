import { copyFile, mkdir } from "node:fs/promises";

await mkdir("dist/assets", { recursive: true });
await copyFile("web/lili.css", "dist/assets/lili.css");
