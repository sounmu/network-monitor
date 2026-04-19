import { Suspense } from "react";

import HostPageClient from "./HostPageClient";

/**
 * Host detail is driven by a runtime query parameter (`?key=<host_key>`)
 * instead of a dynamic segment. `output: 'export'` serialises any
 * `[host_key]` segment into the baked HTML payload, which makes the
 * client router treat the build-time placeholder as the source of
 * truth — no amount of `window.location.pathname` reading can override
 * that. A plain static page + `useSearchParams()` reads the live URL
 * deterministically instead.
 *
 * `useSearchParams` is a Client hook; Next.js 16 requires its callers
 * to sit inside a Suspense boundary.
 */
export default function HostPage() {
  return (
    <Suspense fallback={<div className="skeleton" style={{ height: 320 }} />}>
      <HostPageClient />
    </Suspense>
  );
}
