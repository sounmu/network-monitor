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
 * Shared page header that mirrors the compact card-header rhythm used on
 * the Infrastructure Overview (16/700 title, lucide icon, optional count
 * chip, actions on the right). Applying it to agents / alerts / monitors
 * / status keeps the typographic scale identical across top-level pages
 * so every route reads as part of the same app instead of five
 * differently-weighted hero banners.
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
    <header className={`page-header page-header--align-${align}`}>
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
