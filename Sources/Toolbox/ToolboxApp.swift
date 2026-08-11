import SwiftUI

@main
struct ToolboxApp: App {
    var body: some Scene {
        Window("Toolbox", id: "main") {
            ContentView()
                .frame(minWidth: 780, minHeight: 560)
        }
        .windowResizability(.contentMinSize)
        .commands {
            CommandGroup(replacing: .newItem) {}
        }
    }
}

struct ContentView: View {
    @State private var selection: Utility.ID? = Utility.all.first?.id

    private var selected: Utility? {
        Utility.all.first { $0.id == selection }
    }

    var body: some View {
        NavigationSplitView {
            List(selection: $selection) {
                ForEach(Utility.Category.allCases) { category in
                    Section(category.rawValue) {
                        ForEach(Utility.inCategory(category)) { utility in
                            HStack(spacing: 10) {
                                Image(systemName: utility.symbol)
                                    .foregroundStyle(utility.tint)
                                    .frame(width: 20)
                                Text(utility.title)
                                    .lineLimit(1)
                            }
                            .tag(utility.id)
                        }
                    }
                }
            }
            .navigationSplitViewColumnWidth(min: 210, ideal: 230, max: 280)
        } detail: {
            if let selected {
                // Identity keyed on the utility so switching tools resets each
                // pane's file queue and results instead of leaking state across.
                selected.makeView()
                    .id(selected.id)
            } else {
                ContentUnavailableView(
                    "Pick a utility",
                    systemImage: "square.grid.2x2",
                    description: Text("Choose a tool from the sidebar to get started.")
                )
            }
        }
    }
}
