# Command center

The command center is Toolbox's home view. It lists every tool, groups them into Views, and opens a tool's workspace.

## Sub-features

- `tool-search` filters the Tool library by tool title and description.
- `search-shortcut` focuses `Search tools` with `super+k` on macOS or `ctrl+k` on Windows and Linux.
- `view-nav` opens one View from the sidebar: File Security, PDF Editor, PDF Conversion, Image Editor, Compress Files or Media.
- `favorite-filter` shows tools saved with their favorite checkbox.
- `library-open` opens a tool or its View from a Tool library card.
- `more-tools` expands a library card with `N more tool(s)`.

## How to get to it (user POV)

- Start at the home view after Toolbox launches. A workspace returns there with `← All tools`.
- In the sidebar choose `All tools`, `Favorites`, or a View under VIEWS. `Settings` sits at the bottom (`super+,`).
- Type in `Search tools`. Its placeholder reads "Find a tool, like merge, OCR or compress".
- In Tool library choose `Open <View>` for a View, `Open <tool title>` for a tool, or `N more tool(s)` to expand a card.
- Inside a workspace, the checkbox `Add <tool title> to favorites` saves the tool to Favorites.

## Driving it with CUA desktop control

Preconditions:

- `helpers/doctor.sh` prints `doctor: OK`.
- Settings and native file dialogs are closed.
- Record the starting Favorites list if you will change it.

- **Search.** Focus `Search tools` and type `Compress`. The Tool library narrows to matching `Open <tool title>` buttons. Re-read the tree to confirm.
- **Keyboard search.** From the home view press `super+k`. Focus lands in `Search tools`.
- **Open a View.** Choose `File Security` under VIEWS, or `Open File Security` in Tool library. The workspace heading `File Security` appears with the live status "File Security workspace open."
- **Open a tool.** Choose `Open Lock & Unlock`. The workspace toolbar `File Security tools` shows the `Lock & Unlock` checkbox selected.
- **Favorites.** In a workspace check `Add <tool title> to favorites`, choose `← All tools`, then `Favorites`. The tool appears. Uncheck it afterwards and confirm it leaves Favorites.
- **More tools.** Choose `N more tool(s)` on a card. The card lists the extra `Open <tool title>` buttons.

## Gotchas

- There is no Recent filter, Quick access row, or Tool categories list any more. Older notes that name them are stale.
- Each `Open <tool title>` button carries a help description. Lock & Unlock's still reads "Remove a password you know from PDF, Word, Excel and PowerPoint files." That copy is stale after the 2026-10-20 Unlock decision.
- Favorites persist in the verification profile. Restore the starting state.
- `type_text` into the search field can report fewer characters delivered than it sent while the text did land. Re-read the field value before trusting either report.
