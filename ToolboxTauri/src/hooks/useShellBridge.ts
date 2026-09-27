import { useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export const SHELL_EVENT = "toolbox://shell-event";

export type ShellCommand =
  | "search"
  | "open"
  | "undo"
  | "redo"
  | "export"
  | "zoom-in"
  | "zoom-out"
  | "zoom-reset"
  | "toggle-theme"
  | "shortcuts";

export type ShellWorkspace = "pdf-editor" | "image-editor";

export type ShellEvent =
  | { kind: "files"; activationId: string; workspace: ShellWorkspace; paths: string[] }
  | { kind: "command"; command: ShellCommand }
  | { kind: "rejected-files"; paths: string[]; reason: string };

export const useShellBridge = (onEvent: (event: ShellEvent) => void) => {
  const onEventRef = useRef(onEvent);
  useEffect(() => { onEventRef.current = onEvent; }, [onEvent]);
  useEffect(() => {
    if (!("__TAURI_INTERNALS__" in window)) return;
    let active = true;
    let unlisten: (() => void) | undefined;
    void listen<ShellEvent>(SHELL_EVENT, ({ payload }) => onEventRef.current(payload)).then((dispose) => {
      if (!active) {
        dispose();
        return;
      }
      unlisten = dispose;
      void invoke("shell_ready");
    });
    return () => {
      active = false;
      unlisten?.();
    };
  }, []);
};
