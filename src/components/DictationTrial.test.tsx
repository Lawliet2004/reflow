import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import * as bridge from "../services/tauriApi";
import { DictationTrial } from "./DictationTrial";

let listeners: Map<string, (payload: any) => void>;
const delivery = {
  session_id: 7,
  pasted: true,
  fallback_copy: false,
  paste_chord: "Ctrl+V",
  process_name: "Reflow",
  message: "Done",
};
beforeEach(() => {
  listeners = new Map();
  vi.spyOn(bridge, "safeListen").mockImplementation(async (event, handler) => {
    listeners.set(event, handler);
    return () => {};
  });
});
const emit = (event: string, payload: unknown) => act(() => listeners.get(event)?.(payload));
async function show() {
  const success = vi.fn();
  const reset = vi.fn();
  const view = render(<DictationTrial onSuccess={success} onReset={reset} />);
  await waitFor(() =>
    expect(screen.getByRole("button", { name: "Record a sentence" })).toBeEnabled(),
  );
  return {
    ...view,
    success,
    reset,
    box: screen.getByRole("textbox", { name: "Try dictation here" }),
  };
}
const paste = (box: HTMLElement, text: string) =>
  fireEvent.paste(box, { clipboardData: { getData: () => text } });

it("does not accept typing, unrelated sessions, or a final transcript alone as insertion proof", async () => {
  const { success, box } = await show();
  fireEvent.change(box, { target: { value: "Manually typed" } });
  emit("app:state-changed", "RECORDING");
  fireEvent.change(box, { target: { value: "Hello world" } });
  emit("transcript:source", { session_id: 7, dictation: true, final_text: "Hello world" });
  emit("injection:result", { ...delivery, session_id: 6 });
  paste(box, "Hello world");
  expect(success).not.toHaveBeenCalled();
  emit("injection:result", delivery);
  expect(success).toHaveBeenCalledWith("inserted");
});

it("does not complete setup when a pending copy returns after unmount", async () => {
  let finish!: () => void;
  vi.spyOn(bridge.api, "copyText").mockImplementation(
    () =>
      new Promise<void>((resolve) => {
        finish = resolve;
      }),
  );
  const { success, unmount } = await show();
  emit("app:state-changed", "RECORDING");
  emit("transcript:source", { session_id: 7, dictation: true, final_text: "Hello world" });
  emit("injection:result", { ...delivery, pasted: false, fallback_copy: true });
  emit("app:state-changed", "READY");
  fireEvent.click(screen.getByRole("button", { name: "Copy transcript and use clipboard mode" }));
  unmount();
  await act(async () => finish());
  expect(success).not.toHaveBeenCalled();
});

it.each(["before", "after"])(
  "correlates a native paste arriving %s the result events",
  async (order) => {
    const { success, box, reset } = await show();
    emit("app:state-changed", "RECORDING");
    expect(reset).toHaveBeenCalledOnce();
    if (order === "before") paste(box, "Hello world");
    emit("injection:result", delivery);
    emit("transcript:source", { session_id: 7, dictation: true, final_text: "Hello world" });
    if (order === "after") paste(box, "Hello world");
    expect(success).toHaveBeenCalledWith("inserted");
  },
);

it("does not accept assistant or meeting results", async () => {
  const { success, box } = await show();
  emit("app:state-changed", "RECORDING");
  paste(box, "Hello world");
  emit("injection:result", delivery);
  emit("transcript:source", { session_id: 7, dictation: false, final_text: "Hello world" });
  expect(success).not.toHaveBeenCalled();
});

it("requires an explicit copy choice for a failed automatic insertion", async () => {
  const copy = vi.spyOn(bridge.api, "copyText").mockResolvedValue();
  const { success } = await show();
  emit("app:state-changed", "RECORDING");
  emit("injection:result", { ...delivery, pasted: false, fallback_copy: true });
  emit("transcript:source", { session_id: 7, dictation: true, final_text: "Hello world" });
  expect(success).not.toHaveBeenCalled();
  emit("app:state-changed", "READY");
  fireEvent.click(screen.getByRole("button", { name: "Copy transcript and use clipboard mode" }));
  await waitFor(() => expect(success).toHaveBeenCalledWith("clipboard"));
  expect(copy).toHaveBeenCalledWith("Hello world");
});
