# Command center

The home screen lets you search tools, keep favorites, and open a workspace or a single tool.

## Sub-features

- `tool-search` filters the Tool library by one substring of a tool title.
- `favorites` saves tools with their favorite control and lists them under `Favorites`.
- `tool-library` opens a workspace or a tool from its library card.
- `workspace-rail` opens a workspace from the home rail.

## How to get to it (user POV)

- Start at the home view after Toolbox launches, or choose `← All tools` inside a workspace.
- Choose `All tools`, `Favorites`, or a workspace button in the rail.
- Type in `Search tools`. Its placeholder reads `Find a tool, like merge, split or compress`.
- Choose `Open <workspace>` or `Open <tool>` under `Tool library`.
- Choose `Add <tool> to favorites` or `Remove <tool> from favorites`. It sits beside the tool's toolbar item in its workspace.

## Driving it with CUA desktop control

Preconditions:

- The verification window passes Doctor.
- Settings and native file dialogs are closed.
- Record the starting favorites if you will change them.

- **Search.** Type `merge` in `Search tools`. The library shows `Open Merge PDF`. Clear the field afterward.
- **Search miss.** Type `extract pages`. The library shows `No matching tools` and `Browse all tools`. A single word such as `extract` shows `Open Extract PDF Pages`.
- **Open a tool.** Choose `Open Lock & Unlock`. The window title becomes `Toolbox — File Security` and the tool heading is `Lock & Unlock`.
- **Open a workspace.** Choose `Open PDF Editor`. The toolbar lists the PDF Editor tools.
- **Favorites.** In a workspace, choose `Add Lock & Unlock to favorites`. The control becomes `Remove Lock & Unlock from favorites`. Choose `← All tools`, then `Favorites`, and confirm the tool is listed. Remove the favorite afterward and confirm the control reads `Add Lock & Unlock to favorites`.

## Gotchas

- Search does not split words. Use one word per query.
- Inside a workspace, the rail has no workspace buttons. Return with `← All tools` first.
- Favorites persist in the verification profile. Restore the starting state.
