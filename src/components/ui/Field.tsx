import React, { useId } from "react";
import { cn } from "../../utils/cn";

export interface FieldProps {
  label: string;
  hint?: string;
  error?: string;
  className?: string;
  children: React.ReactNode;
}

function findControl(
  nodes: React.ReactNode,
): React.ReactElement<React.HTMLAttributes<HTMLElement>> | undefined {
  return React.Children.toArray(nodes).flatMap((child) => {
    if (!React.isValidElement<React.HTMLAttributes<HTMLElement>>(child)) return [];
    if (["input", "select", "textarea"].includes(String(child.type))) return [child];
    const nested = findControl(child.props.children);
    return nested ? [nested] : [];
  })[0];
}

export const Field: React.FC<FieldProps> = ({ label, hint, error, className, children }) => {
  const generatedId = useId();
  const hintId = `${generatedId}-hint`;
  const errorId = `${generatedId}-error`;
  const control = findControl(children);
  const controlId = control?.props.id ?? generatedId;
  const associateControl = (nodes: React.ReactNode): React.ReactNode =>
    React.Children.map(nodes, (child) => {
      if (!React.isValidElement<React.HTMLAttributes<HTMLElement>>(child)) return child;
      if (control && child.type === control.type && child.props === control.props) {
        const describedBy = [
          child.props["aria-describedby"],
          error ? errorId : hint ? hintId : null,
        ]
          .filter(Boolean)
          .join(" ");
        return React.cloneElement(child, {
          id: controlId,
          "aria-describedby": describedBy || undefined,
          "aria-invalid": error ? true : child.props["aria-invalid"],
        });
      }
      return child.props.children
        ? React.cloneElement(child, { children: associateControl(child.props.children) })
        : child;
    });
  const controls = associateControl(children);

  return (
    <div className={cn("space-y-1.5", className)}>
      <div className="flex items-center justify-between">
        <label htmlFor={controlId} className="block text-xs font-medium text-ink-2">
          {label}
        </label>
        {hint && !error && (
          <span id={hintId} className="text-2xs text-muted">
            {hint}
          </span>
        )}
      </div>

      <div>{controls}</div>

      {error && (
        <p id={errorId} className="text-xs text-danger font-medium">
          {error}
        </p>
      )}
    </div>
  );
};
