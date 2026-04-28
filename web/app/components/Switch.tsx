"use client";

import type { KeyboardEvent } from "react";

interface BaseProps {
  checked: boolean;
  onChange: (next: boolean) => void;
  disabled?: boolean;
  /** Optional id — handy when pairing the switch with a `<label htmlFor>`. */
  id?: string;
  className?: string;
}

type LabelledProps =
  | (BaseProps & { "aria-label": string; "aria-labelledby"?: never })
  | (BaseProps & { "aria-labelledby": string; "aria-label"?: never });

/**
 * Material Design 3 toggle switch.
 *
 * Renders a `<button role="switch">` styled by the shared `.switch` CSS
 * class in globals.css. The underlying CSS handles sizing, the
 * off→on thumb morph, focus ring, and disabled/pressed states, so
 * callers only provide state + a11y metadata.
 *
 * Always supply either `aria-label` or `aria-labelledby` — screen
 * readers need to know what the toggle controls.
 */
export function Switch(props: LabelledProps) {
  const { checked, onChange, disabled, id, className } = props;

  const handleKeyDown = (event: KeyboardEvent<HTMLButtonElement>) => {
    // M3 / ARIA APG: Space and Enter toggle the switch. The browser handles
    // click behaviour via the default Enter/Space→click mapping for <button>,
    // but some assistive tech expects an explicit response.
    if (event.key === " " || event.key === "Enter") {
      event.preventDefault();
      if (!disabled) onChange(!checked);
    }
  };

  return (
    <button
      id={id}
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={"aria-label" in props ? props["aria-label"] : undefined}
      aria-labelledby={
        "aria-labelledby" in props ? props["aria-labelledby"] : undefined
      }
      disabled={disabled}
      onClick={() => !disabled && onChange(!checked)}
      onKeyDown={handleKeyDown}
      className={className ? `switch ${className}` : "switch"}
    />
  );
}
