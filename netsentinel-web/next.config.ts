import type { NextConfig } from "next";

/**
 * Static export — the web tier is served by the Rust Axum server via
 * tower-http's ServeDir, embedded into the same image as the backend.
 * This drops the ~35 MB Node.js runtime that the old `output: 'standalone'`
 * configuration required and collapses the homelab deployment to a single
 * server container.
 *
 * The export config (`output`, `trailingSlash`, `images.unoptimized`) is
 * applied **only** when `next build` runs (NODE_ENV === 'production').
 * Under `next dev` the flags stay off, so the dev server still supports
 * App Router features normally. Host detail no longer depends on a
 * dynamic segment: the static `/host/index.html` shell reads
 * `?key=<host_key>` at runtime via `useSearchParams()`, which avoids the
 * export-time placeholder/fallback dance entirely.
 *
 * `trailingSlash: true` makes every route emit `{route}/index.html`, the
 * layout `tower-http::ServeDir` maps to naturally.
 *
 * `images.unoptimized: true` disables the build-time next/image
 * optimization pipeline — required because the exported bundle has no
 * Node runtime to run the optimizer at request time.
 */
const isExportBuild = process.env.NODE_ENV === "production";

const nextConfig: NextConfig = isExportBuild
  ? {
      output: "export",
      trailingSlash: true,
      images: { unoptimized: true },
    }
  : {};

export default nextConfig;
