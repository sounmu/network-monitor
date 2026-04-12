"use client";

import { useState } from "react";
import { usePathname } from "next/navigation";
import { Menu, X } from "lucide-react";
import Sidebar from "./Sidebar";
import { useI18n } from "@/app/i18n/I18nContext";

export default function SidebarShell() {
  const [isOpen, setIsOpen] = useState(false);
  const pathname = usePathname();
  const { t } = useI18n();

  // Auto-close sidebar on route change (mobile) — render-time state adjustment
  const [prevPathname, setPrevPathname] = useState(pathname);
  if (prevPathname !== pathname) {
    setPrevPathname(pathname);
    setIsOpen(false);
  }

  return (
    <>
      {/* Mobile hamburger button — shown below 768px via CSS */}
      <button
        className="sidebar-toggle-btn"
        onClick={() => setIsOpen((v) => !v)}
        aria-label={isOpen ? t.sidebar.closeSidebar : t.sidebar.openSidebar}
      >
        {isOpen ? <X size={20} /> : <Menu size={20} />}
      </button>

      {/* Mobile overlay backdrop */}
      {isOpen && (
        <div
          className="sidebar-overlay"
          onClick={() => setIsOpen(false)}
        />
      )}

      {/* Sidebar wrapper — mobile slide-in controlled via CSS class */}
      <div className={`sidebar-wrapper ${isOpen ? "sidebar-open" : ""}`}>
        <Sidebar />
      </div>
    </>
  );
}
