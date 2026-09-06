import React from "react";
import { IntelligenceTier, TierMetadata } from "../../types";
import { Badge } from "../ui/Badge";
import { Button } from "../ui/Button";
import { cn } from "../../utils/cn";
import { Check, Download, Trash2 } from "lucide-react";

export interface TierCardProps {
  metadata: TierMetadata;
  selected: boolean;
  recommended?: boolean;
  installed?: boolean;
  downloading?: boolean;
  downloadProgress?: number;
  onSelect: (tier: IntelligenceTier) => void;
  onDownload?: (tier: IntelligenceTier) => void;
  onDelete?: (tier: IntelligenceTier) => void;
  disabled?: boolean;
}

export function TierCard({
  metadata,
  selected,
  recommended,
  installed,
  downloading,
  downloadProgress = 0,
  onSelect,
  onDownload,
  onDelete,
  disabled,
}: TierCardProps) {
  const isVerbatim = metadata.id === "raw_verbatim";

  return (
    <div
      role="button"
      tabIndex={0}
      onClick={() => !disabled && onSelect(metadata.id)}
      onKeyDown={(e) => {
        if (e.key === " " || e.key === "Enter") {
          e.preventDefault();
          if (!disabled) onSelect(metadata.id);
        }
      }}
      className={cn(
        "relative flex flex-col p-4 rounded-xl border transition-all text-left cursor-pointer select-none focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-500",
        selected
          ? "bg-sky-50/50 dark:bg-sky-950/20 border-sky-500 dark:border-sky-500 shadow-sm ring-1 ring-sky-500"
          : "bg-white dark:bg-slate-900 border-slate-200 dark:border-slate-800 hover:border-slate-300 dark:hover:border-slate-700",
        disabled && "opacity-60 cursor-not-allowed",
      )}
    >
      <div className="flex items-start justify-between gap-2 mb-2">
        <div>
          <div className="flex items-center gap-2">
            <h3 className="font-semibold text-slate-900 dark:text-slate-100">{metadata.label}</h3>
            {selected ? (
              <Badge variant="primary" size="sm" className="gap-1">
                <Check className="w-3 h-3" /> Active
              </Badge>
            ) : null}
            {recommended ? (
              <Badge variant="success" size="sm">
                Recommended
              </Badge>
            ) : null}
          </div>
          <span className="text-xs text-slate-500 dark:text-slate-400">
            {metadata.tagline} ?{" "}
            {metadata.downloadSizeMB > 0 ? `${metadata.downloadSizeMB} MB` : "Zero download"}
          </span>
        </div>

        {!isVerbatim && onDownload && !installed && !downloading ? (
          <Button
            size="sm"
            variant="outline"
            onClick={(e) => {
              e.stopPropagation();
              onDownload(metadata.id);
            }}
            className="text-xs"
          >
            <Download className="w-3.5 h-3.5 mr-1" /> Get Model
          </Button>
        ) : null}

        {!isVerbatim && onDelete && installed && !selected ? (
          <Button
            size="sm"
            variant="ghost"
            onClick={(e) => {
              e.stopPropagation();
              onDelete(metadata.id);
            }}
            className="text-xs text-rose-600 hover:bg-rose-50 dark:hover:bg-rose-950/30"
          >
            <Trash2 className="w-3.5 h-3.5" />
          </Button>
        ) : null}
      </div>

      <p className="text-xs text-slate-600 dark:text-slate-400 leading-relaxed mb-3 flex-1">
        {metadata.description}
      </p>

      {downloading ? (
        <div className="space-y-1 mt-2">
          <div className="flex justify-between text-xs text-slate-500">
            <span>Downloading weights...</span>
            <span>{Math.round(downloadProgress * 100)}%</span>
          </div>
          <div className="w-full bg-slate-200 dark:bg-slate-700 h-1.5 rounded-full overflow-hidden">
            <div
              className="bg-sky-500 h-full transition-all duration-300 rounded-full"
              style={{ width: `${Math.max(5, Math.min(100, downloadProgress * 100))}%` }}
            />
          </div>
        </div>
      ) : null}
    </div>
  );
}
