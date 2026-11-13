import { useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export const SHELL_EVENT = "toolbox://shell-event";
export const TOOL_DROP_EVENT = "toolbox:tool-drop";

/** Where files were dropped, in CSS pixels from the top left of the window. */
export type DropPoint = { x: number; y: number };
/** What a tool receives when files are dropped on the window while it is open. */
export type ToolDrop = { paths: string[]; point: DropPoint | null };

/** The drop a TOOL_DROP_EVENT carries, or null when it carries something else. */
export const toolDrop = (event: Event): ToolDrop | null => {
  const detail = (event as CustomEvent<unknown>).detail as Partial<ToolDrop> | null;
  const paths = detail?.paths;
  if (!Array.isArray(paths) || !paths.every((path) => typeof path === "string")) return null;
  const point = detail?.point;
  return { paths, point: point && Number.isFinite(point.x) && Number.isFinite(point.y) ? point : null };
};

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
  | { kind: "dropped-files"; paths: string[]; point?: DropPoint }
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
