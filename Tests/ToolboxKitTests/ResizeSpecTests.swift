import CoreGraphics
import Testing
@testable import ToolboxKit

@Suite("ResizeSpec")
struct ResizeSpecTests {
    let landscape = CGSize(width: 4000, height: 3000)
    let portrait = CGSize(width: 3000, height: 4000)

    @Test("fit preserves aspect ratio")
    func fitPreservesRatio() {
        let target = ResizeSpec.fit(width: 1000, height: 1000).target(for: landscape)
        #expect(target == CGSize(width: 1000, height: 750))
    }

    @Test("fit with only a width constrains the other side")
    func fitWidthOnly() {
        let target = ResizeSpec.fit(width: 2000, height: nil).target(for: landscape)
        #expect(target == CGSize(width: 2000, height: 1500))
    }

    @Test("fit does not upscale by default")
    func fitNoUpscale() {
        #expect(ResizeSpec.fit(width: 8000, height: 8000).target(for: landscape) == nil)
    }

    @Test("fit upscales when explicitly allowed")
    func fitUpscale() {
        let target = ResizeSpec.fit(width: 8000, height: 8000)
            .target(for: landscape, allowUpscale: true)
        #expect(target == CGSize(width: 8000, height: 6000))
    }

    @Test("longest side handles both orientations")
    func longestSide() {
        #expect(ResizeSpec.longestSide(2000).target(for: landscape)
                == CGSize(width: 2000, height: 1500))
        #expect(ResizeSpec.longestSide(2000).target(for: portrait)
                == CGSize(width: 1500, height: 2000))
    }

    @Test("percent scales both dimensions")
    func percent() {
        #expect(ResizeSpec.percent(50).target(for: landscape)
                == CGSize(width: 2000, height: 1500))
    }

    @Test("100 percent is a no-op")
    func percentIdentity() {
        #expect(ResizeSpec.percent(100).target(for: landscape) == nil)
    }

    @Test("exact ignores aspect ratio")
    func exact() {
        #expect(ResizeSpec.exact(width: 500, height: 500).target(for: landscape)
                == CGSize(width: 500, height: 500))
    }

    @Test("never collapses to zero pixels")
    func neverZero() {
        let target = ResizeSpec.percent(0.001).target(for: landscape)
        #expect(target == CGSize(width: 1, height: 1))
    }

    @Test("rejects degenerate input sizes")
    func degenerateInput() {
        #expect(ResizeSpec.percent(50).target(for: .zero) == nil)
    }

    @Test("isActive reflects usable settings")
    func isActive() {
        #expect(ResizeSpec.none.isActive == false)
        #expect(ResizeSpec.fit(width: nil, height: nil).isActive == false)
        #expect(ResizeSpec.exact(width: 0, height: 100).isActive == false)
        #expect(ResizeSpec.percent(100).isActive == false)
        #expect(ResizeSpec.longestSide(1024).isActive == true)
    }
}
