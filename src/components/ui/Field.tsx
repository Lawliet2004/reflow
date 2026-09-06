import React, { useId } from "react";
import { cn } from "../../utils/cn";

export interface FieldProps {
  label: string;
  hint?: string;
  error?: string;
  className?: string;
  children: React.ReactNode;
}

export const Field: React.FC<FieldProps> = ({ label, hint, error, className, children }) => {
  const generatedId = useId();
  const hintId = `${generatedId}-hint`;
  const errorId = `${generatedId}-error`;

  return (
    <div className={cn("space-y-1.5", className)}>
      <div className="flex items-center justify-between">
        <label
          htmlFor={generatedId}
          className="block text-xs font-medium text-slate-700 dark:text-slate-300"
        >
          {label}
        </label>
        {hint && !error && (
          <span id={hintId} className="text-[11px] text-slate-400 dark:text-slate-500">
            {hint}
          </span>
        )}
      </div>

      <div>{children}</div>

      {error && (
        <p id={errorId} className="text-xs text-rose-600 dark:text-rose-400 font-medium">
          {error}
        </p>
      )}
    </div>
  );
};
