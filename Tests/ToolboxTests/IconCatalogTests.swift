import XCTest
@testable import Toolbox

final class IconCatalogTests: XCTestCase {
    func testSidebarUsesTablerIconNames() {
        XCTAssertEqual(Utility.all.first?.symbol, "lock-open")
        XCTAssertTrue(Utility.all.allSatisfy { !$0.symbol.contains(".") })
    }
}
