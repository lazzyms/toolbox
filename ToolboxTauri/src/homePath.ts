import { useEffect, useState } from "react";
import { homeDir } from "@tauri-apps/api/path";

export const shortenHomePath = (path: string, home: string | null) => {
  if (!home) return path;
  const base = home.replace(/[\\/]+$/, "");
  if (path === base) return "~";
  const separator = path.charAt(base.length);
  return path.startsWith(base) && (separator === "/" || separator === "\\")
    ? `~${path.slice(base.length)}`
    : path;
};

export const useHomeDir = () => {
  const [home, setHome] = useState<string | null>(null);
  useEffect(() => {
    let current = true;
    homeDir().then((dir) => { if (current) setHome(dir); }, () => undefined);
    return () => { current = false; };
  }, []);
  return home;
};
