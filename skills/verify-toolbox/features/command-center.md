# Command center

The command center lets you find a Toolbox action, narrow the library, reopen recent work, and reach a task workspace.

## Sub-features

- `tool-search` filters actions by their title and description.
- `search-shortcut` focuses the search field with `super+k` on macOS or `ctrl+k` on Windows and Linux.
- `category-filter` narrows the library to PDF, Images, or Documents.
- `favorite-filter` shows actions saved with their favorite button.
- `recent-filter` shows actions opened in the verification app.
- `quick-access` opens one of the visible sidebar actions.
- `workspace-open` opens an action or its containing workspace from a library card.

## How to get to it (user POV)

- Start at the home view after Toolbox launches.
- Choose `All tools`, `Favorites`, or `Recent` in Workspace navigation.
- Choose `PDF`, `Images`, or `Documents` under Tool categories.
- Type in `Search tools` or press `super+k` on macOS or `ctrl+k` on Windows and Linux.
- Choose a visible action in Quick access or choose `Open <tool title>` in Tool library.
- Choose the star control named `Add <tool title> to favorites` or `Remove <tool title> from favorites` on an action card.
- Choose `Open <workspace title>` to enter a workspace through its library card.

## Driving it with CUA desktop control

Preconditions:

- The native Toolbox Verification window passes Doctor.
- Settings and native file dialogs are closed.
- Record the starting Favorites and Recent state if you will change either one.

- **Search.** Focus `Search tools` and type `Compress Images`. The updated tree contains one matching Tool library action named `Open Compress Images` and its card heading `Compress Images`.
- **Keyboard search.** Return to the home view, press `super+k` on macOS or `ctrl+k` on Windows and Linux, then type `Compress Images`. The same matching action appears and focus remains in `Search tools` until you move it.
- **Open a search result.** Focus the search field, press `ArrowDown`, and press `Enter`. Toolbox opens the Image editor workspace and its live status reports that the workspace is open.
- **Category filter.** Return to `All tools`, choose `PDF` in Tool categories, and confirm that the library shows PDF actions such as `Open Edit PDF` without `Open Compress Images`. Repeat with `Images` and `Documents` when those filters are in scope.
- **Favorite filter.** Choose `Add Compress Images to favorites`, then choose `Favorites`. The action appears in the filtered library with `aria-pressed` on. Choose `Remove Compress Images from favorites` after verification and confirm that the action leaves Favorites.
- **Recent filter.** Open `Compress Images`, return to the home view, and choose `Recent`. The Image editor action appears. The open action updates the verification app's Recent list.
- **Quick access.** Choose one visible action in Quick access. Its workspace title appears and the live status reports that workspace is open.
- **Workspace card.** Choose `Open <workspace title>` on a Tool library card. Toolbox opens the first action in that workspace. Confirm the workspace title and its initial input state.

## Gotchas

- Search results update the Tool library while the Quick access row can remain visible. Check the library action, not only the sidebar.
- Opening a tool writes to the Recent list. The separate verification app keeps that history out of the installed app.
- Favorites persist in the verification profile. Restore the starting state after a favorites check.
- `ArrowDown` moves focus from the search field to the first result. It does not open the result until you press `Enter`.
