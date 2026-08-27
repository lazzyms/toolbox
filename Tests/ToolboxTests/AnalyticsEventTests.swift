import XCTest
@testable import Toolbox

final class AnalyticsEventTests: XCTestCase {
    func testInstallMeasurementUsesOnlyAppOpenedEvent() {
        let event = AnalyticsEvent.appOpened()

        XCTAssertEqual(event.name, "app_opened")
        XCTAssertTrue(event.parameters.isEmpty)
    }

    func testInstallMeasurementIsEnabledForNewInstalls() {
        XCTAssertTrue(AnalyticsManager.defaultEnabled)
    }
}
