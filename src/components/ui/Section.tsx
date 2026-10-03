import React from "react";
import { cn } from "../../utils/cn";

export interface SectionProps {
  icon?: React.ReactNode;
  title: string;
  description?: string;
  children: React.ReactNode;
  className?: string;
}

export const Section: React.FC<SectionProps> = ({
  icon,
  title,
  description,
  children,
  className,
}) => (
  <section className={cn("settings-section", className)}>
    <div className="mb-5 flex items-center gap-2.5">
      {icon && <div className="flex h-6 w-6 items-center justify-center text-muted">{icon}</div>}
      <h2 className="text-base font-semibold tracking-tight text-ink">{title}</h2>
    </div>
    {description && <p className="-mt-2 mb-4 text-sm leading-relaxed text-muted">{description}</p>}
    <div className="space-y-4">{children}</div>
  </section>
);

export interface RowProps {
  label: string;
  hint?: string;
  children: React.ReactNode;
  className?: string;
}

export const Row: React.FC<RowProps> = ({ label, hint, children, className }) => {
  const id = React.useId();
  const controls = React.Children.map(children, (child) => {
    if (
      !React.isValidElement<React.AriaAttributes>(child) ||
      !["input", "select", "textarea"].includes(String(child.type))
    )
      return child;
    return React.cloneElement(child, {
      "aria-labelledby": child.props["aria-label"] ? undefined : id,
      "aria-describedby": hint ? `${id}-hint` : undefined,
    });
  });
  return (
    <div className={cn("settings-row flex items-center justify-between gap-6 py-1", className)}>
      <div className="min-w-0">
        <p id={id} className="text-sm font-medium text-ink">
          {label}
        </p>
        {hint && (
          <p id={`${id}-hint`} className="mt-0.5 text-xs leading-relaxed text-muted">
            {hint}
          </p>
        )}
      </div>
      <div className="shrink-0">{controls}</div>
    </div>
  );
};
