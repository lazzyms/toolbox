import Foundation

struct AnalyticsEvent {
    let name: String
    let parameters: [String: Any]

    static func appOpened() -> Self { Self(name: "app_opened", parameters: [:]) }

}
