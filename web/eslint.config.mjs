import { defineConfig, globalIgnores } from "eslint/config";
import nextVitals from "eslint-config-next/core-web-vitals";
import nextTs from "eslint-config-next/typescript";

const eslintConfig = defineConfig([
  ...nextVitals,
  ...nextTs,
  // Pin the hooks rules we rely on as errors. `eslint-config-next@16.x` is a
  // moving target and has historically downgraded `react-hooks/*` to warnings
  // across minor versions; set-state-in-effect especially matters after the
  // React 19 upgrade — regressions in `alerts/page.tsx` / Auth/Theme/I18n
  // contexts already required targeted suppressions, and a drift to "warning"
  // would let silent new violations through.
  {
    rules: {
      "react-hooks/rules-of-hooks": "error",
      "react-hooks/exhaustive-deps": "error",
      "react-hooks/set-state-in-effect": "error",
    },
  },
  // Override default ignores of eslint-config-next.
  globalIgnores([
    // Default ignores of eslint-config-next:
    ".next/**",
    "out/**",
    "build/**",
    "next-env.d.ts",
  ]),
]);

export default eslintConfig;
