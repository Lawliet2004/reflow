import React from "react";
import { cn } from "../../utils/cn";

export type BadgeVariant = "default" | "primary" | "success" | "warning" | "danger" | "outline";
export type BadgeSize = "sm" | "md";

export interface BadgeProps extends React.HTMLAttributes<HTMLSpanElement> {
  variant?: BadgeVariant;
  size?: BadgeSize;
}

const variantStyles: Record<BadgeVariant, string> = {
  default: "bg-surface-2 dark:bg-surface-3 text-ink-2 dark:text-ink-2 border-line",
  primary: "bg-accent-soft text-accent border-accent-border",
  success: "bg-success-soft text-success border-transparent",
  warning: "bg-accent-soft text-accent border-accent-border",
  danger: "bg-danger-soft text-danger border-transparent",
  outline: "bg-transparent text-muted border-line-strong",
};

const sizeStyles: Record<BadgeSize, string> = {
  sm: "px-1.5 py-0.5 text-2xs rounded-md font-medium",
  md: "px-2 py-0.5 text-xs rounded-lg font-medium",
};

export const Badge: React.FC<BadgeProps> = ({
  className,
  variant = "default",
  size = "md",
  children,
  ...props
}) => {
  return (
    <span
      className={cn(
        "inline-flex items-center justify-center border select-none transition-colors",
        variantStyles[variant],
        sizeStyles[size],
        className,
      )}
      {...props}
    >
      {children}
    </span>
  );
};
