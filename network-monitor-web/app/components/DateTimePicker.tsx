"use client";

import { useState, useRef, useEffect, useCallback } from "react";
import { Calendar, ChevronLeft, ChevronRight, Clock } from "lucide-react";
import { useI18n } from "@/app/i18n/I18nContext";

interface DateTimePickerProps {
  value: Date;
  onChange: (date: Date) => void;
}


function getDaysInMonth(year: number, month: number) {
  return new Date(year, month + 1, 0).getDate();
}

function getFirstDayOfMonth(year: number, month: number) {
  return new Date(year, month, 1).getDay();
}

function pad(n: number) {
  return String(n).padStart(2, "0");
}

function formatDisplay(date: Date): string {
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

function isSameDay(a: Date, b: Date) {
  return (
    a.getFullYear() === b.getFullYear() &&
    a.getMonth() === b.getMonth() &&
    a.getDate() === b.getDate()
  );
}

export default function DateTimePicker({ value, onChange }: DateTimePickerProps) {
  const { t } = useI18n();
  const DAYS = t.datePicker.days;
  const MONTHS = t.datePicker.months;
  const [open, setOpen] = useState(false);
  const [viewYear, setViewYear] = useState(value.getFullYear());
  const [viewMonth, setViewMonth] = useState(value.getMonth());
  const [hours, setHours] = useState(pad(value.getHours()));
  const [minutes, setMinutes] = useState(pad(value.getMinutes()));
  const containerRef = useRef<HTMLDivElement>(null);

  // Sync view when value changes externally
  useEffect(() => {
    setViewYear(value.getFullYear());
    setViewMonth(value.getMonth());
    setHours(pad(value.getHours()));
    setMinutes(pad(value.getMinutes()));
  }, [value]);

  // Close on outside click
  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [open]);

  const prevMonth = () => {
    if (viewMonth === 0) {
      setViewMonth(11);
      setViewYear((y) => y - 1);
    } else {
      setViewMonth((m) => m - 1);
    }
  };

  const nextMonth = () => {
    if (viewMonth === 11) {
      setViewMonth(0);
      setViewYear((y) => y + 1);
    } else {
      setViewMonth((m) => m + 1);
    }
  };

  const selectDay = useCallback(
    (day: number) => {
      const h = parseInt(hours, 10) || 0;
      const m = parseInt(minutes, 10) || 0;
      const newDate = new Date(viewYear, viewMonth, day, h, m);
      onChange(newDate);
    },
    [viewYear, viewMonth, hours, minutes, onChange]
  );

  const applyTime = useCallback(() => {
    const h = Math.min(23, Math.max(0, parseInt(hours, 10) || 0));
    const m = Math.min(59, Math.max(0, parseInt(minutes, 10) || 0));
    const newDate = new Date(value);
    newDate.setHours(h, m, 0, 0);
    setHours(pad(h));
    setMinutes(pad(m));
    onChange(newDate);
  }, [value, hours, minutes, onChange]);

  const goToToday = () => {
    const now = new Date();
    setViewYear(now.getFullYear());
    setViewMonth(now.getMonth());
  };

  // Build calendar grid
  const daysInMonth = getDaysInMonth(viewYear, viewMonth);
  const firstDay = getFirstDayOfMonth(viewYear, viewMonth);
  const today = new Date();

  const calendarCells: (number | null)[] = [];
  for (let i = 0; i < firstDay; i++) calendarCells.push(null);
  for (let d = 1; d <= daysInMonth; d++) calendarCells.push(d);

  return (
    <div ref={containerRef} style={{ position: "relative", display: "inline-block" }}>
      {/* Trigger button */}
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="date-input"
        style={{
          display: "flex",
          alignItems: "center",
          gap: 8,
          cursor: "pointer",
          minWidth: 190,
          whiteSpace: "nowrap",
        }}
      >
        <Calendar size={14} color="var(--text-muted)" />
        <span style={{ fontFamily: "var(--font-mono), monospace", fontSize: 13 }}>
          {formatDisplay(value)}
        </span>
      </button>

      {/* Dropdown panel */}
      {open && (
        <div
          style={{
            position: "absolute",
            top: "calc(100% + 6px)",
            left: 0,
            zIndex: 100,
            background: "var(--bg-card)",
            border: "1px solid var(--border-subtle)",
            borderRadius: 12,
            boxShadow: "0 8px 30px rgba(0,0,0,0.12), 0 2px 8px rgba(0,0,0,0.06)",
            width: 296,
            padding: 0,
            overflow: "hidden",
            animation: "fadeInUp 0.15s ease forwards",
          }}
        >
          {/* Header: month navigation */}
          <div
            style={{
              display: "flex",
              alignItems: "center",
              justifyContent: "space-between",
              padding: "14px 16px 10px",
              borderBottom: "1px solid var(--border-subtle)",
            }}
          >
            <button type="button" onClick={prevMonth} style={navBtnStyle}>
              <ChevronLeft size={16} />
            </button>
            <button
              type="button"
              onClick={goToToday}
              style={{
                fontSize: 14,
                fontWeight: 700,
                color: "var(--text-primary)",
                background: "none",
                border: "none",
                cursor: "pointer",
                padding: "2px 8px",
                borderRadius: 6,
              }}
            >
              {t.datePicker.monthYearTemplate
                .replace("{year}", String(viewYear))
                .replace("{month}", MONTHS[viewMonth])}
            </button>
            <button type="button" onClick={nextMonth} style={navBtnStyle}>
              <ChevronRight size={16} />
            </button>
          </div>

          {/* Day-of-week header */}
          <div
            style={{
              display: "grid",
              gridTemplateColumns: "repeat(7, 1fr)",
              padding: "8px 12px 4px",
            }}
          >
            {DAYS.map((d) => (
              <div
                key={d}
                style={{
                  textAlign: "center",
                  fontSize: 11,
                  fontWeight: 600,
                  color: "var(--text-muted)",
                  padding: "4px 0",
                }}
              >
                {d}
              </div>
            ))}
          </div>

          {/* Date grid */}
          <div
            style={{
              display: "grid",
              gridTemplateColumns: "repeat(7, 1fr)",
              padding: "0 12px 8px",
              gap: 2,
            }}
          >
            {calendarCells.map((day, i) => {
              if (day === null) {
                return <div key={`empty-${i}`} />;
              }
              const cellDate = new Date(viewYear, viewMonth, day);
              const isSelected = isSameDay(cellDate, value);
              const isToday = isSameDay(cellDate, today);

              return (
                <button
                  key={day}
                  type="button"
                  onClick={() => selectDay(day)}
                  style={{
                    width: 36,
                    height: 36,
                    margin: "0 auto",
                    display: "flex",
                    alignItems: "center",
                    justifyContent: "center",
                    borderRadius: 8,
                    border: isToday && !isSelected ? "1px solid var(--accent-blue)" : "1px solid transparent",
                    background: isSelected ? "var(--accent-blue)" : "transparent",
                    color: isSelected ? "white" : isToday ? "var(--accent-blue)" : "var(--text-primary)",
                    fontSize: 13,
                    fontWeight: isSelected || isToday ? 600 : 400,
                    cursor: "pointer",
                    transition: "all 0.1s ease",
                  }}
                  className={`calendar-day${isSelected ? " calendar-day-selected" : ""}`}
                >
                  {day}
                </button>
              );
            })}
          </div>

          {/* Time picker */}
          <div
            style={{
              display: "flex",
              alignItems: "center",
              gap: 8,
              padding: "10px 16px 14px",
              borderTop: "1px solid var(--border-subtle)",
              background: "var(--bg-primary)",
            }}
          >
            <Clock size={14} color="var(--text-muted)" />
            <input
              type="text"
              value={hours}
              onChange={(e) => setHours(e.target.value.replace(/\D/g, "").slice(0, 2))}
              onBlur={applyTime}
              style={timeInputStyle}
              maxLength={2}
              placeholder="HH"
            />
            <span style={{ fontSize: 14, fontWeight: 600, color: "var(--text-muted)" }}>:</span>
            <input
              type="text"
              value={minutes}
              onChange={(e) => setMinutes(e.target.value.replace(/\D/g, "").slice(0, 2))}
              onBlur={applyTime}
              onKeyDown={(e) => { if (e.key === "Enter") applyTime(); }}
              style={timeInputStyle}
              maxLength={2}
              placeholder="MM"
            />
            <button
              type="button"
              onClick={() => {
                const now = new Date();
                const newDate = new Date(value);
                newDate.setHours(now.getHours(), now.getMinutes(), 0, 0);
                setHours(pad(now.getHours()));
                setMinutes(pad(now.getMinutes()));
                onChange(newDate);
              }}
              style={{
                marginLeft: "auto",
                padding: "4px 10px",
                borderRadius: 6,
                border: "1px solid var(--border-subtle)",
                background: "var(--bg-secondary)",
                color: "var(--text-secondary)",
                fontSize: 11,
                fontWeight: 500,
                cursor: "pointer",
              }}
            >
              {t.datePicker.now}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

const navBtnStyle: React.CSSProperties = {
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
  width: 30,
  height: 30,
  borderRadius: 8,
  border: "1px solid var(--border-subtle)",
  background: "transparent",
  cursor: "pointer",
  color: "var(--text-secondary)",
  transition: "all 0.1s ease",
};

const timeInputStyle: React.CSSProperties = {
  width: 40,
  textAlign: "center",
  padding: "4px 0",
  borderRadius: 6,
  border: "1px solid var(--border-subtle)",
  background: "var(--bg-secondary)",
  color: "var(--text-primary)",
  fontSize: 14,
  fontWeight: 600,
  fontFamily: "var(--font-mono), monospace",
  outline: "none",
};
