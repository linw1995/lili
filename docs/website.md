# Website and demo assets

The landing page introduces Lili as an extensible desktop pet inspired by the pet experience in the ChatGPT desktop app. The complete page is rendered by Leptos: the `Website` component owns navigation, copy, links, and layout, and directly composes the shared `PetShowcase` component. One Rust/WASM application contains the page and its interactive demo.

## Build and preview

With Nix installed, run from the repository root:

```sh
nix develop --command npm run build:site -- --release
nix develop --command node scripts/serve-site.mjs
```

The first command builds the optimized static site into `dist/site/`. The second serves that output at `http://127.0.0.1:4173/`; stop it with Ctrl+C. Nix supplies the pinned Node.js, Rust, WebAssembly target, Trunk, and wasm-bindgen tools. The build itself does not require npm dependencies or a running desktop app.

For development, `nix develop --command npm run preview:site` builds a debug version and starts the server. After source changes, rebuild and reload the page.

## GitHub Pages

The repository includes a [Website workflow](../.github/workflows/Pages.yaml). To deploy it:

1. In the GitHub repository, open **Settings → Pages → Build and deployment** and set **Source** to **GitHub Actions**. Keep the existing workflow; no generated template is needed.
2. Commit the website source, build scripts, UI changes, and `Pages.yaml`, then merge them into `main`. Do not commit `dist/` or `target/`.
3. Open **Actions → Website**. Relevant pushes to `main` build and deploy automatically. For the first deployment or a retry, select **Run workflow** on `main`.
4. Wait for both `build` and `deploy` to succeed. The deployment exposes its URL in the `github-pages` environment; the default project URL is `https://linw1995.github.io/lili/`.

The workflow builds Release output, checks `/lili/` in Chromium and WebKit, records the README animation from that build, uploads `dist/site/`, and deploys it with the built-in `GITHUB_TOKEN`. Pull requests only build, check, and upload a `website-preview` artifact. No `gh-pages` branch or extra secret is required. See [GitHub's publishing-source instructions](https://docs.github.com/en/pages/getting-started-with-github-pages/configuring-a-publishing-source-for-your-github-pages-site).

To serve the already-built artifact at the same repository subpath locally:

```sh
nix develop --command env LILI_SITE_BASE_PATH=/lili/ node scripts/serve-site.mjs
```

Open `http://127.0.0.1:4173/lili/`. All asset references are relative. Other static hosts can serve `dist/site/` too; use HTTP and the `application/wasm` MIME type for WebAssembly. Direct `file://` loading is not supported.

## Checks and README media

The `website` development shell adds the pinned ImageMagick encoder. Generate the site and its README media locally:

```sh
nix develop .#website
npm ci
npx playwright install chromium webkit
npm run capture:site -- --release
LILI_SITE_BASE_PATH=/lili/ node scripts/check-site.mjs
```

On Linux without browser system libraries, use `npx playwright install --with-deps chromium webkit`. The workflow does this automatically. To record an already-built site without rebuilding, run `npm run capture:site -- --no-build`.

The capture command records Review, Idle, Click, Running, Attention, and Failed directly from the Leptos demo. Playwright advances the actual animation controller at a fixed 10 fps; ImageMagick encodes a 9-second looping GIF with a shared palette. Export checks cover animation advancement, duration, size, and the served GIF MIME type.

Generated media lives only in the ignored build output:

- `dist/site/media/lili-demo.gif`: animated README preview, published at `https://linw1995.github.io/lili/media/lili-demo.gif`.
- `dist/site/media/lili-demo.png`: static poster from the first recorded frame.

The README references the published GIF directly and links it to the interactive website. The first successful Pages deployment must complete before that image URL is available. Do not commit either media file; the Website workflow regenerates and includes them in both deployment and PR artifacts.

```markdown
[![Lili demo](https://linw1995.github.io/lili/media/lili-demo.gif)](https://linw1995.github.io/lili/)
```

## Shared material

| Website material | Source of truth |
| --- | --- |
| Pet and notification composition | `PetPreviewScene` in `lili-ui/src/appearance.rs` |
| Landing page | `lili-ui/src/website.rs` |
| Website demo | `PetShowcase` and `site/showcase.css` |
| Notification card | `lili-ui/src/notification_carousel.rs` |
| Animation and gaze | `lili-pet` and the existing UI controllers |
| Pet and notification styles | `web/lili.css` |
| Website palette | `site/theme.css` |
| Pet artwork | `lili-pet/assets/fallback/spritesheet.webp` |
| Project mark | `plugins/lili/assets/logo.png` |
| README animation | `media/lili-demo.gif`, recorded from the built Leptos demo and published on Pages |

`AppearancePage` and `PetShowcase` share the pet component and preview controller. The `website` feature selects the `Website` entrypoint and composes the demonstration in the same document. It does not mount the configuration window, poll desktop state, install pets, or run native actions. Notification controls stay disabled, as in the desktop Appearance preview.

The page uses normal CSS document flow: there is no iframe, `postMessage`, height bridge, or separate JavaScript application. `site/index.html` is only the Trunk boot document with metadata and pre-WASM loading/recovery UI. Page content and interactions are defined in Rust. The product stylesheet and website styles share color-scheme-aware notification tokens.

`Trunk.site.toml` builds the full application into `target/site-build/`. The site builder publishes its complete JS/WASM/snippet/CSS/image graph under `dist/site/assets/<content-hash>/` and switches the root boot document after that version is ready. A local rebuild retains older versions; a clean Pages build includes only its current version. An old tab may need a reload after deployment, but cannot mix different module versions. The old `/preview/` bookmark redirects to the demonstration on the landing page.

The desktop Trunk build owns `dist/` and may clean it. Build the website after the desktop/fixture build when running both.
