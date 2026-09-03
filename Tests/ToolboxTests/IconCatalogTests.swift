import XCTest
@testable import Toolbox

final class IconCatalogTests: XCTestCase {
    func testRegistryKeepsFeaturePresentationText() {
        XCTAssertEqual(Utility.all.first?.title, "Remove PDF Password")
        XCTAssertTrue(Utility.all.allSatisfy { !$0.title.isEmpty && !$0.blurb.isEmpty })

        let storedProperties = Mirror(reflecting: Utility.all[0]).children.compactMap(\.label)
        XCTAssertFalse(storedProperties.contains("symbol"))
        XCTAssertFalse(storedProperties.contains("tint"))
    }
}
