import React from "react";
import { Info } from "lucide-react";

interface ModelNoticeProps {
  notice?: string | null;
}

/**
 * Explains why the running ASR model is not the one selected in Settings.
 *
 * The selector treats the Settings choice as a ceiling and steps down when the
 * larger model cannot fit free VRAM, because a smaller model on the GPU measured
 * 2887 ms against 17546 ms for a larger one pushed to the CPU. That substitution
 * must never be silent: an unexplained accuracy drop reads as a bug, and the
 * user has no way to connect it to how much VRAM the rest of their desktop
 * happens to be using.
 */
export const ModelNotice: React.FC<ModelNoticeProps> = ({ notice }) => {
  if (!notice || !notice.trim()) return null;

  return (
    <div className="model-notice" role="status">
      <Info className="w-4 h-4 model-notice-icon" aria-hidden="true" />
      <span>{notice}</span>
    </div>
  );
};
