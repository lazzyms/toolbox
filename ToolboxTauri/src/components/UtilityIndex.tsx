import { UtilityRegistry } from "../registry";
import { Button } from "@/components/ui/button";
import { useToolAvailability } from "../ToolAvailabilityContext";

export const UtilityIndex = () => {
  const availability = useToolAvailability();
  return (
    <nav className="utility-index" aria-label="Utilities" aria-hidden="true">
    {availability.filter(UtilityRegistry).map((tool) => (
      <Button
        variant="ghost"
        size="sm"
        key={tool.id}
        type="button"
        tabIndex={-1}
        aria-label={`${tool.title}: ${tool.blurb}`}
        onClick={() =>
          window.dispatchEvent(
            new CustomEvent("toolbox:open", { detail: tool.id }),
          )
        }
      >
        {tool.shortTitle}
      </Button>
    ))}
    </nav>
  );
};
