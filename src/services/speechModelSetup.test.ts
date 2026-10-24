import { beforeEach, expect, it, vi } from "vitest";
import { api } from "./tauriApi";
import { prepareSpeechRuntime } from "./speechSetup";
import { setupSpeechModel } from "./speechModelSetup";
import { DEFAULT_ASR_SETTINGS, type AppSettings } from "../types";
const DEFAULT_SETTINGS = { asr: DEFAULT_ASR_SETTINGS, offline_mode: false } as AppSettings;
vi.mock("./tauriApi", () => ({
  api: {
    getSettings: vi.fn(),
    installModel: vi.fn(),
    getModelStatus: vi.fn(),
    reloadModel: vi.fn(),
  },
}));
vi.mock("./speechSetup", () => ({ prepareSpeechRuntime: vi.fn() }));
beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(api.getSettings).mockResolvedValue({
    ...DEFAULT_SETTINGS,
    offline_mode: false,
    asr: { ...DEFAULT_SETTINGS.asr, model: "zipformer-20m", runtime: "python" },
  });
  vi.mocked(api.getModelStatus).mockResolvedValue({ installed: false } as Awaited<
    ReturnType<typeof api.getModelStatus>
  >);
  vi.mocked(api.installModel).mockResolvedValue(undefined);
  vi.mocked(prepareSpeechRuntime).mockResolvedValue(undefined);
});
it("waits for runtime verification and deduplicates setup before downloading", async () => {
  let ready!: () => void;
  vi.mocked(prepareSpeechRuntime).mockImplementation(
    () =>
      new Promise<void>((resolve) => {
        ready = resolve;
      }),
  );
  const first = setupSpeechModel("zipformer-20m");
  expect(setupSpeechModel("zipformer-20m")).toBe(first);
  await vi.waitFor(() => expect(prepareSpeechRuntime).toHaveBeenCalledOnce());
  expect(api.installModel).not.toHaveBeenCalled();
  ready();
  await first;
  expect(api.installModel).toHaveBeenCalledExactlyOnceWith("zipformer-20m");
});
it("never downloads weights if runtime verification fails", async () => {
  vi.mocked(prepareSpeechRuntime).mockRejectedValue(new Error("verification failed"));
  await expect(setupSpeechModel("zipformer-20m")).rejects.toThrow("verification failed");
  expect(api.installModel).not.toHaveBeenCalled();
});
it("does not download after settings change while the runtime is prepared", async () => {
  vi.mocked(api.getSettings)
    .mockResolvedValueOnce({
      ...DEFAULT_SETTINGS,
      offline_mode: false,
      asr: { ...DEFAULT_SETTINGS.asr, model: "zipformer-20m", runtime: "python" },
    })
    .mockResolvedValueOnce({
      ...DEFAULT_SETTINGS,
      asr: { ...DEFAULT_SETTINGS.asr, model: "other" },
    });
  await expect(setupSpeechModel("zipformer-20m")).rejects.toThrow("settings changed");
  expect(api.installModel).not.toHaveBeenCalled();
});

it("prepares and reloads installed models without downloading weights", async () => {
  vi.mocked(api.getModelStatus).mockResolvedValue({ installed: true } as Awaited<
    ReturnType<typeof api.getModelStatus>
  >);
  await setupSpeechModel("zipformer-20m");
  expect(prepareSpeechRuntime).toHaveBeenCalledOnce();
  expect(api.reloadModel).toHaveBeenCalledOnce();
  expect(api.installModel).not.toHaveBeenCalled();
});
