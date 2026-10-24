import type { AudioDevice } from "./index";
export interface MeetingConfig {
  microphone: string | null;
  playback: string | null;
  limit_hours: number;
  summary_model: string | null;
}
export interface Meeting {
  id: string;
  created_at: string;
  config: MeetingConfig;
  capture: string;
  processing: string;
  transcription_status?: string;
  summary_status?: string;
  elapsed_ms: number;
  saved_frames: [number, number];
  last_commit_ms: number;
  levels: [number, number];
  gaps: { track: number; start_ms: number; end_ms: number; reason: string }[];
  warning: string | null;
  error: string | null;
  stop_reason: string | null;
  processed_frames: number;
  summary_progress: number;
  audio_removed: boolean;
}
export interface MeetingSources {
  supported: boolean;
  explanation: string;
  devices: AudioDevice[];
  playback: AudioDevice[];
}
export interface MeetingOutput {
  segments: { text: string; offset_ms: number; end_ms: number }[];
  summary: string | null;
  details: string | null;
}
