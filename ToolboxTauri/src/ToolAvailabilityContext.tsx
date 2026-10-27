import { createContext, useContext, useMemo, type ReactNode } from "react";
import type { AtomicToolId } from "./contracts";
import type { ToolAvailabilitySnapshot } from "./toolFlags";

export type ToolAvailability = Readonly<{
  allows(id: AtomicToolId): boolean;
  filter<T extends { id: AtomicToolId }>(items: readonly T[]): readonly T[];
}>;

const ToolAvailabilityContext = createContext<ToolAvailability | null>(null);

export function ToolAvailabilityProvider({
  snapshot,
  children,
}: {
  snapshot: ToolAvailabilitySnapshot;
  children: ReactNode;
}) {
  const availability = useMemo<ToolAvailability>(() => {
    const allows = (id: AtomicToolId) => snapshot.enabledByTool[id];
    return Object.freeze({
      allows,
      filter: <T extends { id: AtomicToolId }>(items: readonly T[]) =>
        items.filter((item) => allows(item.id)),
    });
  }, [snapshot]);
  return (
    <ToolAvailabilityContext.Provider value={availability}>
      {children}
    </ToolAvailabilityContext.Provider>
  );
}

export function useToolAvailability(): ToolAvailability {
  const availability = useContext(ToolAvailabilityContext);
  if (!availability) throw new Error("Tool availability is not initialized");
  return availability;
}
