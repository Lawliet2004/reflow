import React from "react";
import { cn } from "../../utils/cn";
import { Loader2 } from "lucide-react";

export type ButtonVariant = "primary" | "secondary" | "outline" | "ghost" | "danger";
export type ButtonSize = "sm" | "md" | "lg";

export interface ButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  size?: ButtonSize;
  loading?: boolean;
}

const variantStyles: Record<ButtonVariant, string> = {
  primary:
    "bg-accent hover:bg-accent-hover active:bg-accent-hover text-white shadow-sm border-accent-border focus-visible:ring-accent",
  secondary:
    "bg-surface hover:bg-base-2 dark:bg-surface dark:hover:bg-surface-3 text-ink-2 dark:text-ink-2 border-line dark:border-line focus-visible:ring-accent",
  outline:
    "bg-transparent hover:bg-base-2 dark:hover:bg-surface-3 text-ink-2 dark:text-ink-2 border-line-strong dark:border-line-strong focus-visible:ring-accent",
  ghost:
    "bg-transparent hover:bg-base-2 dark:hover:bg-surface-3 text-muted dark:text-muted border-transparent focus-visible:ring-accent",
  danger:
    "bg-danger hover:opacity-90 active:opacity-100 text-white shadow-sm border-transparent focus-visible:ring-danger",
};

const sizeStyles: Record<ButtonSize, string> = {
  sm: "px-2.5 py-1 text-xs gap-1.5 rounded-lg",
  md: "px-3.5 py-1.5 text-sm gap-2 rounded-lg",
  lg: "px-4 py-2 text-base gap-2.5 rounded-xl",
};

export const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(
  (
    {
      className,
      variant = "secondary",
      size = "md",
      loading = false,
      disabled,
      children,
      ...props
    },
    ref,
  ) => {
    return (
      <button
        ref={ref}
        disabled={disabled || loading}
        className={cn(
          "inline-flex items-center justify-center font-medium border transition-colors select-none focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-offset-2 disabled:opacity-50 disabled:pointer-events-none active:scale-[0.98]",
          variantStyles[variant],
          sizeStyles[size],
          className,
        )}
        {...props}
      >
        {loading && <Loader2 className="w-4 h-4 animate-spin shrink-0" />}
        {children}
      </button>
    );
  },
);

Button.displayName = "Button";
