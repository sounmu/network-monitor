"use client";

import type { ReactNode } from "react";

interface PageHeaderProps {
  /** Leading icon — kept intentionally small (16-20px lucide icon). */
  icon?: ReactNode;
  /** Page title rendered as an <h1>. */
  title: string;
  /** Optional chip rendered right after the title (usually a count). */
  badge?: ReactNode;
  /** Optional tagline under the title row. */
  description?: string;
  /** Right-aligned slot for actions, stats, or both. */
  right?: ReactNode;
  /** Center the whole block — used by the public /status hero. */
  align?: "start" | "center";
}

/**
 * Shared page header that conforms to DESIGN.md §2.2's M3 typography
 * mapping: `<h1>` uses headline-small (20px / 400), the optional
 * description uses body-medium (14px / 400), the count chip uses
 * label-small (11px / 500) on an on-surface 8% state-layer background,
 * and the leading icon is tinted with `--md-sys-color-primary`. All
 * spacing is driven by the 4px-grid tokens from §7.1 and the chip corner
 * is `corner-small` from §3 — no raw hex or pixel literals inside the
 * component.
 *
 * Applied on every top-level page so users get the same typographic
 * rhythm at the top of every route.
 */
export function PageHeader({
  icon,
  title,
  badge,
  description,
  right,
  align = "start",
}: PageHeaderProps) {
  return (
    <header className={`glass-card page-header page-header--align-${align}`}>
      <div className="page-header__row">
        {icon && <span className="page-header__icon">{icon}</span>}
        <h1 className="page-header__title">{title}</h1>
        {badge !== undefined && badge !== null && badge !== false && (
          <span className="page-header__badge">{badge}</span>
        )}
        {right && <div className="page-header__actions">{right}</div>}
      </div>
      {description && <p className="page-header__desc">{description}</p>}
    </header>
  );
}
