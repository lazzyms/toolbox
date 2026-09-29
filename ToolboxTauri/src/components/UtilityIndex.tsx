import { UtilityRegistry } from "../registry";
import { Button } from "@/components/ui/button";

export const UtilityIndex = () => (
  <nav className="utility-index" aria-label="Utilities" aria-hidden="true">
    {UtilityRegistry.map((tool) => (
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
