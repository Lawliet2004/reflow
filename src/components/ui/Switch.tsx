import React from "react";
import { cn } from "../../utils/cn";

export interface SwitchProps extends Omit<
  React.ButtonHTMLAttributes<HTMLButtonElement>,
  "onChange" | "children"
> {
  on: boolean;
  onChange: (v: boolean) => void;
  ariaLabel?: string;
}

export const Switch = React.forwardRef<HTMLButtonElement, SwitchProps>(
  ({ on, onChange, ariaLabel, className, disabled, ...props }, ref) => {
    return (
      <button
        ref={ref}
        type="button"
        role="switch"
        aria-checked={on}
        aria-label={ariaLabel}
        disabled={disabled}
        onClick={() => onChange(!on)}
        className={cn(
          "relative inline-flex h-[22px] w-[40px] shrink-0 cursor-pointer items-center rounded-full transition-colors",
          "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent focus-visible:ring-offset-2",
          "disabled:cursor-not-allowed disabled:opacity-50",
          "before:absolute before:-inset-3 before:content-['']",
          on ? "bg-accent" : "bg-line-strong",
          className,
        )}
        {...props}
      >
        <span
          aria-hidden="true"
          className={cn(
            "pointer-events-none absolute top-[2px] h-[18px] w-[18px] rounded-full bg-white shadow-xs transition-all",
            on ? "left-[20px]" : "left-[2px]",
          )}
        />
      </button>
    );
  },
);

Switch.displayName = "Switch";
